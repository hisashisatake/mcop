use nice_plug::prelude::*;
use serde::{Deserialize, Serialize};
use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
use std::sync::{Arc, RwLock};

pub(crate) const DEFAULT_ALGORITHM: u8 = 0;
pub(crate) const DEFAULT_REVERB_TIME: u8 = 128;
pub(crate) const DEFAULT_CHORUS_MOD_RATE: u8 = 128;
pub(crate) const DEFAULT_CHORUS_MOD_DEPTH: u8 = 128;
pub(crate) const DEFAULT_CHORUS_FEEDBACK: u8 = 0;
pub(crate) const DEFAULT_CHORUS_SEND_TO_REVERB: u8 = 0;
pub(crate) const DEFAULT_REVERB_TYPE: u8 = 3;
pub(crate) const DEFAULT_CHORUS_TYPE: u8 = 0;

/// 中央128のバイポーラパラメーター（0〜255のオフセットバイナリ）を、DAWのオートメーション表示でも
/// -128〜+127の符号付きで見せる。エディタ側は`ui_core::BipolarHandle`が同じ写像を行うので、
/// **同じノブがUIとDAWで違う数字を出す**という不整合を防ぐためにここでも揃える。
/// 対象は`(生値 - 128) / 128`を係数として使うもの（P.DEP±/F.DEP±/DT1/FINE/TX.OFS）。
fn bipolar_int(param: IntParam) -> IntParam {
    param
        .with_value_to_string(Arc::new(|v| {
            let centered = v - 128;
            if centered == 0 {
                "0".to_string()
            } else {
                format!("{centered:+}")
            }
        }))
        .with_string_to_value(Arc::new(|s| {
            // Rustのi32パースは先頭の'+'をそのまま受け付けるため、符号の前処理は不要。
            s.trim().parse::<i32>().ok().map(|centered| (centered + 128).clamp(0, 255))
        }))
}

/// キーオンから即座にフルレベルへ達しサステインし、キーオフでレベル0へ落ちる2段EG。
/// DAWパラメーターに載らないpersist状態のEG群（`Op505EgBank`）の既定値に使う。
/// `TimeEgParams::default()`（全段time=0/level=0=無音）をそのまま使うとプラグイン挿入直後に
/// 無音になるため、これで明示的に「鳴る」状態にする。
///
/// 段1（`TimeStage::default()`＝time 0/level 0）はリリース用。OP EGは必ずレベル0へ着地させる
/// 必要がある（ボイス解放条件が「全4オペレーターがidle」のため。`ui_core::TimeEgProfile`参照）。
/// 押している間は段0で静止するのでサステイン中の出力は1段時代と変わらない。
pub(crate) fn instant_sustain_eg() -> TimeEgParams {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
    TimeEgParams {
        stages,
        stage_count: 2,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 0,
     ..Default::default()}
}

/// TimeEg 7本（OP1〜4 EG／Pitch FG／Cutoff FG／Gain FG）の束。1本＝10段×3(time/level/curve)+
/// メタ10(stage_count/loop_enabled/loop_start/release_point/sync_enabled/sync_rate/
/// retrigger_mode/level_drift/depth_drift/texture)=40値、7本で280値
/// （8段時代は28値×7本=196値だった。段数拡張後もメタ側の実数はここに明記しないと
/// 古いコメントのまま取り残されるため、フィールド名を列挙してある）。
/// `Op505Patch`の全269値のうち大半を占めるが、DAWパラメーターにはせずnice-plugの
/// `#[persist]`でプロジェクト状態として保存する（理由: TimeEgHandleは「EG1本を丸ごと
/// 読み書き」するAPIのため、DAWパラメーター化するとグラフの点を1つ動かすたび29個の
/// オートメーションイベントが走り記録単位が壊れる。詳細はplan参照）。
/// **DAWパラメーター数（75個）はこの束が`#[persist]`である限り不変**——段数拡張はここに
/// 収まる値の中身が増えるだけで、DAWから見えるパラメーター一覧には影響しない。
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Op505EgBank {
    pub operators: [TimeEgParams; 4],
    pub pitch_fg: TimeEgParams,
    pub cutoff_fg: TimeEgParams,
    pub gain_fg: TimeEgParams,
}

impl Default for Op505EgBank {
    fn default() -> Self {
        let eg = instant_sustain_eg();
        // Pitch/Cutoff FGはレベルをバイポーラ解釈する（生値128＝無変調の中心）ため、
        // 振幅系の`instant_sustain_eg()`（段0 level=255）を流用すると「全開プラスへ張り付いた」
        // 初期状態になってしまう。無変調の中央EGを使う。
        let neutral_fg = op505_core::neutral_bipolar_eg();
        Self {
            operators: [eg; 4],
            pitch_fg: neutral_fg,
            cutoff_fg: neutral_fg,
            gain_fg: eg,
        }
    }
}

/// オペレーター単位のDAWパラメーター一式（11個）。EG本体は`Op505EgBank`（persist）側が持つ。
/// min/max/defaultは`gesture-app/editor-wasm/src/op505_state.rs`の
/// `operator_panel_params()`（`op!`マクロ呼び出し）を正本として写した。
#[derive(Params)]
pub(crate) struct OperatorVstParams {
    #[id = "tl"]
    pub tl: IntParam,
    #[id = "mul"]
    pub mul: IntParam,
    #[id = "dt1"]
    pub dt1: IntParam,
    #[id = "ksr"]
    pub ksr: IntParam,
    #[id = "ame"]
    pub ame: BoolParam,
    #[id = "vel_sens"]
    pub vel_sens: IntParam,
    #[id = "op_fine"]
    pub op_fine_tune: IntParam,
    #[id = "wf"]
    pub waveform: IntParam,
    #[id = "op_eg_shift"]
    pub eg_shift: IntParam,
    #[id = "op_level_scale"]
    pub level_scale: IntParam,
    #[id = "op_vel_gain"]
    pub velocity_gain: IntParam,
}

impl Default for OperatorVstParams {
    /// 「鳴る」状態を初期値とする（`ym38x6-vst`の`OperatorVstParams::default()`と同じ配慮。
    /// `Op505OperatorParams::default()`はtl=0で無音のため個別に明示値を設定する）。
    fn default() -> Self {
        Self {
            tl: IntParam::new("TL", 200, IntRange::Linear { min: 0, max: 255 }),
            mul: IntParam::new("MUL", 1, IntRange::Linear { min: 0, max: 15 }),
            dt1: bipolar_int(IntParam::new("DT1", 128, IntRange::Linear { min: 0, max: 255 })),
            ksr: IntParam::new("KSR", 64, IntRange::Linear { min: 0, max: 255 }),
            ame: BoolParam::new("AM Enable", false),
            vel_sens: IntParam::new("Velocity Sensitivity", 0, IntRange::Linear { min: 0, max: 255 }),
            op_fine_tune: bipolar_int(IntParam::new("Op Fine Tune", 128, IntRange::Linear { min: 0, max: 255 })),
            waveform: IntParam::new("Waveform", 0, IntRange::Linear { min: 0, max: 255 }),
            eg_shift: IntParam::new("Op EG Shift", 0, IntRange::Linear { min: 0, max: 255 }),
            level_scale: IntParam::new("Op Level Scale", 0, IntRange::Linear { min: 0, max: 255 }),
            velocity_gain: IntParam::new("Op Velocity Gain", 255, IntRange::Linear { min: 0, max: 255 }),
        }
    }
}

#[derive(Params)]
pub(crate) struct Op505VstParams {
    // ---- チャンネル単位 ----
    #[id = "algorithm"]
    pub algorithm: IntParam,
    #[id = "feedback"]
    pub feedback: IntParam,
    #[id = "cutoff"]
    pub cutoff: IntParam,
    #[id = "resonance"]
    pub resonance: IntParam,
    #[id = "filter_type"]
    pub filter_type: IntParam,
    #[id = "filter_self_osc"]
    pub filter_self_oscillation: BoolParam,

    // ---- FG Depth（EG本体はOp505EgBank側） ----
    // 符号を持たない振れ幅の倍率（0＝変調なし）。符号はEGのレベル波形側が持つため
    // `bipolar_int`は使わない。
    #[id = "pitch_fg_depth"]
    pub pitch_fg_depth: IntParam,
    #[id = "cutoff_fg_depth"]
    pub cutoff_fg_depth: IntParam,

    // ---- Gain FGの行先スイッチ（Depthなし、bool 2個。旧CHIP LFO AM経路の厳密代替。
    //      memory `project_chip_lfo_retirement_investigation.md`参照） ----
    #[id = "gain_fg_to_master"]
    pub gain_fg_to_master: BoolParam,
    #[id = "gain_fg_to_operators"]
    pub gain_fg_to_operators: BoolParam,

    // ---- オペレーター単位（11個 × 4op = 44個） ----
    #[nested(array, group = "Operator")]
    pub operators: [OperatorVstParams; 4],

    // ---- マスター単位 ----
    #[id = "rev_send"]
    pub rev_send: IntParam,
    #[id = "cho_send"]
    pub cho_send: IntParam,
    #[id = "rev_type"]
    pub reverb_type: IntParam,
    #[id = "rev_time"]
    pub reverb_time: IntParam,
    #[id = "cho_type"]
    pub chorus_type: IntParam,
    #[id = "cho_rate"]
    pub chorus_mod_rate: IntParam,
    #[id = "cho_depth"]
    pub chorus_mod_depth: IntParam,
    #[id = "cho_fb"]
    pub chorus_feedback: IntParam,
    #[id = "cho_to_rev"]
    pub chorus_send_to_reverb: IntParam,

    // ---- TimeEg 7本（persist状態、DAWパラメーターではない。plan参照） ----
    #[persist = "op505_egs"]
    pub egs: Arc<RwLock<Op505EgBank>>,
}

impl Default for Op505VstParams {
    fn default() -> Self {
        Self {
            algorithm: IntParam::new("Algorithm", DEFAULT_ALGORITHM as i32, IntRange::Linear { min: 0, max: 7 }),
            feedback: IntParam::new("Feedback", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff: IntParam::new("Filter Cutoff", 255, IntRange::Linear { min: 0, max: 255 }),
            resonance: IntParam::new("Filter Resonance", 0, IntRange::Linear { min: 0, max: 255 }),
            filter_type: IntParam::new("Filter Type", 0, IntRange::Linear { min: 0, max: 255 }),
            filter_self_oscillation: BoolParam::new("Filter Self-Oscillation", true),
            pitch_fg_depth: IntParam::new("Pitch FG Depth", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_depth: IntParam::new("Cutoff FG Depth", 0, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_to_master: BoolParam::new("Gain FG to Master", true),
            gain_fg_to_operators: BoolParam::new("Gain FG to Operators", false),
            operators: Default::default(),
            rev_send: IntParam::new("Reverb Send", 0, IntRange::Linear { min: 0, max: 255 }),
            cho_send: IntParam::new("Chorus Send", 0, IntRange::Linear { min: 0, max: 255 }),
            reverb_type: IntParam::new("Reverb Type", DEFAULT_REVERB_TYPE as i32, IntRange::Linear { min: 0, max: 7 }),
            reverb_time: IntParam::new("Reverb Time", DEFAULT_REVERB_TIME as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_type: IntParam::new("Chorus Type", DEFAULT_CHORUS_TYPE as i32, IntRange::Linear { min: 0, max: 7 }),
            chorus_mod_rate: IntParam::new("Chorus Mod Rate", DEFAULT_CHORUS_MOD_RATE as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_mod_depth: IntParam::new("Chorus Mod Depth", DEFAULT_CHORUS_MOD_DEPTH as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_feedback: IntParam::new("Chorus Feedback", DEFAULT_CHORUS_FEEDBACK as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_send_to_reverb: IntParam::new("Chorus Send To Reverb", DEFAULT_CHORUS_SEND_TO_REVERB as i32, IntRange::Linear { min: 0, max: 255 }),
            egs: Arc::new(RwLock::new(Op505EgBank::default())),
        }
    }
}
