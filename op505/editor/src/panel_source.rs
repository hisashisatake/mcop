//! パネル組み立て（`Op505PanelParams`の構築）をホスト非依存にする[`PanelParamSource`]と、
//! それを1回で網羅する[`build_panel_params`]。
//!
//! 戻り値を`Box<dyn … + '_>`（＝`&self`に縛る）にするのが要——`'static`にするとVSTが実装
//! 不能（`ParamSetter`借用ベースのため）、トレイト自体にライフタイム引数を付けると`&dyn`が
//! 扱いづらくなる。VSTは借用ベースで`'s`、standaloneは`Rc`のcloneで`'static`（`'static: 's`
//! なのでどちらも通る）。詳細設計は`.claude/plans/fancy-wishing-toast.md`「② PanelParamSource」参照。

use op505_ui::{BoolParamHandle, IntParamHandle, Op505BipolarFgPanelParams, Op505OperatorPanelParams, Op505PanelParams, TimeEgHandle};

use crate::param_spec::{BoolField, EgSlot, FgSlot, FxInt, IntField, OpIndex, OpInt, PatchInt};

/// ホスト差分（DAWパラメーター経由か`Rc<RefCell<Op505Patch>>`直接かなど）を吸収する3操作。
/// `IntField`/`BoolField`/`EgSlot`はop505-editorの正本（`param_spec`）が全列挙するため、
/// 実装側は`_ =>`を使わず全パターンを網羅すること。
pub trait PanelParamSource {
    fn int(&self, field: IntField) -> Box<dyn IntParamHandle + '_>;
    fn boolean(&self, field: BoolField) -> Box<dyn BoolParamHandle + '_>;
    fn eg(&self, slot: EgSlot) -> Box<dyn TimeEgHandle + '_>;
}

/// `src`から`Op505PanelParams`（`op505_ui::draw_op505_panel`への入力一式）を組み立てる。
pub fn build_panel_params<'a>(src: &'a dyn PanelParamSource) -> Op505PanelParams<'a> {
    Op505PanelParams {
        algorithm: src.int(IntField::Patch(PatchInt::Algorithm)),
        feedback: src.int(IntField::Patch(PatchInt::Feedback)),
        fixed_note_enable: src.boolean(BoolField::FixedNoteEnable),
        fixed_note: src.int(IntField::Patch(PatchInt::FixedNote)),
        fixed_note_fine: src.int(IntField::Patch(PatchInt::FixedNoteFine)),
        cutoff: src.int(IntField::Patch(PatchInt::Cutoff)),
        resonance: src.int(IntField::Patch(PatchInt::Resonance)),
        filter_type: src.int(IntField::Patch(PatchInt::FilterType)),
        filter_self_oscillation: src.boolean(BoolField::FilterSelfOscillation),
        pitch_fg: bipolar_fg_panel_params(src, FgSlot::Pitch),
        cutoff_fg: bipolar_fg_panel_params(src, FgSlot::Cutoff),
        gain_fg: bipolar_fg_panel_params(src, FgSlot::Gain),
        gain_fg_to_master: src.boolean(BoolField::GainFgToMaster),
        gain_fg_to_operators: src.boolean(BoolField::GainFgToOperators),
        rev_send: src.int(IntField::Fx(FxInt::RevSend)),
        reverb_type: src.int(IntField::Fx(FxInt::ReverbType)),
        reverb_time: src.int(IntField::Fx(FxInt::ReverbTime)),
        cho_send: src.int(IntField::Fx(FxInt::ChoSend)),
        chorus_type: src.int(IntField::Fx(FxInt::ChorusType)),
        chorus_mod_rate: src.int(IntField::Fx(FxInt::ChorusModRate)),
        chorus_mod_depth: src.int(IntField::Fx(FxInt::ChorusModDepth)),
        chorus_feedback: src.int(IntField::Fx(FxInt::ChorusFeedback)),
        chorus_send_to_reverb: src.int(IntField::Fx(FxInt::ChorusSendToReverb)),
        operators: OpIndex::ALL.map(|op| operator_panel_params(src, op)),
    }
}

fn bipolar_fg_panel_params<'a>(src: &'a dyn PanelParamSource, fg: FgSlot) -> Op505BipolarFgPanelParams<'a> {
    Op505BipolarFgPanelParams { eg: src.eg(EgSlot::Fg(fg)), depth: src.int(IntField::Patch(PatchInt::FgDepth(fg))) }
}

fn operator_panel_params<'a>(src: &'a dyn PanelParamSource, op: OpIndex) -> Op505OperatorPanelParams<'a> {
    Op505OperatorPanelParams {
        tl: src.int(IntField::Patch(PatchInt::Op(op, OpInt::Tl))),
        eg: src.eg(EgSlot::Op(op)),
        mul: src.int(IntField::Patch(PatchInt::Op(op, OpInt::Mul))),
        dt1: src.int(IntField::Patch(PatchInt::Op(op, OpInt::Dt1))),
        ksr: src.int(IntField::Patch(PatchInt::Op(op, OpInt::Ksr))),
        vel_sens: src.int(IntField::Patch(PatchInt::Op(op, OpInt::VelSens))),
        op_fine_tune: src.int(IntField::Patch(PatchInt::Op(op, OpInt::OpFineTune))),
        ame: src.boolean(BoolField::Ame(op)),
        waveform: src.int(IntField::Patch(PatchInt::Op(op, OpInt::Waveform))),
        eg_shift: src.int(IntField::Patch(PatchInt::Op(op, OpInt::EgShift))),
        level_scale: src.int(IntField::Patch(PatchInt::Op(op, OpInt::LevelScale))),
        velocity_gain: src.int(IntField::Patch(PatchInt::Op(op, OpInt::VelocityGain))),
    }
}
