//! トレイ起動音色エディタの`eframe::App`実装（Step 1、全サブステップ完了）。
//!
//! 「編集対象ch」セレクタ・op505-uiパネル（`draw_op505_panel`）・PRESETSサイドバー・
//! MASTER EFFECTSの4つを1画面にまとめる。値の変更はローカルの`dirty`フラグを立てるだけに
//! 留め、フレーム末尾でまとめて`SharedEditState`へpublishする（ドラッグ中に1サンプルごと
//! 書き込むと過負荷になるための1フレーム1回バッチ処理、gesture-app/op505-vstと同じ方針）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{mpsc, Arc};

use op505_core::Op505Patch;
use op505_editor::panel_source::build_panel_params;
use op505_editor::patch_source::{MasterEffectsState, PatchPanelSource};
use op505_editor::preset_panel::{draw_editor_top_bar, draw_presets_drawer, poll_presets_events, EditorPresetState, UndoUiState};
use op505_editor::undo::{EditorSnapshot, UndoApply, UndoStack};

use super::keyboard::{self, KeyboardState};
use super::preset_host::StandalonePresetHost;
use crate::midi_source::MidiSink;
use crate::shared::SharedEditState;

pub struct EditorApp {
    shared: Arc<SharedEditState>,
    /// 鍵盤（`keyboard.rs`）が試聴用のNote On/Offを積む先。実際のMIDI入力と同じキューを共有する。
    midi_sink: MidiSink,
    /// 編集中のパッチ本体。`Op505State`（旧`panel_params.rs`）が2フィールドだけを持つ薄いラッパー
    /// だったため、`op505-editor::patch_source::PatchPanelSource`への移行と同時にここへ
    /// インライン化した（Step 7）。
    patch: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    master: Rc<RefCell<MasterEffectsState>>,
    master_dirty: Rc<Cell<bool>>,
    undo: Rc<RefCell<UndoStack>>,
    presets: EditorPresetState,
    keyboard: KeyboardState,
    /// UIローカルの編集対象ch選択。`None`＝「(なし)」（既定、`SharedEditState::NO_EDIT_CHANNEL`と
    /// 対応）。ウィンドウを開いた直後に演奏中のチャンネルを勝手に上書きしないよう、既定は
    /// 必ず`None`にする（ユーザー確認済み、gesture-appコントローラー化ロードマップのplan参照）。
    edit_channel: Option<usize>,
    /// 未保存の確認ダイアログの結果待ち（`request_close_confirm`/`poll_close_confirm`参照）。
    pending_close_confirm: Option<mpsc::Receiver<bool>>,
    /// 確認ダイアログで「閉じる」が選ばれた後に送り直す`ViewportCommand::Close`を、
    /// 同じ未保存チェックで再度横取りしない（無限ループ防止）ためのフラグ。
    close_confirmed: bool,
}

impl EditorApp {
    /// `initial_patch`は`SharedEditState`が現在保持している値（起動直後は`default_patch`、
    /// 前回このエディタで編集した値が残っていればそれ）。エディタを開くたびにゼロから
    /// 組み立て直すのではなく、直前の状態を引き継ぐ。
    pub fn new(shared: Arc<SharedEditState>, midi_sink: MidiSink, initial_patch: Op505Patch) -> Self {
        Self {
            shared,
            midi_sink,
            patch: Rc::new(RefCell::new(initial_patch)),
            dirty: Rc::new(Cell::new(false)),
            master: Rc::new(RefCell::new(MasterEffectsState::default())),
            master_dirty: Rc::new(Cell::new(false)),
            undo: Rc::new(RefCell::new(UndoStack::new())),
            presets: EditorPresetState::new(),
            keyboard: KeyboardState::new(),
            edit_channel: None,
            pending_close_confirm: None,
            close_confirmed: false,
        }
    }

    /// 現在の状態をUndo用のスナップショットとして丸ごと切り出す。
    fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            patch: *self.patch.borrow(),
            master: *self.master.borrow(),
            patch_name: self.presets.patch_name().to_string(),
            bank: self.presets.bank(),
            program: self.presets.program(),
            has_selection: self.presets.has_selection(),
        }
    }

    /// `UndoStack::undo()`を呼び、結果があれば`apply_undo`で反映する。Ctrl+Zとボタンの両方から呼ぶ。
    fn try_undo(&mut self) {
        let apply = self.undo.borrow_mut().undo();
        if let Some(apply) = apply {
            self.apply_undo(apply);
        }
    }

    /// `UndoStack::redo()`を呼び、結果があれば`apply_undo`で反映する。Ctrl+Yとボタンの両方から呼ぶ。
    fn try_redo(&mut self) {
        let apply = self.undo.borrow_mut().redo();
        if let Some(apply) = apply {
            self.apply_undo(apply);
        }
    }

    /// Undo/Redoが返した内容を実際に反映する。`dirty`/`master_dirty`経由で既存の
    /// publish経路（`shared.publish_patch`→cpalコールバックの`apply_live_active`）に乗るため、
    /// 発音中の音へも即時反映される。
    fn apply_undo(&mut self, apply: UndoApply) {
        *self.patch.borrow_mut() = apply.snapshot.patch;
        self.dirty.set(true);
        *self.master.borrow_mut() = apply.snapshot.master;
        self.master_dirty.set(true);
        self.presets.restore_selection(
            apply.snapshot.patch_name,
            apply.snapshot.bank,
            apply.snapshot.program,
            apply.snapshot.has_selection,
        );
        if let Some(op) = &apply.bank_op {
            let host = StandalonePresetHost { patch: &self.patch, dirty: &self.dirty, shared: &self.shared };
            self.presets.apply_bank_op(op, &host);
        }
    }

    /// 未保存の変更がある状態でウィンドウを閉じようとしたときの確認ダイアログを別スレッドで
    /// 表示する。`rfd::MessageDialog::show()`はブロッキング呼び出しで、egui描画コールバック
    /// の内側で直接呼ぶとネストしたメッセージループがホストごとクラッシュしうる
    /// （`preset_panel.rs`のDelete確認ダイアログと同じ理由・同じ非同期パターン）。
    fn request_close_confirm(&mut self) {
        if self.pending_close_confirm.is_some() {
            return; // 確認中の二重リクエストを防ぐ
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let confirmed = matches!(
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Warning)
                    .set_title("Unsaved Changes")
                    .set_description("You have unsaved changes (+ New Voice / Delete not yet saved). Close anyway?")
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show(),
                rfd::MessageDialogResult::Yes
            );
            let _ = tx.send(confirmed);
        });
        self.pending_close_confirm = Some(rx);
    }

    /// 毎フレーム呼ぶ。確認ダイアログの結果が届いていれば反映する（未到着ならno-op）。
    fn poll_close_confirm(&mut self, ctx: &egui::Context) {
        let confirmed = match &self.pending_close_confirm {
            Some(rx) => match rx.try_recv() {
                Ok(confirmed) => confirmed,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => false,
            },
            None => return,
        };
        self.pending_close_confirm = None;
        if confirmed {
            // 一度キャンセルした`Close`を送り直す。次フレームでも同じチェックに引っかからない
            // よう`close_confirmed`を立てておく（無限ループ防止）。
            self.close_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let previous_edit_channel = self.edit_channel;

        // 未保存（Undo履歴が残っている＝Save/Save Asで基点をクリアしていない）のままウィンドウを
        // 閉じようとした場合、一旦キャンセルして確認ダイアログを出す。standaloneはプリセット
        // レジストリの寿命がウィンドウの寿命に紐づく（`EditorPresetState::new()`が開くたびに
        // ディスクから再構築する）ため、これが無いと遅延保存化（Step 8）により
        // 「+ New Voiceして閉じたら消える」事故になる。
        self.poll_close_confirm(ui.ctx());
        let close_requested = ui.ctx().input(|i| i.viewport().close_requested());
        if close_requested && !self.close_confirmed && self.undo.borrow().can_undo() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close_confirm();
        }

        self.undo.borrow_mut().begin_frame(self.snapshot());

        // テキスト欄（音色名・数値直接入力）にフォーカスがある間はeguiのTextEdit組み込みの
        // 文字単位Undoに委ね、パッチ全体のUndo/Redoは発火させない
        // （`preset_panel.rs`のDelete処理と同じ`!any_text_focused`ガード）。
        let any_text_focused = ui.memory(|m| m.focused().is_some());
        if !any_text_focused {
            let want_undo = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z));
            let want_redo = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Y));
            if want_undo {
                self.try_undo();
            } else if want_redo {
                self.try_redo();
            }
        }

        // + New Voice/Delete（BankChange）はハンドルのbegin_edit/end_edit経由で記録できない
        // （registry自体の変更でありパッチ全体のMementoでは表現できないため）。
        // パネル描画の前後でスナップショットを取り、bank_opが返ってきたときだけ
        // `push_bank_change`で明示的に1エントリ積む。
        let host = StandalonePresetHost { patch: &self.patch, dirty: &self.dirty, shared: &self.shared };
        let bank_change_before = self.snapshot();
        let undo_ui = UndoUiState { can_undo: self.undo.borrow().can_undo(), can_redo: self.undo.borrow().can_redo() };

        let mut presets_events = egui::Panel::top("editor_top_bar")
            .show_inside(ui, |ui| {
                // ハンバーガー・File（Open/Save/Save As）・Undo/Redoアイコンをまとめたメニューバー
                // （2026-09-03、PRESETSサイドバー常設→ハンバーガー開閉のオーバーレイ化に伴う移設）。
                let events = draw_editor_top_bar(ui, &mut self.presets, &host, undo_ui);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Edit Channel:");
                    let selected_text = match self.edit_channel {
                        None => "(None)".to_string(),
                        Some(ch) => format!("{}", ch + 1),
                    };
                    egui::ComboBox::from_id_salt("edit_channel_combo").selected_text(selected_text).show_ui(ui, |ui| {
                        if ui.selectable_label(self.edit_channel.is_none(), "(None)").clicked() {
                            self.edit_channel = None;
                        }
                        for ch in 0..16usize {
                            let label = format!("{}", ch + 1);
                            if ui.selectable_label(self.edit_channel == Some(ch), label).clicked() {
                                self.edit_channel = Some(ch);
                            }
                        }
                    });
                    if self.edit_channel.is_some() {
                        ui.colored_label(
                            egui::Color32::from_rgb(230, 160, 40),
                            "Program Change is ignored on this channel while editing",
                        );
                    }
                });
                events
            })
            .inner;

        egui::Panel::bottom("editor_keyboard").show_inside(ui, |ui| {
            keyboard::draw_keyboard(ui, &mut self.keyboard, &self.midi_sink, self.edit_channel);
        });

        let central_response = egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("op505_editor_scroll").show(ui, |ui| {
                let source = PatchPanelSource {
                    patch: self.patch.clone(),
                    patch_dirty: self.dirty.clone(),
                    master: self.master.clone(),
                    master_dirty: self.master_dirty.clone(),
                    undo: self.undo.clone(),
                };
                let panel = build_panel_params(&source);
                op505_ui::draw_op505_panel(ui, &panel);
            });
        });

        // ---- PRESETSドロワー（ハンバーガー開閉のオーバーレイ、CentralPanelの残り領域へ重ねる） ----
        presets_events.merge(draw_presets_drawer(ui.ctx(), &mut self.presets, &host, central_response.response.rect));
        presets_events.merge(poll_presets_events(&mut self.presets, &host));

        // 音色名欄はPresetSession内にあり、begin_edit/end_editを持つハンドル経由の記録が
        // できないため、フォーカス取得/喪失を1操作の区切りとして扱う
        // （`draw_presets_drawer_contents`のdoc参照）。
        if let Some(op) = presets_events.bank_op.clone() {
            self.undo.borrow_mut().push_bank_change(op, bank_change_before, self.snapshot());
        }
        if presets_events.history_cleared {
            // Open/Save/Save Asが完了した：ディスク上の実体と対応する新しい基点ができたので、
            // それ以前のUndo履歴（特にバンクファイルの内容を前提とする`BankOp`）は前提が
            // 崩れており巻き戻しの意味を持たない。
            self.undo.borrow_mut().clear();
        }
        if presets_events.patch_name_focus_gained {
            self.undo.borrow_mut().note_begin_edit();
        }
        if presets_events.patch_name_focus_lost {
            self.undo.borrow_mut().note_end_edit();
        }
        // リスト選択(apply_entry)はbegin_edit/end_editを経由しない単発のパッチ全体差し替え
        // なので、同フレームでbegin/endの両方を発行して1エントリとして記録する。
        if presets_events.list_selection_applied {
            let mut undo = self.undo.borrow_mut();
            undo.note_begin_edit();
            undo.note_end_edit();
        }
        if presets_events.undo_requested {
            self.try_undo();
        }
        if presets_events.redo_requested {
            self.try_redo();
        }

        self.undo.borrow_mut().end_frame(self.snapshot());

        if self.edit_channel != previous_edit_channel {
            self.shared.set_edit_channel(self.edit_channel);
        }

        if self.dirty.get() {
            self.dirty.set(false);
            self.shared.publish_patch(*self.patch.borrow());
        }

        if self.master_dirty.get() {
            self.master_dirty.set(false);
            // 編集対象は常にスロット0（`op505_midi::EffectControlTarget`のNRPN(0,1)
            // Channel Effect Routeが未設定のチャンネルが送る既定スロットと同じ。standaloneの
            // エディタは「編集対象ch」を選ばない限りマルチスロットを意識させない設計のため、
            // スロット選択UIは設けない）。値の並び順は`values_in_fx_order()`が`FxInt::ALL`の
            // 順で組み立て、`shared::FX_*`定数の並びと一致することをテストで凍結している
            // （`shared.rs`の`fx_int_order_matches_fx_constants`参照）。
            let values = self.master.borrow().values_in_fx_order();
            self.shared.publish_fx(0, values);
        }
    }
}
