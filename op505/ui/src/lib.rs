//! OP505（N点Time/Level方式EG）のエディタパネル描画ロジック。eguiとsound-coreのみに依存し、
//! nice-plug/Tauri/cpalには依存しない（`ym38x6-ui`のOP505版に相当）。
//!
//! `ym38x6-ui`のパネルは`panel.xml`からのコード生成だが、こちらは手書きにしている
//! （設計判断: XML DSLの価値は「レイアウトを頻繁に微調整するノブ列」にあるが、OP505パネルの
//! 中心的な問いは「TimeEg（可変1〜8段・ループ・多段リリース）というカスタムウィジェットの
//! 中身」であってDSLの得意分野ではない。DSLにTimeEgを描く語彙も無く、形が決まっていない
//! ウィジェットのために拡張するのは順序が逆と判断した）。
//!
//! 各EG（OP×4のオペレーターEG＋PITCH/CUTOFF/GAIN FG）は`ui_core::time_eg_editor`
//! （折れ線ドラッグ編集/ノブ編集をタブで切り替えるハイブリッドエディタ、Step 8）へ委譲する。

use egui::Ui;
use ui_core::algorithm_diagram::{algorithm_diagram, carriers};
use ui_core::eg_preview::EgAmplitudeMapping;
use ui_core::knob::knob;
use ui_core::time_eg_editor::time_eg_editor;
use ui_core::{bool_checkbox, enum_selector, waveform_selector, LFO_FADE_MODE_NAMES, LFO_WAVEFORM_NAMES};

/// `time_eg_editor`の暫定固定サイズ（Step 2）。GRAPHモードの旧固定値（ヘッダ20+グラフ120+spin35
/// =175、幅260）を踏襲した仮値で、panel.xmlへの移行（Step 4）で`<time-eg-editor>`要素の
/// `<style>`が正本になり置き換わる。
const TIME_EG_EDITOR_SIZE: egui::Vec2 = egui::Vec2::new(260.0, 175.0);

// editor-wasm(op505_state.rs)がハンドル実装で使うため、ym38x6-uiと同じくトレイトを再エクスポートする。
pub use ui_core::{BoolParamHandle, IntParamHandle, TimeEgHandle};

/// Pitch FG（新規）／Cutoff FG（旧Filter EG）に共通の「TimeEg＋バイポーラDepth」一式。
pub struct Op505BipolarFgPanelParams<'a> {
    pub eg: Box<dyn TimeEgHandle + 'a>,
    /// バイポーラDepth（0〜255、中心128＝変調なし）。
    pub depth: Box<dyn IntParamHandle + 'a>,
}

/// オペレーター単位パラメーター一式。ym38x6-uiの`OperatorPanelParams`からAR/D1R/D2R/D1L/RR/
/// Floor/Loop/Curve（8フィールド）を除き、`eg: Box<dyn TimeEgHandle>`へ統合したもの
/// （その他11フィールドはym38x6-uiと同名・同型。実コードで突き合わせ済み）。
pub struct Op505OperatorPanelParams<'a> {
    pub tl: Box<dyn IntParamHandle + 'a>,
    pub eg: Box<dyn TimeEgHandle + 'a>,
    pub mul: Box<dyn IntParamHandle + 'a>,
    pub dt1: Box<dyn IntParamHandle + 'a>,
    pub ksr: Box<dyn IntParamHandle + 'a>,
    pub vel_sens: Box<dyn IntParamHandle + 'a>,
    pub op_fine_tune: Box<dyn IntParamHandle + 'a>,
    pub ame: Box<dyn BoolParamHandle + 'a>,
    pub waveform: Box<dyn IntParamHandle + 'a>,
    pub eg_shift: Box<dyn IntParamHandle + 'a>,
    pub level_scale: Box<dyn IntParamHandle + 'a>,
    pub velocity_gain: Box<dyn IntParamHandle + 'a>,
}

/// `draw_op505_panel`に渡すパラメーター一式。ym38x6-uiの`PanelParams`と異なりMASTER EFFECTは
/// 含まない（エンジン非依存のため38x6パネル側から操作できる。`EditorState`のマスターエフェクト
/// リファクタをこのステップに持ち込まないための意図的な省略）。
pub struct Op505PanelParams<'a> {
    // CHANNEL
    pub algorithm: Box<dyn IntParamHandle + 'a>,
    pub feedback: Box<dyn IntParamHandle + 'a>,
    // CHIP LFO
    pub chip_lfo_freq: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_pmd: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_amd: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_delay: Box<dyn IntParamHandle + 'a>,
    pub pms: Box<dyn IntParamHandle + 'a>,
    pub ams: Box<dyn IntParamHandle + 'a>,
    // TEXTURE LFO
    pub texture_lfo_rate: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_depth: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_delay: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_waveform: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_fade_mode: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_fade_time: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_offset: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_destination: Box<dyn IntParamHandle + 'a>,
    // FILTER
    pub cutoff: Box<dyn IntParamHandle + 'a>,
    pub resonance: Box<dyn IntParamHandle + 'a>,
    pub filter_type: Box<dyn IntParamHandle + 'a>,
    pub filter_self_oscillation: Box<dyn BoolParamHandle + 'a>,
    // PITCH FG / CUTOFF FG / GAIN FG
    pub pitch_fg: Op505BipolarFgPanelParams<'a>,
    pub cutoff_fg: Op505BipolarFgPanelParams<'a>,
    pub gain_fg: Box<dyn TimeEgHandle + 'a>,
    // OPERATORS
    pub operators: [Op505OperatorPanelParams<'a>; 4],
}

fn operator_panel(ui: &mut Ui, op: &Op505OperatorPanelParams, index: usize, carrier: bool) {
    ui.group(|ui| {
        ui.label(egui::RichText::new(format!("OP {}", index + 1)).strong());
        ui.horizontal(|ui| {
            time_eg_editor(ui, TIME_EG_EDITOR_SIZE, &*op.eg, EgAmplitudeMapping::DbLinear, op.tl.value().clamp(0, 255) as u8);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    knob(ui, &*op.tl, "TL");
                    knob(ui, &*op.mul, "MUL");
                    knob(ui, &*op.dt1, "DT1");
                    knob(ui, &*op.ksr, "KSR");
                });
                ui.horizontal(|ui| {
                    if carrier {
                        knob(ui, &*op.velocity_gain, "V.GAIN");
                    } else {
                        knob(ui, &*op.vel_sens, "VEL");
                    }
                    knob(ui, &*op.op_fine_tune, "FINE");
                    knob(ui, &*op.eg_shift, "EGSFT");
                    knob(ui, &*op.level_scale, "L.SCALE");
                });
                ui.horizontal(|ui| {
                    waveform_selector(ui, &*op.waveform, index);
                    bool_checkbox(ui, &*op.ame, "AM");
                });
            });
        });
    });
}

fn bipolar_fg_panel(ui: &mut Ui, fg: &Op505BipolarFgPanelParams) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            time_eg_editor(ui, TIME_EG_EDITOR_SIZE, &*fg.eg, EgAmplitudeMapping::AmplitudeLinear, 255);
            knob(ui, &*fg.depth, "DEPTH");
        });
    });
}

/// OP505パネル（OPERATOR×4 / CHANNEL・CHIP LFO・TEXTURE LFO / FILTER / PITCH FG・CUTOFF FG・
/// GAIN FG）を描画する。縦スクロールエリアを内部に含む。PRESETSサイドバー・ウィンドウ枠は
/// 呼び出し側（ホスト）が用意すること（`ym38x6_ui::draw_param_panel`と同じ契約）。
pub fn draw_op505_panel(ui: &mut Ui, params: &Op505PanelParams) {
    egui::ScrollArea::vertical().id_salt("op505_panel").show(ui, |ui| {
        ui.group(|ui| {
            ui.label(egui::RichText::new("CHANNEL").strong());
            ui.horizontal(|ui| {
                algorithm_diagram(ui, egui::vec2(110.0, 66.0), params.algorithm.value().clamp(0, 7) as u8);
                knob(ui, &*params.algorithm, "ALG");
                knob(ui, &*params.feedback, "FB");
            });
        });

        ui.horizontal(|ui| {
            ui.group(|ui| {
                ui.label(egui::RichText::new("CHIP LFO").strong());
                ui.horizontal(|ui| {
                    knob(ui, &*params.chip_lfo_freq, "CH.FRQ");
                    knob(ui, &*params.chip_lfo_pmd, "CH.PMD");
                    knob(ui, &*params.chip_lfo_amd, "CH.AMD");
                    knob(ui, &*params.chip_lfo_delay, "CH.DLY");
                    knob(ui, &*params.pms, "PMS");
                    knob(ui, &*params.ams, "AMS");
                });
            });
            ui.group(|ui| {
                ui.label(egui::RichText::new("TEXTURE LFO").strong());
                ui.horizontal(|ui| {
                    enum_selector(ui, &*params.texture_lfo_waveform, "TX.WAVE", &LFO_WAVEFORM_NAMES, 1);
                    enum_selector(ui, &*params.texture_lfo_fade_mode, "TX.FADE", &LFO_FADE_MODE_NAMES, 2);
                    knob(ui, &*params.texture_lfo_rate, "TX.RATE");
                    knob(ui, &*params.texture_lfo_depth, "TX.DEP");
                    knob(ui, &*params.texture_lfo_delay, "TX.DLY");
                    knob(ui, &*params.texture_lfo_fade_time, "TX.FTM");
                    knob(ui, &*params.texture_lfo_offset, "TX.OFS");
                    knob(ui, &*params.texture_lfo_destination, "TX.DEST");
                });
            });
        });

        ui.group(|ui| {
            ui.label(egui::RichText::new("FILTER").strong());
            ui.horizontal(|ui| {
                knob(ui, &*params.cutoff, "CUTOFF");
                knob(ui, &*params.resonance, "RESO");
                knob(ui, &*params.filter_type, "TYPE");
                bool_checkbox(ui, &*params.filter_self_oscillation, "SELF-OSC");
            });
        });

        bipolar_fg_panel(ui, &params.pitch_fg);
        bipolar_fg_panel(ui, &params.cutoff_fg);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                time_eg_editor(ui, TIME_EG_EDITOR_SIZE, &*params.gain_fg, EgAmplitudeMapping::AmplitudeLinear, 255);
            });
        });

        let carrier_ops = carriers(params.algorithm.value().clamp(0, 7) as u8);
        for (i, op) in params.operators.iter().enumerate() {
            operator_panel(ui, op, i, carrier_ops.contains(&i));
        }
    });
}
