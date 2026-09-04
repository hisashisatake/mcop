//! [`PanelParamSource`]の`Rc<RefCell<Op505Patch>>`ベース実装（[`PatchPanelSource`]）と、
//! MASTER EFFECTS（Reverb/Chorus）用のホスト非依存ミラー状態（[`MasterEffectsState`]）。
//!
//! op505-standaloneが使う。op505-vstはDAWパラメーター／`ParamSetter`ベースのため、
//! 同じ`PanelParamSource`トレイトの別実装（`VstPanelSource`）を`op505-vst::param_adapter`に
//! 持つ（VSTは`nice_plug::ParamSetter`という借用ベースの型を経由する必要があり、
//! `Rc<RefCell<..>>`では表現できないため。詳細はplan「② PanelParamSource」参照）。
//!
//! フィールドごとにget/set関数ポインタを持つ旧`op!`/`ch!`マクロ展開方式
//! （74個のフィールド分の記述）ではなく、`PatchInt`/`OpInt`/`BoolField`/`EgSlot`という
//! 1個の値から`read_int`/`write_int`等の網羅matchで読み書きする方式にする
//! （オペレーターは`p.operators[i.index()]`の1アームで済むため大幅に短くなる）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use op505_core::{Op505OperatorParams, Op505Patch};
use op505_ui::{BoolParamHandle, IntParamHandle, TimeEgHandle};
use sound_core::TimeEgParams;

use crate::panel_source::PanelParamSource;
use crate::param_spec::{BoolField, EgSlot, FgSlot, FxInt, IntField, OpInt, PatchInt};
use crate::undo::UndoStack;

/// MASTER EFFECTS（Reverb/Chorus）パネル用の状態。`Op505Patch`の外側にあるエンジン非依存の
/// ミラーのため分離して保持する（`op505-vst`のDAWパラメーターに相当するホスト側ミラー）。
/// `PartialEq`はUndoスタック（`crate::undo::EditorSnapshot`）が操作前後の比較に使う。
#[derive(Clone, Copy, PartialEq)]
pub struct MasterEffectsState {
    pub rev_send: i32,
    pub reverb_type: i32,
    pub reverb_time: i32,
    pub cho_send: i32,
    pub chorus_type: i32,
    pub chorus_mod_rate: i32,
    pub chorus_mod_depth: i32,
    pub chorus_feedback: i32,
    pub chorus_send_to_reverb: i32,
    pub master_volume: i32,
}

impl Default for MasterEffectsState {
    /// 各既定値はop505-editorの正本（`FxInt::spec().default`）と一致する
    /// （op505-vstの`Op505VstParams::default()`と食い違うと無音デバッグ地獄になるため）。
    fn default() -> Self {
        Self {
            rev_send: FxInt::RevSend.spec().default,
            reverb_type: FxInt::ReverbType.spec().default,
            reverb_time: FxInt::ReverbTime.spec().default,
            cho_send: FxInt::ChoSend.spec().default,
            chorus_type: FxInt::ChorusType.spec().default,
            chorus_mod_rate: FxInt::ChorusModRate.spec().default,
            chorus_mod_depth: FxInt::ChorusModDepth.spec().default,
            chorus_feedback: FxInt::ChorusFeedback.spec().default,
            chorus_send_to_reverb: FxInt::ChorusSendToReverb.spec().default,
            master_volume: FxInt::MasterVolume.spec().default,
        }
    }
}

impl MasterEffectsState {
    /// `fx`に対応する現在値を読む。VST側のUndo/Redo差分適用でも使うため`pub`。
    pub fn field(self, fx: FxInt) -> i32 {
        match fx {
            FxInt::RevSend => self.rev_send,
            FxInt::ReverbType => self.reverb_type,
            FxInt::ReverbTime => self.reverb_time,
            FxInt::ChoSend => self.cho_send,
            FxInt::ChorusType => self.chorus_type,
            FxInt::ChorusModRate => self.chorus_mod_rate,
            FxInt::ChorusModDepth => self.chorus_mod_depth,
            FxInt::ChorusFeedback => self.chorus_feedback,
            FxInt::ChorusSendToReverb => self.chorus_send_to_reverb,
            FxInt::MasterVolume => self.master_volume,
        }
    }

    fn set_field(&mut self, fx: FxInt, value: i32) {
        match fx {
            FxInt::RevSend => self.rev_send = value,
            FxInt::ReverbType => self.reverb_type = value,
            FxInt::ReverbTime => self.reverb_time = value,
            FxInt::ChoSend => self.cho_send = value,
            FxInt::ChorusType => self.chorus_type = value,
            FxInt::ChorusModRate => self.chorus_mod_rate = value,
            FxInt::ChorusModDepth => self.chorus_mod_depth = value,
            FxInt::ChorusFeedback => self.chorus_feedback = value,
            FxInt::ChorusSendToReverb => self.chorus_send_to_reverb = value,
            FxInt::MasterVolume => self.master_volume = value,
        }
    }

    /// `FxInt::ALL`と同じ並び順の10値配列。ホスト側（standaloneの`SharedEditState::publish_fx`）は
    /// この順序をそのまま送る——呼び出し側で並びを凍結するテストを持つこと（ずれるとreverb_typeと
    /// reverb_timeが入れ替わって無音デバッグ地獄になる）。
    pub fn values_in_fx_order(&self) -> [u8; 10] {
        FxInt::ALL.map(|fx| self.field(fx) as u8)
    }

    /// `FxInt`ごとの読み取り関数から組み立てる。VST側がDAWパラメーター（`IntParam`）から
    /// `MasterEffectsState`を構築するために使う（standaloneは`Rc<RefCell<MasterEffectsState>>`を
    /// 直接持つため使わない）。
    pub fn from_fn(mut read: impl FnMut(FxInt) -> i32) -> Self {
        let mut state = Self::default();
        for fx in FxInt::ALL {
            state.set_field(fx, read(fx));
        }
        state
    }
}

/// `field`に対応する`Op505Patch`側の現在値を読む。VST側のUndo/Redo差分適用（`diff_to`が返した
/// フィールドの目標値を読む）でも使うため`pub`（`op505-editor`はop505-vst/standalone共有クレート）。
pub fn read_int(p: &Op505Patch, field: PatchInt) -> i32 {
    match field {
        PatchInt::Algorithm => p.channel.algorithm as i32,
        PatchInt::Feedback => p.channel.feedback as i32,
        PatchInt::FixedNote => p.channel.fixed_note as i32,
        PatchInt::FixedNoteFine => p.channel.fixed_note_fine as i32,
        PatchInt::Cutoff => p.channel.filter_cutoff as i32,
        PatchInt::Resonance => p.channel.filter_resonance as i32,
        PatchInt::FilterType => p.channel.filter_type as i32,
        PatchInt::FgDepth(FgSlot::Pitch) => p.channel.pitch_fg.depth as i32,
        PatchInt::FgDepth(FgSlot::Cutoff) => p.channel.cutoff_fg.depth as i32,
        PatchInt::FgDepth(FgSlot::Gain) => p.channel.gain_fg.depth as i32,
        PatchInt::Op(op, op_int) => read_op_int(&p.operators[op.index()], op_int),
    }
}

fn write_int(p: &mut Op505Patch, field: PatchInt, value: i32) {
    match field {
        PatchInt::Algorithm => p.channel.algorithm = value as u8,
        PatchInt::Feedback => p.channel.feedback = value as u8,
        PatchInt::FixedNote => p.channel.fixed_note = value as u8,
        PatchInt::FixedNoteFine => p.channel.fixed_note_fine = value as u8,
        PatchInt::Cutoff => p.channel.filter_cutoff = value as u8,
        PatchInt::Resonance => p.channel.filter_resonance = value as u8,
        PatchInt::FilterType => p.channel.filter_type = value as u8,
        PatchInt::FgDepth(FgSlot::Pitch) => p.channel.pitch_fg.depth = value as u8,
        PatchInt::FgDepth(FgSlot::Cutoff) => p.channel.cutoff_fg.depth = value as u8,
        PatchInt::FgDepth(FgSlot::Gain) => p.channel.gain_fg.depth = value as u8,
        PatchInt::Op(op, op_int) => write_op_int(&mut p.operators[op.index()], op_int, value),
    }
}

fn read_op_int(op: &Op505OperatorParams, field: OpInt) -> i32 {
    match field {
        OpInt::Tl => op.tl as i32,
        OpInt::Mul => op.mul as i32,
        OpInt::Dt1 => op.dt1 as i32,
        OpInt::Ksr => op.ksr as i32,
        OpInt::VelSens => op.velocity_sensitivity as i32,
        OpInt::OpFineTune => op.op_fine_tune as i32,
        OpInt::Waveform => op.waveform as i32,
        OpInt::EgShift => op.eg_shift as i32,
        OpInt::LevelScale => op.level_scale as i32,
        OpInt::VelocityGain => op.velocity_gain as i32,
    }
}

fn write_op_int(op: &mut Op505OperatorParams, field: OpInt, value: i32) {
    match field {
        OpInt::Tl => op.tl = value as u8,
        OpInt::Mul => op.mul = value as u8,
        OpInt::Dt1 => op.dt1 = value as u8,
        OpInt::Ksr => op.ksr = value as u8,
        OpInt::VelSens => op.velocity_sensitivity = value as u8,
        OpInt::OpFineTune => op.op_fine_tune = value as u8,
        OpInt::Waveform => op.waveform = value as u8,
        OpInt::EgShift => op.eg_shift = value as u8,
        OpInt::LevelScale => op.level_scale = value as u8,
        OpInt::VelocityGain => op.velocity_gain = value as u8,
    }
}

/// [`read_int`]と同じ理由で`pub`。
pub fn read_bool(p: &Op505Patch, field: BoolField) -> bool {
    match field {
        BoolField::FixedNoteEnable => p.channel.fixed_note_enable,
        BoolField::FilterSelfOscillation => p.channel.filter_self_oscillation,
        BoolField::GainFgToMaster => p.channel.gain_fg_to_master,
        BoolField::GainFgToOperators => p.channel.gain_fg_to_operators,
        BoolField::Ame(op) => p.operators[op.index()].am_enable,
    }
}

fn write_bool(p: &mut Op505Patch, field: BoolField, value: bool) {
    match field {
        BoolField::FixedNoteEnable => p.channel.fixed_note_enable = value,
        BoolField::FilterSelfOscillation => p.channel.filter_self_oscillation = value,
        BoolField::GainFgToMaster => p.channel.gain_fg_to_master = value,
        BoolField::GainFgToOperators => p.channel.gain_fg_to_operators = value,
        BoolField::Ame(op) => p.operators[op.index()].am_enable = value,
    }
}

/// [`read_int`]と同じ理由で`pub`。
pub fn read_eg(p: &Op505Patch, slot: EgSlot) -> TimeEgParams {
    match slot {
        EgSlot::Op(op) => p.operators[op.index()].eg,
        EgSlot::Fg(FgSlot::Pitch) => p.channel.pitch_fg.eg,
        EgSlot::Fg(FgSlot::Cutoff) => p.channel.cutoff_fg.eg,
        EgSlot::Fg(FgSlot::Gain) => p.channel.gain_fg.eg,
    }
}

fn write_eg(p: &mut Op505Patch, slot: EgSlot, params: TimeEgParams) {
    match slot {
        EgSlot::Op(op) => p.operators[op.index()].eg = params,
        EgSlot::Fg(FgSlot::Pitch) => p.channel.pitch_fg.eg = params,
        EgSlot::Fg(FgSlot::Cutoff) => p.channel.cutoff_fg.eg = params,
        EgSlot::Fg(FgSlot::Gain) => p.channel.gain_fg.eg = params,
    }
}

struct PatchIntHandle {
    patch: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    undo: Rc<RefCell<UndoStack>>,
    field: PatchInt,
}

impl IntParamHandle for PatchIntHandle {
    fn value(&self) -> i32 {
        read_int(&self.patch.borrow(), self.field)
    }
    fn min(&self) -> i32 {
        self.field.spec().min
    }
    fn max(&self) -> i32 {
        self.field.spec().max
    }
    fn default(&self) -> i32 {
        self.field.spec().default
    }
    fn name(&self) -> String {
        self.field.spec().short_name.to_string()
    }
    fn begin_edit(&self) {
        self.undo.borrow_mut().note_begin_edit();
    }
    fn set(&self, value: i32) {
        let clamped = value.clamp(self.min(), self.max());
        write_int(&mut self.patch.borrow_mut(), self.field, clamped);
        self.dirty.set(true);
    }
    fn end_edit(&self) {
        self.undo.borrow_mut().note_end_edit();
    }
}

struct PatchBoolHandle {
    patch: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    undo: Rc<RefCell<UndoStack>>,
    field: BoolField,
}

impl BoolParamHandle for PatchBoolHandle {
    fn value(&self) -> bool {
        read_bool(&self.patch.borrow(), self.field)
    }
    fn begin_edit(&self) {
        self.undo.borrow_mut().note_begin_edit();
    }
    fn set(&self, value: bool) {
        write_bool(&mut self.patch.borrow_mut(), self.field, value);
        self.dirty.set(true);
    }
    fn end_edit(&self) {
        self.undo.borrow_mut().note_end_edit();
    }
}

struct PatchTimeEgHandle {
    patch: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    undo: Rc<RefCell<UndoStack>>,
    slot: EgSlot,
}

impl TimeEgHandle for PatchTimeEgHandle {
    fn params(&self) -> TimeEgParams {
        read_eg(&self.patch.borrow(), self.slot)
    }
    fn set_params(&self, params: TimeEgParams) {
        write_eg(&mut self.patch.borrow_mut(), self.slot, params);
        self.dirty.set(true);
    }
    fn name(&self) -> String {
        self.slot.name().to_string()
    }
    fn begin_edit(&self) {
        self.undo.borrow_mut().note_begin_edit();
    }
    fn end_edit(&self) {
        self.undo.borrow_mut().note_end_edit();
    }
}

struct MasterEffectsIntHandle {
    state: Rc<RefCell<MasterEffectsState>>,
    dirty: Rc<Cell<bool>>,
    undo: Rc<RefCell<UndoStack>>,
    fx: FxInt,
}

impl IntParamHandle for MasterEffectsIntHandle {
    fn value(&self) -> i32 {
        self.state.borrow().field(self.fx)
    }
    fn min(&self) -> i32 {
        self.fx.spec().min
    }
    fn max(&self) -> i32 {
        self.fx.spec().max
    }
    fn default(&self) -> i32 {
        self.fx.spec().default
    }
    fn name(&self) -> String {
        self.fx.spec().short_name.to_string()
    }
    fn begin_edit(&self) {
        self.undo.borrow_mut().note_begin_edit();
    }
    fn set(&self, value: i32) {
        let clamped = value.clamp(self.min(), self.max());
        self.state.borrow_mut().set_field(self.fx, clamped);
        self.dirty.set(true);
    }
    fn end_edit(&self) {
        self.undo.borrow_mut().note_end_edit();
    }
}

/// [`PanelParamSource`]の`Rc<RefCell<Op505Patch>>`ベース実装。`Rc`のcloneで各ハンドルを
/// 作るため戻り値は`'static`になり、`Box<dyn .. + '_>`（`PanelParamSource`が要求するのはこれより
/// 弱い制約）へ問題なく収まる。
pub struct PatchPanelSource {
    pub patch: Rc<RefCell<Op505Patch>>,
    pub patch_dirty: Rc<Cell<bool>>,
    pub master: Rc<RefCell<MasterEffectsState>>,
    pub master_dirty: Rc<Cell<bool>>,
    pub undo: Rc<RefCell<UndoStack>>,
}

impl PanelParamSource for PatchPanelSource {
    fn int(&self, field: IntField) -> Box<dyn IntParamHandle + '_> {
        match field {
            IntField::Patch(patch_field) => Box::new(PatchIntHandle {
                patch: self.patch.clone(),
                dirty: self.patch_dirty.clone(),
                undo: self.undo.clone(),
                field: patch_field,
            }),
            IntField::Fx(fx) => Box::new(MasterEffectsIntHandle {
                state: self.master.clone(),
                dirty: self.master_dirty.clone(),
                undo: self.undo.clone(),
                fx,
            }),
        }
    }

    fn boolean(&self, field: BoolField) -> Box<dyn BoolParamHandle + '_> {
        Box::new(PatchBoolHandle { patch: self.patch.clone(), dirty: self.patch_dirty.clone(), undo: self.undo.clone(), field })
    }

    fn eg(&self, slot: EgSlot) -> Box<dyn TimeEgHandle + '_> {
        Box::new(PatchTimeEgHandle { patch: self.patch.clone(), dirty: self.patch_dirty.clone(), undo: self.undo.clone(), slot })
    }
}
