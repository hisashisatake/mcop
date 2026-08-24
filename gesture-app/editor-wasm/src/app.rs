use std::cell::{Cell, RefCell};
use std::rc::Rc;

use op505_core::Op505Patch;
use op505_ui::draw_op505_panel;

use crate::ipc;
use crate::keyboard;
use crate::op505_state::Op505State;
use crate::state::MasterEffectsState;

/// Open/Save/Save As/PRESETS選択/Bank・Programスピンの全経路が共有する表示状態。
/// `current_bank`/`current_program`は「見た目と動作を一致させる」ための唯一の正——
/// PRESETSリストのハイライトもこの値との比較で決める（独立した選択状態を持たない）。
#[derive(Clone)]
struct Identity {
    current_file_name: Rc<RefCell<Option<String>>>,
    current_patch_name: Rc<RefCell<String>>,
    current_bank: Rc<Cell<u16>>,
    current_program: Rc<Cell<u8>>,
    unsaved_changes: Rc<Cell<bool>>,
    /// `op505_list_bank_entries`で取得した、今のbankの担当ファイルの音色一覧。`apply()`のたびに
    /// 再取得し、Save/Save As/Openによるレジストリの変化を常に反映する。
    presets: Rc<RefCell<Vec<ipc::PresetEntry>>>,
    /// `apply()`が立てるプリセット読込直後1回ぶんの「次のsend_op505_patch()完了ではunsaved_changesを
    /// 立てない」抑制フラグ。`load_preset`はエンジンへの反映のため`patch`のdirtyを立てるが、
    /// それをそのまま`unsaved_changes`に波及させると、プリセットを読み込んだ瞬間から
    /// 未保存マーク`*`が付いてしまう（ユーザーはまだ何も編集していない）ため。
    suppress_unsaved: Rc<Cell<bool>>,
}

impl Identity {
    fn apply(&self, loaded: ipc::PatchIdentity) {
        *self.current_file_name.borrow_mut() = loaded.file_name;
        *self.current_patch_name.borrow_mut() = loaded.patch_name;
        self.current_bank.set(loaded.bank);
        self.current_program.set(loaded.program);
        crate::program_sync::set_current(loaded.bank, loaded.program);
        self.unsaved_changes.set(false);
        self.suppress_unsaved.set(true);
        crate::shift_keys::request_repaint();

        let presets = self.presets.clone();
        let bank = loaded.bank;
        wasm_bindgen_futures::spawn_local(async move {
            *presets.borrow_mut() = ipc::fetch_bank_entries(bank).await;
            crate::shift_keys::request_repaint();
        });
    }
}

/// 指定のbank/programをレジストリから読み込む（`ipc::load_preset`＝`op505_get_bank_program`、
/// ディスクの再検索はしない）。Bank/Programスピンの変更・PRESETSリストのクリック・起動直後の
/// 初期ロードのいずれもこれを呼ぶ（レジストリ自体がOpen/Save/Save Asで正しく更新されているため、
/// 「別ファイルへ飛ぶ経路」と「ファイル内に留まる経路」を分ける必要がない）。
/// `keep_fg`は`ipc::load_preset`参照（PRESETSリストのShift+クリック専用、他の呼び出し元は常にfalse）。
async fn handle_navigate(patch: Rc<RefCell<Op505Patch>>, dirty: Rc<Cell<bool>>, identity: Identity, bank: u16, program: u8, keep_fg: bool) {
    if let Some(loaded) = ipc::load_preset(&patch, &dirty, bank, program, keep_fg).await {
        identity.apply(loaded);
    }
}

/// Bank欄のハンドル。ノブ下の数値欄＋±ボタン（`ui_core::spin_control`）と同じ見た目・操作感にする
/// （メイン画面はHTML nativeのnumber inputだが、eguiでは同じ見た目を作れないため、
/// エディタ内の他のパラメーターと統一したルック＆フィールに合わせる）。
/// 値が変わったら`handle_navigate`と同じ経路でオンメモリのPresetBankから読み込む。
struct BankField {
    current_bank: Rc<Cell<u16>>,
    current_program: Rc<Cell<u8>>,
    patch: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    identity: Identity,
}

impl op505_ui::IntParamHandle for BankField {
    fn value(&self) -> i32 {
        self.current_bank.get() as i32
    }
    fn min(&self) -> i32 {
        0
    }
    fn max(&self) -> i32 {
        16383
    }
    fn default(&self) -> i32 {
        0
    }
    fn name(&self) -> String {
        "Bank".to_string()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        let bank = value.clamp(0, 16383) as u16;
        self.current_bank.set(bank);
        wasm_bindgen_futures::spawn_local(handle_navigate(
            self.patch.clone(),
            self.dirty.clone(),
            self.identity.clone(),
            bank,
            self.current_program.get(),
            false,
        ));
    }
    fn end_edit(&self) {}
}

/// 現在のbankの担当ファイルへ新規音色（"VoiceNNN"）を追加し、それを画面へ反映する
/// （PRESETSリスト末尾の「+ New Voice」用）。`source_patch`は通常クリックなら`Op505Patch::default()`、
/// Shift+クリックなら現在編集中のパッチのコピー（呼び出し元のUIコードが分岐する）。
async fn handle_add_preset(patch: Rc<RefCell<Op505Patch>>, dirty: Rc<Cell<bool>>, bank: u16, source_patch: Op505Patch, identity: Identity) {
    if let Some(loaded) = ipc::add_preset(&patch, &dirty, bank, source_patch).await {
        identity.apply(loaded);
    }
}

/// 現在選択中(current_program)の音色をDELETEキーで削除する（確認ダイアログはバックエンド側の
/// ネイティブYes/Noダイアログ、`ipc::delete_preset`参照）。削除できたら、残った音色一覧の先頭へ
/// 自動的に切り替える（削除された音色を編集し続ける状態を避けるため）。1件も残らなければ
/// リストを空にするだけで画面のパッチはそのまま（切り替え先が無いため）。
async fn handle_delete_preset(patch: Rc<RefCell<Op505Patch>>, dirty: Rc<Cell<bool>>, bank: u16, program: u8, identity: Identity) {
    let Some(remaining) = ipc::delete_preset(bank, program).await else { return };
    match remaining.first() {
        Some(first) => handle_navigate(patch, dirty, identity, bank, first.program, false).await,
        None => {
            *identity.presets.borrow_mut() = remaining;
            crate::shift_keys::request_repaint();
        }
    }
}

/// presets_dir内/外を区別しない任意の`.op505`ファイルをネイティブOpenダイアログで選ぶ。
/// `bank`はダイアログの初期ディレクトリ決定にのみ使う。
async fn handle_open_patch_file(patch: Rc<RefCell<Op505Patch>>, dirty: Rc<Cell<bool>>, bank: u16, identity: Identity) {
    if let Some(loaded) = ipc::open_patch_file(&patch, &dirty, bank).await {
        identity.apply(loaded);
    }
}

/// 現在のbank/programの担当ファイルへ上書き保存する。名前入力欄で音色名を変更していれば
/// それも一緒に保存される。
async fn handle_save_patch_overwrite(patch: Rc<RefCell<Op505Patch>>, bank: u16, program: u8, patch_name: String, identity: Identity) {
    if let Some(saved) = ipc::save_patch_overwrite(&patch, bank, program, patch_name).await {
        identity.apply(saved);
    }
}

/// ネイティブSaveダイアログで保存先を選ぶ。bank/programは今表示されている値をそのまま書き込む
/// （自動採番はしない）。成功したら以後の上書き保存先として記録する。
async fn handle_save_patch_as(
    patch: Rc<RefCell<Op505Patch>>,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
    identity: Identity,
) {
    if let Some(saved) = ipc::save_patch_as(&patch, patch_name, bank, program, default_file_name).await {
        identity.apply(saved);
    }
}

/// gesture-app埋め込み用エディタ本体。`draw_op505_panel`(op505-ui)を1回呼ぶだけでVSTと同じ
/// ノブパネルを描画する。値変更はローカル状態へ即時反映しつつ`dirty`フラグを立て、フレーム末尾で
/// まとめて`op505_set_patch`/`set_master_effects`へ送る（ドラッグ中に1サンプルごとIPCを叩くと
/// 過負荷になるための1フレーム1回バッチ送信）。
pub struct EditorApp {
    op505: Op505State,
    master_effects: Rc<RefCell<MasterEffectsState>>,
    master_effects_dirty: Rc<Cell<bool>>,
    /// ZXCV(白鍵)/ASDF(黒鍵)ミニ鍵盤の表示オクターブ＋押下状態。
    keyboard: keyboard::KeyboardState,
    /// `op505_list_bank_entries`で取得したプリセット一覧（取得完了まで空）。非同期タスクと共有する。
    presets: Rc<RefCell<Vec<ipc::PresetEntry>>>,
    /// Open/Save As/PRESETS選択/Bank・Programスピンで読み込んだファイルの実ファイル名。Noneなら
    /// 未Openで「Save」は無効化する（presets_dir内/外を区別しない、Open/Save/Save As共通の状態）。
    current_file_name: Rc<RefCell<Option<String>>>,
    /// 音色名の入力欄（`PresetEntry.name`）。ファイル名とは独立して編集できる
    /// （1ファイルに複数音色が入りうるため、ファイル名と音色名は別概念）。
    current_patch_name: Rc<RefCell<String>>,
    /// 今表示中のbank/program。PRESETSリストのハイライトもこの値との比較で決める
    /// （独立した選択状態を持たない。Open等で読み込んだファイルの実際のbank/programにも追従する）。
    current_bank: Rc<Cell<u16>>,
    current_program: Rc<Cell<u8>>,
    /// 直近の保存以降にパラメーターまたは音色名を変更したか（フレーム末尾のdirty処理に便乗して立てる）。
    unsaved_changes: Rc<Cell<bool>>,
    /// `Identity::apply()`が立てる、次のsend_op505_patch()完了1回ぶんの`unsaved_changes`抑制フラグ
    /// （`Identity`のdoc参照）。
    suppress_unsaved: Rc<Cell<bool>>,
}

impl EditorApp {
    pub fn new() -> Self {
        // PRESETSリストは「今開いているファイルの中身」なので、ここでは空のまま用意するだけでよい
        // （下の初期`handle_navigate`が完了すると、その`Identity::apply()`が自動で取得・反映する）。
        let presets = Rc::new(RefCell::new(Vec::new()));

        let op505 = Op505State::new();
        let master_effects = Rc::new(RefCell::new(MasterEffectsState::default()));
        let master_effects_dirty = Rc::new(Cell::new(false));
        let current_file_name = Rc::new(RefCell::new(None));
        let current_patch_name = Rc::new(RefCell::new(String::new()));
        let current_bank = Rc::new(Cell::new(0));
        let current_program = Rc::new(Cell::new(0));
        let unsaved_changes = Rc::new(Cell::new(false));
        let suppress_unsaved = Rc::new(Cell::new(false));

        // メイン画面（main.js）のBank/Program欄の現在値を初期値として読み込む。起動直後から
        // エディタとメイン画面の選択が一致した状態にする（見た目と動作を一致させるため）。
        let identity = Identity {
            current_file_name: current_file_name.clone(),
            current_patch_name: current_patch_name.clone(),
            current_bank: current_bank.clone(),
            current_program: current_program.clone(),
            unsaved_changes: unsaved_changes.clone(),
            presets: presets.clone(),
            suppress_unsaved: suppress_unsaved.clone(),
        };
        let (initial_bank, initial_program) = ipc::read_program_fields();
        wasm_bindgen_futures::spawn_local(handle_navigate(op505.patch.clone(), op505.dirty.clone(), identity, initial_bank, initial_program, false));

        Self {
            op505,
            master_effects,
            master_effects_dirty,
            keyboard: keyboard::KeyboardState::new(),
            presets,
            current_file_name,
            current_patch_name,
            current_bank,
            current_program,
            unsaved_changes,
            suppress_unsaved,
        }
    }

    fn identity(&self) -> Identity {
        Identity {
            current_file_name: self.current_file_name.clone(),
            current_patch_name: self.current_patch_name.clone(),
            current_bank: self.current_bank.clone(),
            current_program: self.current_program.clone(),
            unsaved_changes: self.unsaved_changes.clone(),
            presets: self.presets.clone(),
            suppress_unsaved: self.suppress_unsaved.clone(),
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::shift_keys::register_context(ui.ctx());
        // main.js側でBank/Program欄の変更が起きていれば、メイン画面の現在値を読み直して
        // `handle_navigate`する（PRESETSサイドバーを常にメイン画面のBank/Program欄と一致させる
        // 唯一の経路。`engine_sync.rs`参照）。
        if crate::engine_sync::take_selection_stale() {
            let (bank, program) = ipc::read_program_fields();
            wasm_bindgen_futures::spawn_local(handle_navigate(self.op505.patch.clone(), self.op505.dirty.clone(), self.identity(), bank, program, false));
        }

        // 鍵盤は画面下に端から端まで張り付ける（枠線・余白なし）。塗りはVST(op505-vst)と同じ
        // 標準ダークテーマのpanel_fillに合わせる（独自の色を増やさず一貫させるため）。
        // 他のパネルより先に確保することで、PRESETSサイドバーより下まで全幅で届く
        // （Panel::leftを先に確保すると鍵盤がサイドバー分狭くなってしまうため、この順序が重要）。
        let panel_fill = ui.visuals().panel_fill;
        egui::Panel::bottom("keyboard_panel")
            .frame(egui::Frame::NONE.fill(panel_fill))
            .show_inside(ui, |ui| {
                keyboard::draw_keyboard(ui, &mut self.keyboard);
            });

        // プリセット一覧（左サイドバー、VSTのPRESETSパネルと同じ並び）。鍵盤の上の領域のみを占める。
        egui::Panel::left("presets_panel")
            .resizable(true)
            .default_size(180.0)
            .min_size(120.0)
            .max_size(400.0)
            .show_inside(ui, |ui| {
                let patch = self.op505.patch.clone();
                let dirty = self.op505.dirty.clone();

                // Open/Save/Save As（presets_dir内/外を区別しない、ネイティブファイルダイアログ）。
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        wasm_bindgen_futures::spawn_local(handle_open_patch_file(patch.clone(), dirty.clone(), self.current_bank.get(), self.identity()));
                    }
                    let save_enabled = self.current_file_name.borrow().is_some();
                    if ui.add_enabled(save_enabled, egui::Button::new("Save")).clicked() {
                        wasm_bindgen_futures::spawn_local(handle_save_patch_overwrite(
                            patch.clone(),
                            self.current_bank.get(),
                            self.current_program.get(),
                            self.current_patch_name.borrow().clone(),
                            self.identity(),
                        ));
                    }
                    if ui.button("Save As").clicked() {
                        let patch_name = self.current_patch_name.borrow().clone();
                        // ダイアログの提案ファイル名は「今開いているファイル名」を優先する
                        // （Save Asはバンク全体を書き出すため、個々の音色名より自然）。
                        // 何も開いていなければ音色名、それも空なら"patch"にフォールバックする。
                        let default_file_name = self
                            .current_file_name
                            .borrow()
                            .as_deref()
                            .and_then(|f| f.strip_suffix(".op505"))
                            .filter(|f| !f.is_empty())
                            .map(str::to_string)
                            .or_else(|| (!patch_name.is_empty()).then(|| patch_name.clone()))
                            .unwrap_or_else(|| "patch".to_string());
                        wasm_bindgen_futures::spawn_local(handle_save_patch_as(
                            patch.clone(),
                            patch_name,
                            self.current_bank.get(),
                            self.current_program.get(),
                            default_file_name,
                            self.identity(),
                        ));
                    }
                });

                // Bank（presets_dir内/外を区別しない、常に見える唯一の「今何を編集しているか」）。
                // 他のパラメーターと同じ数値欄＋±ボタン（ui_core::spin_control）を使う。
                // 値が変わったらpresets_dir全体から(bank, 現在のprogram)を探して読み込む
                // （＝handle_navigate、BankField::set参照。別ファイルへ飛びうる唯一の操作）。
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Bank").text_style(egui::TextStyle::Body));
                    let bank_field = BankField {
                        current_bank: self.current_bank.clone(),
                        current_program: self.current_program.clone(),
                        patch: patch.clone(),
                        dirty: dirty.clone(),
                        identity: self.identity(),
                    };
                    ui_core::spin_control(ui, &bank_field, egui::TextStyle::Body, 44.0);
                });
                let file_label = self.current_file_name.borrow().clone().unwrap_or_else(|| "(unsaved)".to_string());
                let mark = if self.unsaved_changes.get() { "*" } else { "" };
                ui.label(format!("{file_label}{mark}"));
                // 音色名（PresetEntry.name）はここで直接編集できる。ファイル名とは独立した項目
                // （Save時にはこの内容がエントリ名として書き込まれる）。
                let mut patch_name = self.current_patch_name.borrow().clone();
                if ui.text_edit_singleline(&mut patch_name).changed() {
                    *self.current_patch_name.borrow_mut() = patch_name;
                    self.unsaved_changes.set(true);
                }
                ui.separator();

                // 選択中(current_program)の音色をDELETEキーで削除する（対象はマウス位置に依存せず、
                // 常にハイライト中の音色に固定）。名前編集欄など他のテキスト入力にフォーカスがある間は
                // 文字削除と衝突しうるため無効化する。
                let any_text_focused = ui.memory(|m| m.focused().is_some());
                if !any_text_focused && !self.presets.borrow().is_empty() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    wasm_bindgen_futures::spawn_local(handle_delete_preset(
                        patch.clone(),
                        dirty.clone(),
                        self.current_bank.get(),
                        self.current_program.get(),
                        self.identity(),
                    ));
                }

                // 「今開いているファイル」自身の音色一覧（presets_dir全体のブラウザではない）。
                ui.label(egui::RichText::new("PRESETS").strong());
                egui::ScrollArea::vertical().id_salt("presets").auto_shrink([false, false]).show(ui, |ui| {
                    for preset in self.presets.borrow().iter() {
                        let label = format!("{:03} {}", preset.program, preset.name);
                        let selected = preset.program == self.current_program.get();
                        if ui.selectable_label(selected, &label).clicked() {
                            // Shift+クリックなら、PITCH FG/CUTOFF FG/GAIN FGは今の設定を保ったまま
                            // それ以外だけ差し替える（`ipc::load_preset`の`keep_fg`参照）。
                            let keep_fg = ui.input(|i| i.modifiers.shift);
                            // レジストリを引くだけ（ディスク再検索なし）。今のbankのまま
                            // programだけ切り替える＝別ファイルへは飛ばない（handle_navigate参照）。
                            self.current_program.set(preset.program);
                            wasm_bindgen_futures::spawn_local(handle_navigate(
                                patch.clone(),
                                dirty.clone(),
                                self.identity(),
                                self.current_bank.get(),
                                preset.program,
                                keep_fg,
                            ));
                        }
                    }
                    // リスト末尾の「音色追加」行。通常クリックは新規デフォルト音色（"VoiceNNN"）を
                    // 追加し、Shift+クリックは代わりに現在編集中のパッチをコピーして追加する
                    // （既存音色を土台にバリエーションを作りたい用途）。
                    if ui.selectable_label(false, "+ New Voice").clicked() {
                        let copy_current = ui.input(|i| i.modifiers.shift);
                        let source_patch = if copy_current { *patch.borrow() } else { Op505Patch::default() };
                        wasm_bindgen_futures::spawn_local(handle_add_preset(patch.clone(), dirty.clone(), self.current_bank.get(), source_patch, self.identity()));
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let panel = self.op505.build_panel_params(&self.master_effects, &self.master_effects_dirty);
            draw_op505_panel(ui, &panel);
        });

        // プリセット読込直後の1回だけは`suppress_unsaved`が消費し、未保存マーク`*`を立てない
        // （`Identity`のdoc参照。読込自体がdirtyを立てるため、素通しすると読んだ瞬間`*`が付いてしまう）。
        if self.op505.dirty.get() {
            self.op505.dirty.set(false);
            ipc::send_op505_patch(&self.op505.patch.borrow());
            if self.suppress_unsaved.replace(false) {
                // 消費済み。
            } else {
                self.unsaved_changes.set(true);
            }
        }
        if self.master_effects_dirty.get() {
            self.master_effects_dirty.set(false);
            ipc::send_master_effects(&self.master_effects.borrow());
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}
