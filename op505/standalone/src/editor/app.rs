//! トレイ起動音色エディタの`eframe::App`実装（Step 1、全サブステップ完了）。
//!
//! 「編集対象ch」セレクタ・op505-uiパネル（`draw_op505_panel`）・PRESETSサイドバー・
//! MASTER EFFECTSの4つを1画面にまとめる。値の変更はローカルの`dirty`フラグを立てるだけに
//! 留め、フレーム末尾でまとめて`SharedEditState`へpublishする（ドラッグ中に1サンプルごと
//! 書き込むと過負荷になるための1フレーム1回バッチ処理、gesture-app/op505-vstと同じ方針）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use op505_core::Op505Patch;
use op505_editor::layout::PRESETS_SIDEBAR_WIDTH;
use op505_editor::panel_source::build_panel_params;
use op505_editor::patch_source::{MasterEffectsState, PatchPanelSource};
use op505_editor::preset_panel::{draw_presets_panel, EditorPresetState};
use op505_editor::undo::{EditorSnapshot, UndoStack};

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
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let previous_edit_channel = self.edit_channel;
        self.undo.borrow_mut().begin_frame(self.snapshot());

        egui::Panel::top("editor_top_bar").show_inside(ui, |ui| {
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
        });

        egui::Panel::left("presets_panel").resizable(false).exact_size(PRESETS_SIDEBAR_WIDTH).show_inside(ui, |ui| {
            let host = StandalonePresetHost { patch: &self.patch, dirty: &self.dirty, shared: &self.shared };
            draw_presets_panel(ui, &mut self.presets, &host);
        });

        egui::Panel::bottom("editor_keyboard").show_inside(ui, |ui| {
            keyboard::draw_keyboard(ui, &mut self.keyboard, &self.midi_sink, self.edit_channel);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
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
