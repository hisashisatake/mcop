use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ym38x6_ui::{draw_param_panel, OperatorPanelParams, PanelParams};

use crate::handle::{int_field, op_int_field, BoolField, IntField};
use crate::ipc;
use crate::keyboard;
use crate::state::EditorState;


// オペレーターインデックスをconst genericsにしているのは、`IntField`/`BoolField`の
// get/setが（クロージャの環境キャプチャを避けて軽量にするため）プレーンな関数ポインタ
// `fn(&EditorState) -> i32`型である一方、配列インデックスは本来は実行時の値だから。
// 非キャプチャクロージャしか`fn`型へ強制変換できないため、インデックスをconst genericsの
// `I`として埋め込み、各モノモーフィック化(I=0,1,2,3)ごとにコンパイル時定数として扱わせている。
fn operator_panel<const I: usize>(state: &Rc<RefCell<EditorState>>, dirty: &Rc<Cell<bool>>) -> OperatorPanelParams<'static> {
    macro_rules! op {
        ($field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
            op_int_field!(state, dirty, I, $field, $name, $min, $max, $default)
        };
    }
    OperatorPanelParams {
        tl: op!(tl, "TL", 0, 255, 200),
        ar: op!(ar, "AR", 0, 255, 255),
        d1r: op!(d1r, "D1R", 0, 255, 100),
        d2r: op!(d2r, "D2R", 0, 255, 80),
        d1l: op!(d1l, "D1L", 0, 255, 180),
        rr: op!(rr, "RR", 0, 255, 150),
        mul: op!(mul, "MUL", 0, 15, 1),
        dt1: op!(dt1, "DT1", 0, 255, 128),
        ksr: op!(ksr, "KSR", 0, 255, 64),
        vel_sens: op!(vel_sens, "VEL", 0, 255, 0),
        op_fine_tune: op!(op_fine_tune, "FINE", 0, 255, 128),
        ame: Box::new(BoolField {
            state: state.clone(),
            dirty: dirty.clone(),
            get: |s: &EditorState| s.operators[I].ame,
            set: |s: &mut EditorState, v: bool| s.operators[I].ame = v,
        }),
        waveform: op!(waveform, "Waveform", 0, 255, 0),
    }
}

fn build_panel_params(state: &Rc<RefCell<EditorState>>, dirty: &Rc<Cell<bool>>) -> PanelParams<'static> {
    macro_rules! ch {
        ($field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
            int_field!(state, dirty, $field, $name, $min, $max, $default)
        };
    }
    PanelParams {
        algorithm: ch!(algorithm, "Algorithm", 0, 7, 0),
        feedback: ch!(feedback, "Feedback", 0, 255, 0),
        lfo_rate: ch!(lfo_rate, "Perf LFO Rate", 0, 255, 0),
        lfo_depth: ch!(lfo_depth, "Perf LFO Depth", 0, 255, 0),
        lfo_delay: ch!(lfo_delay, "Perf LFO Delay", 0, 255, 0),
        tone_freq: ch!(tone_freq, "Tone LFO Freq", 0, 255, 0),
        tone_pmd: ch!(tone_pmd, "Tone LFO PMD", 0, 255, 0),
        tone_amd: ch!(tone_amd, "Tone LFO AMD", 0, 255, 0),
        tone_delay: ch!(tone_delay, "Tone LFO Delay", 0, 255, 0),
        pms: ch!(pms, "PMS", 0, 255, 0),
        ams: ch!(ams, "AMS", 0, 255, 0),
        cutoff: ch!(cutoff, "Filter Cutoff", 0, 255, 255),
        resonance: ch!(resonance, "Filter Resonance", 0, 255, 0),
        feg_a: ch!(feg_a, "Filter EG Attack", 0, 255, 0),
        feg_d: ch!(feg_d, "Filter EG Decay", 0, 255, 0),
        feg_s: ch!(feg_s, "Filter EG Sustain", 0, 255, 0),
        feg_r: ch!(feg_r, "Filter EG Release", 0, 255, 0),
        feg_depth: ch!(feg_depth, "Filter EG Depth", 0, 255, 0),
        rev_send: ch!(rev_send, "Reverb Send", 0, 255, 0),
        reverb_type: ch!(reverb_type, "Reverb Type", 0, 7, 3),
        cho_send: ch!(cho_send, "Chorus Send", 0, 255, 0),
        chorus_type: ch!(chorus_type, "Chorus Type", 0, 7, 0),
        reverb_time: ch!(reverb_time, "Reverb Time", 0, 255, 128),
        chorus_mod_rate: ch!(chorus_mod_rate, "Chorus Mod Rate", 0, 255, 128),
        chorus_mod_depth: ch!(chorus_mod_depth, "Chorus Mod Depth", 0, 255, 128),
        chorus_feedback: ch!(chorus_feedback, "Chorus Feedback", 0, 255, 0),
        chorus_send_to_reverb: ch!(chorus_send_to_reverb, "Chorus Send To Reverb", 0, 255, 0),
        operators: [
            operator_panel::<0>(state, dirty),
            operator_panel::<1>(state, dirty),
            operator_panel::<2>(state, dirty),
            operator_panel::<3>(state, dirty),
        ],
    }
}

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
    /// `list_bank_entries`で取得した、今のbankの担当ファイルの音色一覧。`apply()`のたびに
    /// 再取得し、Save/Save As/Openによるレジストリの変化を常に反映する。
    presets: Rc<RefCell<Vec<ipc::PresetEntry>>>,
}

impl Identity {
    fn apply(&self, loaded: ipc::PatchIdentity) {
        *self.current_file_name.borrow_mut() = loaded.file_name;
        *self.current_patch_name.borrow_mut() = loaded.patch_name;
        self.current_bank.set(loaded.bank);
        self.current_program.set(loaded.program);
        crate::program_sync::set_current(loaded.bank, loaded.program);
        self.unsaved_changes.set(false);
        crate::shift_keys::request_repaint();

        let presets = self.presets.clone();
        let bank = loaded.bank;
        wasm_bindgen_futures::spawn_local(async move {
            *presets.borrow_mut() = ipc::fetch_bank_entries(bank).await;
            crate::shift_keys::request_repaint();
        });
    }
}

/// 指定のbank/programをレジストリから読み込む（`ipc::load_preset`＝`get_bank_program`、
/// ディスクの再検索はしない）。Bank/Programスピンの変更・PRESETSリストのクリック・起動直後の
/// 初期ロードのいずれもこれを呼ぶ（レジストリ自体がOpen/Save/Save Asで正しく更新されているため、
/// 「別ファイルへ飛ぶ経路」と「ファイル内に留まる経路」を分ける必要がない）。
async fn handle_navigate(state: Rc<RefCell<EditorState>>, dirty: Rc<Cell<bool>>, identity: Identity, bank: u16, program: u8) {
    if let Some(loaded) = ipc::load_preset(state, dirty, bank, program).await {
        identity.apply(loaded);
    }
}

/// Bank欄のハンドル。ノブ下の数値欄＋±ボタン（`ym38x6_ui::spin_control`）と同じ見た目・操作感にする
/// （メイン画面はHTML nativeのnumber inputだが、eguiでは同じ見た目を作れないため、
/// エディタ内の他のパラメーターと統一したルック＆フィールに合わせる）。
/// 値が変わったら`handle_navigate`と同じ経路でオンメモリのPresetBankから読み込む。
struct BankField {
    current_bank: Rc<Cell<u16>>,
    current_program: Rc<Cell<u8>>,
    state: Rc<RefCell<EditorState>>,
    dirty: Rc<Cell<bool>>,
    identity: Identity,
}

impl ym38x6_ui::IntParamHandle for BankField {
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
            self.state.clone(),
            self.dirty.clone(),
            self.identity.clone(),
            bank,
            self.current_program.get(),
        ));
    }
    fn end_edit(&self) {}
}

/// presets_dir内/外を区別しない任意の`.38x6`ファイルをネイティブOpenダイアログで選ぶ。
/// `bank`はダイアログの初期ディレクトリ決定にのみ使う。
async fn handle_open_patch_file(state: Rc<RefCell<EditorState>>, dirty: Rc<Cell<bool>>, bank: u16, identity: Identity) {
    if let Some(loaded) = ipc::open_patch_file(state, dirty, bank).await {
        identity.apply(loaded);
    }
}

/// 現在のbank/programの担当ファイルへ上書き保存する。名前入力欄で音色名を変更していれば
/// それも一緒に保存される。
async fn handle_save_patch_overwrite(
    state: Rc<RefCell<EditorState>>,
    bank: u16,
    program: u8,
    patch_name: String,
    identity: Identity,
) {
    if let Some(saved) = ipc::save_patch_overwrite(state, bank, program, patch_name).await {
        identity.apply(saved);
    }
}

/// ネイティブSaveダイアログで保存先を選ぶ。bank/programは今表示されている値をそのまま書き込む
/// （自動採番はしない）。成功したら以後の上書き保存先として記録する。
async fn handle_save_patch_as(
    state: Rc<RefCell<EditorState>>,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
    identity: Identity,
) {
    if let Some(saved) = ipc::save_patch_as(state, patch_name, bank, program, default_file_name).await {
        identity.apply(saved);
    }
}

/// gesture-app埋め込み用エディタ本体。`draw_param_panel`(ym38x6-ui)を1回呼ぶだけで
/// VSTと同じノブパネルを描画する。値変更はローカル`EditorState`へ即時反映しつつ
/// `dirty`フラグを立て、フレーム末尾でまとめて`ym38x6_set_patch`/`set_master_effects`へ送る
/// （ドラッグ中に1サンプルごとIPCを叩くと過負荷になるための1フレーム1回バッチ送信）。
pub struct EditorApp {
    state: Rc<RefCell<EditorState>>,
    dirty: Rc<Cell<bool>>,
    /// ZXCV(白鍵)/ASDF(黒鍵)ミニ鍵盤の表示オクターブ＋押下状態。
    keyboard: keyboard::KeyboardState,
    /// `list_presets`で取得したプリセット一覧（取得完了まで空）。非同期タスクと共有する。
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
}

impl EditorApp {
    pub fn new() -> Self {
        // PRESETSリストは「今開いているファイルの中身」なので、ここでは空のまま用意するだけでよい
        // （下の初期`handle_navigate`が完了すると、その`Identity::apply()`が自動で取得・反映する）。
        let presets = Rc::new(RefCell::new(Vec::new()));

        let state = Rc::new(RefCell::new(EditorState::default()));
        let dirty = Rc::new(Cell::new(false));
        let current_file_name = Rc::new(RefCell::new(None));
        let current_patch_name = Rc::new(RefCell::new(String::new()));
        let current_bank = Rc::new(Cell::new(0));
        let current_program = Rc::new(Cell::new(0));
        let unsaved_changes = Rc::new(Cell::new(false));

        // メイン画面（main.js）のBank/Program欄の現在値を初期値として読み込む。起動直後から
        // エディタとメイン画面の選択が一致した状態にする（見た目と動作を一致させるため）。
        let (initial_bank, initial_program) = ipc::read_program_fields();
        wasm_bindgen_futures::spawn_local(handle_navigate(
            state.clone(),
            dirty.clone(),
            Identity {
                current_file_name: current_file_name.clone(),
                current_patch_name: current_patch_name.clone(),
                current_bank: current_bank.clone(),
                current_program: current_program.clone(),
                unsaved_changes: unsaved_changes.clone(),
                presets: presets.clone(),
            },
            initial_bank,
            initial_program,
        ));

        Self {
            state,
            dirty,
            keyboard: keyboard::KeyboardState::new(),
            presets,
            current_file_name,
            current_patch_name,
            current_bank,
            current_program,
            unsaved_changes,
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
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::shift_keys::register_context(ui.ctx());

        // 鍵盤は画面下に端から端まで張り付ける（枠線・余白なし）。塗りはVST(ym38x6-vst)と同じ
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
                // Open/Save/Save As（presets_dir内/外を区別しない、ネイティブファイルダイアログ）。
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        wasm_bindgen_futures::spawn_local(handle_open_patch_file(
                            self.state.clone(),
                            self.dirty.clone(),
                            self.current_bank.get(),
                            self.identity(),
                        ));
                    }
                    let save_enabled = self.current_file_name.borrow().is_some();
                    if ui.add_enabled(save_enabled, egui::Button::new("Save")).clicked() {
                        wasm_bindgen_futures::spawn_local(handle_save_patch_overwrite(
                            self.state.clone(),
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
                            .and_then(|f| f.strip_suffix(".38x6"))
                            .filter(|f| !f.is_empty())
                            .map(str::to_string)
                            .or_else(|| (!patch_name.is_empty()).then(|| patch_name.clone()))
                            .unwrap_or_else(|| "patch".to_string());
                        wasm_bindgen_futures::spawn_local(handle_save_patch_as(
                            self.state.clone(),
                            patch_name,
                            self.current_bank.get(),
                            self.current_program.get(),
                            default_file_name,
                            self.identity(),
                        ));
                    }
                });

                // Bank（presets_dir内/外を区別しない、常に見える唯一の「今何を編集しているか」）。
                // 他のパラメーターと同じ数値欄＋±ボタン（ym38x6_ui::spin_control）を使う。
                // 値が変わったらpresets_dir全体から(bank, 現在のprogram)を探して読み込む
                // （＝handle_navigate、BankField::set参照。別ファイルへ飛びうる唯一の操作）。
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Bank").text_style(egui::TextStyle::Body));
                    let bank_field = BankField {
                        current_bank: self.current_bank.clone(),
                        current_program: self.current_program.clone(),
                        state: self.state.clone(),
                        dirty: self.dirty.clone(),
                        identity: self.identity(),
                    };
                    ym38x6_ui::spin_control(ui, &bank_field, egui::TextStyle::Body);
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

                // 「今開いているファイル」自身の音色一覧（presets_dir全体のブラウザではない）。
                ui.label(egui::RichText::new("PRESETS").strong());
                egui::ScrollArea::vertical().id_salt("presets").show(ui, |ui| {
                    for preset in self.presets.borrow().iter() {
                        let label = format!("{:03} {}", preset.program, preset.name);
                        let selected = preset.program == self.current_program.get();
                        if ui.selectable_label(selected, &label).clicked() {
                            // レジストリを引くだけ（ディスク再検索なし）。今のbankのまま
                            // programだけ切り替える＝別ファイルへは飛ばない（handle_navigate参照）。
                            self.current_program.set(preset.program);
                            wasm_bindgen_futures::spawn_local(handle_navigate(
                                self.state.clone(),
                                self.dirty.clone(),
                                self.identity(),
                                self.current_bank.get(),
                                preset.program,
                            ));
                        }
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let panel = build_panel_params(&self.state, &self.dirty);
            draw_param_panel(ui, &panel);
        });

        if self.dirty.get() {
            self.dirty.set(false);
            ipc::send_patch(&self.state.borrow());
            self.unsaved_changes.set(true);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}
