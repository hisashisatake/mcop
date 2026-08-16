use crate::algorithm_diagram::algorithm_diagram;
use crate::knob::{bool_checkbox, knob};
use crate::layout::{self, leaf, row, row_grow, stack, stack_grow, Justify};
use crate::param_handle::{BoolParamHandle, IntParamHandle, TimeEgHandle};
use crate::selector::{enum_selector, CHORUS_TYPE_NAMES, LFO_FADE_MODE_NAMES, LFO_WAVEFORM_NAMES, REVERB_TYPE_NAMES};
use crate::time_eg_editor::time_eg_editor;
use crate::waveform::waveform_selector;
use ui_core::mapping::mul_fine_ratio;
use ui_core::{EgAmplitudeMapping, TimeEgProfile};

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

/// `draw_op505_panel`に渡すパラメーター一式。MASTER EFFECTS（Reverb/Chorus）はエンジン非依存の
/// 共有状態のため、`op505-vst`（DAWパラメーター）とgesture-app editor-wasm
/// （`EditorState`の既存フィールド、ym38x6パネルと共用）の両方から同じ9フィールドとして渡す
/// （フェーズ2で追加。当初はVST独自ストリップとして省く設計だったが、共有パネルへ統合した）。
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
    // MASTER EFFECTS
    pub rev_send: Box<dyn IntParamHandle + 'a>,
    pub reverb_type: Box<dyn IntParamHandle + 'a>,
    pub reverb_time: Box<dyn IntParamHandle + 'a>,
    pub cho_send: Box<dyn IntParamHandle + 'a>,
    pub chorus_type: Box<dyn IntParamHandle + 'a>,
    pub chorus_mod_rate: Box<dyn IntParamHandle + 'a>,
    pub chorus_mod_depth: Box<dyn IntParamHandle + 'a>,
    pub chorus_feedback: Box<dyn IntParamHandle + 'a>,
    pub chorus_send_to_reverb: Box<dyn IntParamHandle + 'a>,
    // OPERATORS
    pub operators: [Op505OperatorPanelParams<'a>; 4],
}

// draw_op505_panel: パラメーターグリッド（CHANNEL・CHIP LFO / TEXTURE LFO・FILTER / PITCH FG /
// CUTOFF FG / GAIN FG / OP1〜4）を描画する。縦スクロールエリアを内部に含む。PRESETSサイドバー・
// ウィンドウ枠は呼び出し側（ホスト）が用意すること（`ym38x6_ui::draw_param_panel`と同じ契約）。
//
// 本体は`panel.xml`から`build.rs`が生成する（`op505-ui/build.rs`参照）。手編集は禁止、
// レイアウトを変える場合は`panel.xml`を編集して再ビルドする。
include!(concat!(env!("OUT_DIR"), "/panel_generated.rs"));
