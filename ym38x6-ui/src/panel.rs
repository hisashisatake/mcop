use crate::algorithm_diagram::algorithm_diagram;
use crate::eg_preview::{eg_preview, EgAmplitudeMapping};
use crate::knob::{bool_checkbox, knob};
use crate::layout::{self, leaf, row, row_grow, stack, Justify};
use crate::param_handle::{BoolParamHandle, IntParamHandle};
use crate::selector::{
    enum_selector, CHORUS_TYPE_NAMES, LFO_FADE_MODE_NAMES, LFO_WAVEFORM_NAMES, REVERB_TYPE_NAMES,
};
use crate::waveform::waveform_selector;
use sound_core::eg::EgParams;

/// MUL値(0〜15)→周波数比。`ym38x6_core::mapping::mul_to_ratio`と同じ表（0=0.5倍、1〜15=等倍）だが、
/// `ym38x6-ui`はnice-plug/Tauri非依存の`sound-core`のみに依存する方針のため、
/// OPヘッダの実効比率表示専用にこの極小テーブルをインライン複製する（内部エンジンは無改変）。
fn mul_to_ratio(mul: u8) -> f32 {
    const TABLE: [f32; 16] = [
        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    TABLE[(mul as usize).min(15)]
}

/// op_fine_tune値(0〜255、中心128)→セント。`ym38x6_core::mapping::op_fine_tune_to_cents`と同じ式
/// （中心128で±0、両端±1200セント）。上記`mul_to_ratio`と同じ理由でインライン複製する。
fn op_fine_tune_to_cents(v: u8) -> f32 {
    const OP_FINE_TUNE_RANGE_CENTS: f32 = 1200.0;
    (v as f32 - 128.0) / 128.0 * OP_FINE_TUNE_RANGE_CENTS
}

/// MUL＋FINE（op_fine_tune）による実効周波数比（DT1は除く）。OPヘッダの読み取り表示専用。
fn mul_fine_ratio(mul: u8, op_fine_tune: u8) -> f32 {
    mul_to_ratio(mul) * 2f32.powf(op_fine_tune_to_cents(op_fine_tune) / 1200.0)
}

/// オペレーター単位パラメーター一式（VST/gesture-app共通のハンドル束）。
pub struct OperatorPanelParams<'a> {
    pub tl: Box<dyn IntParamHandle + 'a>,
    pub ar: Box<dyn IntParamHandle + 'a>,
    pub d1r: Box<dyn IntParamHandle + 'a>,
    pub d2r: Box<dyn IntParamHandle + 'a>,
    pub d1l: Box<dyn IntParamHandle + 'a>,
    pub rr: Box<dyn IntParamHandle + 'a>,
    pub mul: Box<dyn IntParamHandle + 'a>,
    pub dt1: Box<dyn IntParamHandle + 'a>,
    pub ksr: Box<dyn IntParamHandle + 'a>,
    pub vel_sens: Box<dyn IntParamHandle + 'a>,
    pub op_fine_tune: Box<dyn IntParamHandle + 'a>,
    pub ame: Box<dyn BoolParamHandle + 'a>,
    pub waveform: Box<dyn IntParamHandle + 'a>,
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。`op_loop`がOFFの間は無効。
    pub floor: Box<dyn IntParamHandle + 'a>,
    /// OP単位ループEG（VCF/VCAのFGと同じLoop/Floor/Curve機構をFMオペレーターEGに開放したもの）。
    pub op_loop: Box<dyn BoolParamHandle + 'a>,
    /// 0=線形（角の立つ三角）/1=サイン風（レイズドコサインで角を丸める）。
    pub curve: Box<dyn BoolParamHandle + 'a>,
    /// EGSFT（TX81Z EG Shift）。EGの減衰レンジ(dB)を圧縮する（0〜255、既定0＝96dBフルレンジ）。
    pub eg_shift: Box<dyn IntParamHandle + 'a>,
    /// Level Scaling（ノート依存の出力レベル減衰、OPL系KSL相当）。0〜255、既定0＝スケーリングなし。
    pub level_scale: Box<dyn IntParamHandle + 'a>,
    /// キャリア出力へのベロシティ音量ゲイン深さ（0〜255、既定255＝フル）。
    /// `vel_sens`（明るさ、モジュレーター専用）とは独立・別軸。役割はALGで決まるため、
    /// パネルはそのOPがキャリアかモジュレーターかでVEL/V.GAINを排他的にグレーアウトする。
    pub velocity_gain: Box<dyn IntParamHandle + 'a>,
}

/// ファンクションジェネレーター（Pitch/Cutoff/Gain FG共通）のループ可能EGパラメーター一式。
/// `sound_core::eg::EgParams`と1:1対応する（新規の別部品を作らない設計、spec-sound.md参照）。
pub struct FgEgPanelParams<'a> {
    pub ar: Box<dyn IntParamHandle + 'a>,
    pub d1r: Box<dyn IntParamHandle + 'a>,
    pub d1l: Box<dyn IntParamHandle + 'a>,
    pub d2r: Box<dyn IntParamHandle + 'a>,
    pub rr: Box<dyn IntParamHandle + 'a>,
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。
    pub floor: Box<dyn IntParamHandle + 'a>,
    /// キーオンからAR開始までの遅延(0〜255、既定0＝遅延なし)。
    pub delay: Box<dyn IntParamHandle + 'a>,
    /// 0=ワンショット／1=ループ。
    pub loop_enabled: Box<dyn BoolParamHandle + 'a>,
    /// 0=線形（角の立つ三角）／1=サイン風（レイズドコサインで角を丸める）。
    pub curve: Box<dyn BoolParamHandle + 'a>,
}

/// Pitch FG（新規）／Cutoff FG（旧Filter EG）に共通の「共通EG＋バイポーラDepth」一式。
pub struct BipolarFgPanelParams<'a> {
    pub eg: FgEgPanelParams<'a>,
    /// バイポーラDepth（0〜255、中心128＝変調なし、128超＝＋方向、128未満＝−方向）。
    pub depth: Box<dyn IntParamHandle + 'a>,
}

/// `draw_param_panel`に渡すパラメーター一式。
/// OPERATOR / CHANNEL / TEXTURE LFO / CHIP LFO / PITCH FG / CUTOFF FG / GAIN FG / MASTER EFFECT
/// の全グリッドを含む。
pub struct PanelParams<'a> {
    // CHANNEL
    pub algorithm: Box<dyn IntParamHandle + 'a>,
    pub feedback: Box<dyn IntParamHandle + 'a>,
    // TEXTURE LFO（旧PERF LFO。5波形専用・焼き込み専用）
    pub texture_lfo_rate: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_depth: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_delay: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_waveform: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_fade_mode: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_fade_time: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_offset: Box<dyn IntParamHandle + 'a>,
    /// 質感LFOの行き先（0=Pitch/1=Volume/2=TL/3=Cutoff）。パネルではモジュラー風の
    /// パッチケーブルUI（`texture_lfo_patchbay`）で選ぶ。内部は単一enumのまま。
    pub texture_lfo_destination: Box<dyn IntParamHandle + 'a>,
    // CHIP LFO（旧TONE LFO。チップ内LFO、VCO差し替えで消えるレイヤー）
    pub chip_lfo_freq: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_pmd: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_amd: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_delay: Box<dyn IntParamHandle + 'a>,
    pub pms: Box<dyn IntParamHandle + 'a>,
    pub ams: Box<dyn IntParamHandle + 'a>,
    // PITCH FG（新規）
    pub pitch_fg: BipolarFgPanelParams<'a>,
    // FILTER（CUTOFF FG、旧Filter EG）
    pub cutoff: Box<dyn IntParamHandle + 'a>,
    pub resonance: Box<dyn IntParamHandle + 'a>,
    pub cutoff_fg: BipolarFgPanelParams<'a>,
    // VCA（GAIN FG、旧VCA EG。音量に負値は無いためDepthを持たずFloorが深さ役）
    pub gain_fg: FgEgPanelParams<'a>,
    // MASTER EFFECT
    pub rev_send: Box<dyn IntParamHandle + 'a>,
    pub reverb_type: Box<dyn IntParamHandle + 'a>,
    pub cho_send: Box<dyn IntParamHandle + 'a>,
    pub chorus_type: Box<dyn IntParamHandle + 'a>,
    pub reverb_time: Box<dyn IntParamHandle + 'a>,
    pub chorus_mod_rate: Box<dyn IntParamHandle + 'a>,
    pub chorus_mod_depth: Box<dyn IntParamHandle + 'a>,
    pub chorus_feedback: Box<dyn IntParamHandle + 'a>,
    pub chorus_send_to_reverb: Box<dyn IntParamHandle + 'a>,
    // OPERATORS
    pub operators: [OperatorPanelParams<'a>; 4],
}


// draw_param_panel: パラメーターグリッド（OP1〜4 / CHANNEL・TEXTURE LFO・CHIP LFO / PITCH FG /
// CUTOFF FG・GAIN FG / MASTER EFFECT）を描画する。縦スクロールエリアを内部に含む。
// PRESETSサイドバー・ウィンドウ枠（ResizableWindow等）・外側のCentralPanelは呼び出し側
// （ホスト）が用意すること。
//
// 本体は`panel.xml`から`build.rs`が生成する（`ym38x6-ui/build.rs`参照）。手編集は禁止、
// レイアウトを変える場合は`panel.xml`を編集して再ビルドする。
include!(concat!(env!("OUT_DIR"), "/panel_generated.rs"));

