use nice_plug::prelude::*;
use op505_core::{Op505BipolarFg, Op505ChannelParams, Op505GainFg, Op505OperatorParams, Op505Patch};
use op505_editor::param_spec::{BoolField, FgSlot, FxInt, IntField, IntSpec, OpIndex, OpInt, PatchInt};
use serde::{Deserialize, Serialize};
use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
use std::sync::{Arc, RwLock};

/// op505-editorの正本（`param_spec`）から導出する。`PatchInt`/`FxInt`の`spec()`は`const fn`のため
/// ここもconstのまま保てる（正本とリテラルが2箇所に分かれることを防ぐ）。
pub(crate) const DEFAULT_ALGORITHM: u8 = PatchInt::Algorithm.spec().default as u8;
pub(crate) const DEFAULT_REVERB_TIME: u8 = FxInt::ReverbTime.spec().default as u8;
pub(crate) const DEFAULT_CHORUS_MOD_RATE: u8 = FxInt::ChorusModRate.spec().default as u8;
pub(crate) const DEFAULT_CHORUS_MOD_DEPTH: u8 = FxInt::ChorusModDepth.spec().default as u8;
pub(crate) const DEFAULT_CHORUS_FEEDBACK: u8 = FxInt::ChorusFeedback.spec().default as u8;
pub(crate) const DEFAULT_CHORUS_SEND_TO_REVERB: u8 = FxInt::ChorusSendToReverb.spec().default as u8;
pub(crate) const DEFAULT_REVERB_TYPE: u8 = FxInt::ReverbType.spec().default as u8;
pub(crate) const DEFAULT_CHORUS_TYPE: u8 = FxInt::ChorusType.spec().default as u8;

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

/// `spec`からDAWの`IntParam`を組み立てる。`daw_bipolar`が立っていれば`bipolar_int`を適用する。
fn param_from_int_spec(spec: IntSpec) -> IntParam {
    let param = IntParam::new(spec.daw_name, spec.default, IntRange::Linear { min: spec.min, max: spec.max });
    if spec.daw_bipolar {
        bipolar_int(param)
    } else {
        param
    }
}

/// `field`の正本（op505-editor::param_spec）からDAWの`IntParam`を組み立てる。
fn int_param(field: IntField) -> IntParam {
    param_from_int_spec(field.spec())
}

/// オペレーター単位の`OpInt`から直接組み立てる。`OperatorVstParams::default()`は4オペレーター
/// 共通のため`OpIndex`を持たない（`OpInt::spec()`がオペレーター非依存であることは
/// op505-editor側の`op_field_spec_is_operator_independent`でテスト済み）。
fn op_int_param(op_int: OpInt) -> IntParam {
    param_from_int_spec(op_int.spec())
}

/// `field`の正本からDAWの`BoolParam`を組み立てる。
fn bool_param(field: BoolField) -> BoolParam {
    let spec = field.spec();
    BoolParam::new(spec.daw_name, spec.default)
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
/// **DAWパラメーター数（67個。内訳はop505-editor::param_spec::IntField/BoolFieldのenum件数
/// 59+8）はこの束が`#[persist]`である限り不変**——段数拡張はここに
/// 収まる値の中身が増えるだけで、DAWから見えるパラメーター一覧には影響しない
/// （従来コメントの「78個」は誤りだった。実数は`param_ids_are_frozen`テストで凍結済み）。
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
            // Gain FGはオペレーターEG用の`eg`（STAGES=2）ではなく専用の透過既定
            // （STAGES=0＝無効化、`Op505ChannelParams::default()`が使うものと同じ）を使う。
            // 以前は`eg`を誤って流用しており、新規Add直後のGain FGだけSTAGES=2/RELが立って
            // 見える不整合があった（ユーザー指摘、2026-08-28）。
            gain_fg: op505_core::default_gain_fg().eg,
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
            tl: op_int_param(OpInt::Tl),
            mul: op_int_param(OpInt::Mul),
            dt1: op_int_param(OpInt::Dt1),
            ksr: op_int_param(OpInt::Ksr),
            // AM Enableの既定はオペレーター非依存（BoolField::Ame(_)のspec()参照）。
            ame: bool_param(BoolField::Ame(OpIndex::Op1)),
            vel_sens: op_int_param(OpInt::VelSens),
            op_fine_tune: op_int_param(OpInt::OpFineTune),
            waveform: op_int_param(OpInt::Waveform),
            eg_shift: op_int_param(OpInt::EgShift),
            level_scale: op_int_param(OpInt::LevelScale),
            velocity_gain: op_int_param(OpInt::VelocityGain),
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
    /// Gain FGだけ既定255（旧仕様と完全一致するための「EGの形をそのまま使う」既定値）。
    /// Pitch/Cutoff FGの既定0（無変調）とは意味が異なる点に注意（`Op505GainFg`のdocコメント参照）。
    #[id = "gain_fg_depth"]
    pub gain_fg_depth: IntParam,

    // ---- Gain FGの行先スイッチ（Depthなし、bool 2個。旧CHIP LFO AM経路の厳密代替。
    //      memory `project_chip_lfo_retirement_investigation.md`参照） ----
    #[id = "gain_fg_to_master"]
    pub gain_fg_to_master: BoolParam,
    #[id = "gain_fg_to_operators"]
    pub gain_fg_to_operators: BoolParam,

    // ---- 固定音階（GM2リズムチャンネル用。ノートオン周波数を無視して固定ピッチで鳴らす。
    //      memory `project_gm2_rhythm_channel_implementation.md`参照） ----
    #[id = "fixed_note_enable"]
    pub fixed_note_enable: BoolParam,
    #[id = "fixed_note"]
    pub fixed_note: IntParam,
    #[id = "fixed_note_fine"]
    pub fixed_note_fine: IntParam,

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
            algorithm: int_param(IntField::Patch(PatchInt::Algorithm)),
            feedback: int_param(IntField::Patch(PatchInt::Feedback)),
            cutoff: int_param(IntField::Patch(PatchInt::Cutoff)),
            resonance: int_param(IntField::Patch(PatchInt::Resonance)),
            filter_type: int_param(IntField::Patch(PatchInt::FilterType)),
            filter_self_oscillation: bool_param(BoolField::FilterSelfOscillation),
            pitch_fg_depth: int_param(IntField::Patch(PatchInt::FgDepth(FgSlot::Pitch))),
            cutoff_fg_depth: int_param(IntField::Patch(PatchInt::FgDepth(FgSlot::Cutoff))),
            gain_fg_depth: int_param(IntField::Patch(PatchInt::FgDepth(FgSlot::Gain))),
            gain_fg_to_master: bool_param(BoolField::GainFgToMaster),
            gain_fg_to_operators: bool_param(BoolField::GainFgToOperators),
            fixed_note_enable: bool_param(BoolField::FixedNoteEnable),
            fixed_note: int_param(IntField::Patch(PatchInt::FixedNote)),
            fixed_note_fine: int_param(IntField::Patch(PatchInt::FixedNoteFine)),
            operators: Default::default(),
            rev_send: int_param(IntField::Fx(FxInt::RevSend)),
            cho_send: int_param(IntField::Fx(FxInt::ChoSend)),
            reverb_type: int_param(IntField::Fx(FxInt::ReverbType)),
            reverb_time: int_param(IntField::Fx(FxInt::ReverbTime)),
            chorus_type: int_param(IntField::Fx(FxInt::ChorusType)),
            chorus_mod_rate: int_param(IntField::Fx(FxInt::ChorusModRate)),
            chorus_mod_depth: int_param(IntField::Fx(FxInt::ChorusModDepth)),
            chorus_feedback: int_param(IntField::Fx(FxInt::ChorusFeedback)),
            chorus_send_to_reverb: int_param(IntField::Fx(FxInt::ChorusSendToReverb)),
            egs: Arc::new(RwLock::new(Op505EgBank::default())),
        }
    }
}

/// 現在のDAWパラメーター＋TimeEg束から`Op505Patch`を構築する（MIDIチャンネル非依存）。
/// オーディオスレッドは`cached_egs`を、GUIスレッドは`params.egs.read()`の結果を渡す
/// （どちらも同じ関数を通ることが、保存される音と鳴っている音が一致することの根拠）。
/// NRPN(0,9)〜(0,15)由来の`overrides`やCC1/76/77/78のPitch FG演奏補正はここでは適用しない
/// （`apply_pitch_fg_expression`と同じ「note_patchへの後処理」パターンで別途適用するため）。
pub(crate) fn build_patch(p: &Op505VstParams, egs: &Op505EgBank) -> Op505Patch {
    let operators = std::array::from_fn(|i| {
        let op = &p.operators[i];
        Op505OperatorParams {
            tl: op.tl.value() as u8,
            eg: egs.operators[i],
            mul: op.mul.value() as u8,
            dt1: op.dt1.value() as u8,
            ksr: op.ksr.value() as u8,
            am_enable: op.ame.value(),
            velocity_sensitivity: op.vel_sens.value() as u8,
            waveform: op.waveform.value() as u8,
            op_fine_tune: op.op_fine_tune.value() as u8,
            eg_shift: op.eg_shift.value() as u8,
            level_scale: op.level_scale.value() as u8,
            velocity_gain: op.velocity_gain.value() as u8,
        }
    });

    let channel = Op505ChannelParams {
        algorithm: p.algorithm.value() as u8,
        feedback: p.feedback.value() as u8,
        filter_cutoff: p.cutoff.value() as u8,
        filter_resonance: p.resonance.value() as u8,
        filter_type: p.filter_type.value() as u8,
        filter_self_oscillation: p.filter_self_oscillation.value(),
        pitch_fg: Op505BipolarFg { eg: egs.pitch_fg, depth: p.pitch_fg_depth.value() as u8 },
        cutoff_fg: Op505BipolarFg { eg: egs.cutoff_fg, depth: p.cutoff_fg_depth.value() as u8 },
        gain_fg: Op505GainFg { eg: egs.gain_fg, depth: p.gain_fg_depth.value() as u8 },
        gain_fg_to_master: p.gain_fg_to_master.value(),
        gain_fg_to_operators: p.gain_fg_to_operators.value(),
        fixed_note_enable: p.fixed_note_enable.value(),
        fixed_note: p.fixed_note.value() as u8,
        fixed_note_fine: p.fixed_note_fine.value() as u8,
        ..Op505ChannelParams::default()
    };

    Op505Patch { operators, channel }
}

/// `build_patch`の逆写像：`patch`のDAWパラメーター部分（TimeEg以外）を`setter`経由で書き込む。
/// PRESETSリストのクリックのように「音色を丸ごと選び直す」操作の共通処理（`apply_patch_egs`と
/// セットで呼ぶ）。
pub(crate) fn apply_patch(p: &Op505VstParams, setter: &ParamSetter<'_>, patch: &Op505Patch) {
    macro_rules! set {
        ($param:expr, $v:expr) => {
            setter.begin_set_parameter(&$param);
            setter.set_parameter(&$param, $v);
            setter.end_set_parameter(&$param);
        };
    }
    let ch = &patch.channel;
    set!(p.algorithm, ch.algorithm as i32);
    set!(p.feedback, ch.feedback as i32);
    set!(p.cutoff, ch.filter_cutoff as i32);
    set!(p.resonance, ch.filter_resonance as i32);
    set!(p.filter_type, ch.filter_type as i32);
    set!(p.filter_self_oscillation, ch.filter_self_oscillation);
    set!(p.pitch_fg_depth, ch.pitch_fg.depth as i32);
    set!(p.cutoff_fg_depth, ch.cutoff_fg.depth as i32);
    set!(p.gain_fg_depth, ch.gain_fg.depth as i32);
    set!(p.gain_fg_to_master, ch.gain_fg_to_master);
    set!(p.gain_fg_to_operators, ch.gain_fg_to_operators);
    set!(p.fixed_note_enable, ch.fixed_note_enable);
    set!(p.fixed_note, ch.fixed_note as i32);
    set!(p.fixed_note_fine, ch.fixed_note_fine as i32);
    for (i, op) in patch.operators.iter().enumerate() {
        let op_p = &p.operators[i];
        set!(op_p.tl, op.tl as i32);
        set!(op_p.mul, op.mul as i32);
        set!(op_p.dt1, op.dt1 as i32);
        set!(op_p.ksr, op.ksr as i32);
        set!(op_p.ame, op.am_enable);
        set!(op_p.vel_sens, op.velocity_sensitivity as i32);
        set!(op_p.op_fine_tune, op.op_fine_tune as i32);
        set!(op_p.waveform, op.waveform as i32);
        set!(op_p.eg_shift, op.eg_shift as i32);
        set!(op_p.level_scale, op.level_scale as i32);
        set!(op_p.velocity_gain, op.velocity_gain as i32);
    }
}

/// `build_patch`の逆写像（TimeEg 7本側）：`patch`のTimeEgを`egs`へ直接書き込む。persist状態の
/// ため`ParamSetter`を経由しない、純粋関数（`apply_patch`とセットで呼ぶ）。
pub(crate) fn apply_patch_egs(egs: &mut Op505EgBank, patch: &Op505Patch) {
    for (i, op) in patch.operators.iter().enumerate() {
        egs.operators[i] = op.eg;
    }
    egs.pitch_fg = patch.channel.pitch_fg.eg;
    egs.cutoff_fg = patch.channel.cutoff_fg.eg;
    egs.gain_fg = patch.channel.gain_fg.eg;
}

/// `field`に対応する`Op505VstParams`側の`IntParam`を返す。`_ =>`を使わない全列挙。
/// `param_adapter::VstPanelSource`（パネル描画）と`#[cfg(test)]`側の一致検証テストの両方が使う。
pub(crate) fn int_param_ref(params: &Op505VstParams, field: IntField) -> &IntParam {
    match field {
        IntField::Patch(PatchInt::Algorithm) => &params.algorithm,
        IntField::Patch(PatchInt::Feedback) => &params.feedback,
        IntField::Patch(PatchInt::FixedNote) => &params.fixed_note,
        IntField::Patch(PatchInt::FixedNoteFine) => &params.fixed_note_fine,
        IntField::Patch(PatchInt::Cutoff) => &params.cutoff,
        IntField::Patch(PatchInt::Resonance) => &params.resonance,
        IntField::Patch(PatchInt::FilterType) => &params.filter_type,
        IntField::Patch(PatchInt::FgDepth(FgSlot::Pitch)) => &params.pitch_fg_depth,
        IntField::Patch(PatchInt::FgDepth(FgSlot::Cutoff)) => &params.cutoff_fg_depth,
        IntField::Patch(PatchInt::FgDepth(FgSlot::Gain)) => &params.gain_fg_depth,
        IntField::Patch(PatchInt::Op(op, op_int)) => {
            let o = &params.operators[op.index()];
            match op_int {
                OpInt::Tl => &o.tl,
                OpInt::Mul => &o.mul,
                OpInt::Dt1 => &o.dt1,
                OpInt::Ksr => &o.ksr,
                OpInt::VelSens => &o.vel_sens,
                OpInt::OpFineTune => &o.op_fine_tune,
                OpInt::Waveform => &o.waveform,
                OpInt::EgShift => &o.eg_shift,
                OpInt::LevelScale => &o.level_scale,
                OpInt::VelocityGain => &o.velocity_gain,
            }
        }
        IntField::Fx(fx) => match fx {
            FxInt::RevSend => &params.rev_send,
            FxInt::ReverbType => &params.reverb_type,
            FxInt::ReverbTime => &params.reverb_time,
            FxInt::ChoSend => &params.cho_send,
            FxInt::ChorusType => &params.chorus_type,
            FxInt::ChorusModRate => &params.chorus_mod_rate,
            FxInt::ChorusModDepth => &params.chorus_mod_depth,
            FxInt::ChorusFeedback => &params.chorus_feedback,
            FxInt::ChorusSendToReverb => &params.chorus_send_to_reverb,
        },
    }
}

/// `field`に対応する`Op505VstParams`側の`BoolParam`を返す。`_ =>`を使わない全列挙。
pub(crate) fn bool_param_ref(params: &Op505VstParams, field: BoolField) -> &BoolParam {
    match field {
        BoolField::FixedNoteEnable => &params.fixed_note_enable,
        BoolField::FilterSelfOscillation => &params.filter_self_oscillation,
        BoolField::GainFgToMaster => &params.gain_fg_to_master,
        BoolField::GainFgToOperators => &params.gain_fg_to_operators,
        BoolField::Ame(op) => &params.operators[op.index()].ame,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// op505-editorの正本（`param_spec`）とDAWパラメーターのname/range/defaultが1個も
    /// 食い違わないことを検証する。「片方だけ変えても検出できない」問題の構造的解決
    /// （計画`fancy-wishing-toast.md`のStep 2）。
    #[test]
    fn vst_int_params_match_spec() {
        let params = Op505VstParams::default();
        for field in IntField::all() {
            let spec = field.spec();
            let param = int_param_ref(&params, field);
            let IntRange::Linear { min, max } = param.range() else {
                panic!("{field:?} の range が Linear ではない");
            };
            assert_eq!(param.name(), spec.daw_name, "{field:?} の name が正本と不一致");
            assert_eq!(min, spec.min, "{field:?} の min が正本と不一致");
            assert_eq!(max, spec.max, "{field:?} の max が正本と不一致");
            assert_eq!(param.default_plain_value(), spec.default, "{field:?} の default が正本と不一致");
        }
    }

    #[test]
    fn vst_bool_params_match_spec() {
        let params = Op505VstParams::default();
        for field in BoolField::ALL {
            let spec = field.spec();
            let param = bool_param_ref(&params, field);
            assert_eq!(param.name(), spec.daw_name, "{field:?} の name が正本と不一致");
            assert_eq!(param.default_plain_value(), spec.default, "{field:?} の default が正本と不一致");
        }
    }

    /// DAWパラメーターIDの集合を凍結する。ここが変わると既存DAWプロジェクトの
    /// オートメーション対応が壊れる（`#[id]`は絶対に変更しない、というリスク①の防御層）。
    /// オペレーター単位のIDは`#[nested(array, ...)]`により`{id}_{1..=4}`へ展開される。
    #[test]
    fn param_ids_are_frozen() {
        let params = Op505VstParams::default();
        let mut ids: Vec<String> = params.param_map().into_iter().map(|(id, _, _)| id).collect();
        ids.sort();

        let mut expected: Vec<String> = [
            "algorithm",
            "feedback",
            "cutoff",
            "resonance",
            "filter_type",
            "filter_self_osc",
            "pitch_fg_depth",
            "cutoff_fg_depth",
            "gain_fg_depth",
            "gain_fg_to_master",
            "gain_fg_to_operators",
            "fixed_note_enable",
            "fixed_note",
            "fixed_note_fine",
            "rev_send",
            "cho_send",
            "rev_type",
            "rev_time",
            "cho_type",
            "cho_rate",
            "cho_depth",
            "cho_fb",
            "cho_to_rev",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        for idx in 1..=4 {
            for base in ["tl", "mul", "dt1", "ksr", "ame", "vel_sens", "op_fine", "wf", "op_eg_shift", "op_level_scale", "op_vel_gain"] {
                expected.push(format!("{base}_{idx}"));
            }
        }
        expected.sort();

        assert_eq!(ids, expected, "DAWパラメーターID集合が変化した（#[id]属性の変更は禁止）");
    }

    #[test]
    fn build_patch_reflects_daw_params_and_egs() {
        // IntParamの値は`ParamSetter`（実プラグイン文脈が要る）でしか書き換えられないため、
        // 構築時のdefault値そのものを検証対象にする（`IntParam::new`の第2引数=初期値=`.value()`）。
        let params = Op505VstParams {
            algorithm: IntParam::new("Algorithm", 4, IntRange::Linear { min: 0, max: 7 }),
            fixed_note: IntParam::new("Fixed Note", 72, IntRange::Linear { min: 0, max: 127 }),
            ..Op505VstParams::default()
        };
        let mut egs = Op505EgBank::default();
        egs.operators[0].stage_count = 3;

        let patch = build_patch(&params, &egs);

        assert_eq!(patch.channel.algorithm, 4);
        assert_eq!(patch.channel.fixed_note, 72);
        assert_eq!(patch.operators[0].eg, egs.operators[0], "EGはegs引数からそのままコピーされるはず");
    }

    #[test]
    fn apply_patch_egs_copies_all_seven_egs() {
        let mut patch = Op505Patch::default();
        for (i, op) in patch.operators.iter_mut().enumerate() {
            op.eg.stage_count = (i + 1) as u8;
        }
        patch.channel.pitch_fg.eg.stage_count = 5;
        patch.channel.cutoff_fg.eg.stage_count = 6;
        patch.channel.gain_fg.eg.stage_count = 7;

        let mut egs = Op505EgBank::default();
        apply_patch_egs(&mut egs, &patch);

        for (i, op) in patch.operators.iter().enumerate() {
            assert_eq!(egs.operators[i], op.eg, "オペレーターEG[{i}]が写っていないはず");
        }
        assert_eq!(egs.pitch_fg, patch.channel.pitch_fg.eg);
        assert_eq!(egs.cutoff_fg, patch.channel.cutoff_fg.eg);
        assert_eq!(egs.gain_fg, patch.channel.gain_fg.eg);
    }

    #[test]
    fn build_patch_after_apply_patch_egs_round_trips() {
        let mut patch = Op505Patch::default();
        patch.operators[2].eg.stage_count = 4;
        patch.channel.gain_fg.eg.stage_count = 9;

        let mut egs = Op505EgBank::default();
        apply_patch_egs(&mut egs, &patch);

        let params = Op505VstParams::default();
        let rebuilt = build_patch(&params, &egs);

        assert_eq!(rebuilt.operators[2].eg, patch.operators[2].eg);
        assert_eq!(rebuilt.channel.gain_fg, patch.channel.gain_fg);
    }
}
