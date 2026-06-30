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
    /// 現在選択中のプリセット（左パネルのハイライト表示用）。
    selected_preset: Option<(u16, u8)>,
}

impl EditorApp {
    pub fn new() -> Self {
        let presets = Rc::new(RefCell::new(Vec::new()));
        let presets_for_fetch = presets.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let fetched = ipc::fetch_presets().await;
            *presets_for_fetch.borrow_mut() = fetched;
            crate::shift_keys::request_repaint();
        });

        Self {
            state: Rc::new(RefCell::new(EditorState::default())),
            dirty: Rc::new(Cell::new(false)),
            keyboard: keyboard::KeyboardState::new(),
            presets,
            selected_preset: None,
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
            .resizable(false)
            .exact_size(180.0)
            .show_inside(ui, |ui| {
                ui.label(egui::RichText::new("PRESETS").strong());
                egui::ScrollArea::vertical().id_salt("presets").show(ui, |ui| {
                    for preset in self.presets.borrow().iter() {
                        let label = if preset.bank == 0 {
                            format!("{:03} {}", preset.program, preset.name)
                        } else {
                            format!("[{:04X}:{:03}] {}", preset.bank, preset.program, preset.name)
                        };
                        let selected = self.selected_preset == Some((preset.bank, preset.program));
                        if ui.selectable_label(selected, &label).clicked() {
                            self.selected_preset = Some((preset.bank, preset.program));
                            wasm_bindgen_futures::spawn_local(ipc::load_preset(
                                self.state.clone(),
                                self.dirty.clone(),
                                preset.bank,
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
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}
