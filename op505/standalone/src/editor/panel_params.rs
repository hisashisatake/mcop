//! op505-uiパネル（`draw_op505_panel`）向けの状態とハンドル。
//! `gesture-app/editor-wasm/src/op505_state.rs`+`handle.rs`の移植（wasm依存を除いただけで
//! ロジックは同一）。フラットなミラー構造体を新設せず`Op505Patch`（op505-core、`Default`実装済み）
//! をそのまま`Rc<RefCell<>>`で保持する方針も踏襲する。
//!
//! MASTER EFFECTS（`MasterEffectsState`）はエンジン非依存のフロント側ミラーで、`op505-vst`の
//! DAWパラメーターやgesture-appのIPC送信用ミラーと同じ役割。本サブステップ（Step 1の
//! パネル表示・編集対象ch）ではまだ`shared::SharedEditState`のFX APIへ配線しない
//! （配線は後続のサブステップ）——このファイルはあくまで`Op505PanelParams`を組み立てるための
//! 状態一式を提供するだけ。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use op505_core::Op505Patch;
use op505_editor::param_spec::{FgSlot, FxInt, OpInt, PatchInt};
use op505_ui::{BoolParamHandle, IntParamHandle, Op505BipolarFgPanelParams, Op505OperatorPanelParams, Op505PanelParams, TimeEgHandle};
use sound_core::TimeEgParams;

/// MASTER EFFECTS（Reverb/Chorus）パネル用の状態。エンジン非依存のため`Op505State`と分離して
/// 保持し、`Op505State::build_panel_params`から共有参照される。
#[derive(Clone, Copy)]
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
        }
    }
}

/// `MasterEffectsState`内の1つのi32フィールドへのハンドル。`get`/`set`はフィールドアクセサ関数ポインタ。
pub struct MasterEffectsIntField {
    pub state: Rc<RefCell<MasterEffectsState>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&MasterEffectsState) -> i32,
    pub set: fn(&mut MasterEffectsState, i32),
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub name: &'static str,
}

impl IntParamHandle for MasterEffectsIntField {
    fn value(&self) -> i32 {
        (self.get)(&self.state.borrow())
    }
    fn min(&self) -> i32 {
        self.min
    }
    fn max(&self) -> i32 {
        self.max
    }
    fn default(&self) -> i32 {
        self.default
    }
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        let clamped = value.clamp(self.min, self.max);
        (self.set)(&mut self.state.borrow_mut(), clamped);
        self.dirty.set(true);
    }
    fn end_edit(&self) {}
}

/// MASTER EFFECTSフィールド用の`Box<dyn IntParamHandle>`を組み立てる。
/// `$spec`はop505-editorの正本（`FxInt::spec()`）。
macro_rules! master_int_field {
    ($state:expr, $dirty:expr, $field:ident, $spec:expr) => {
        Box::new(crate::editor::panel_params::MasterEffectsIntField {
            state: $state.clone(),
            dirty: $dirty.clone(),
            get: |s: &MasterEffectsState| s.$field,
            set: |s: &mut MasterEffectsState, v: i32| s.$field = v,
            min: $spec.min,
            max: $spec.max,
            default: $spec.default,
            name: $spec.short_name,
        }) as Box<dyn IntParamHandle>
    };
}

/// `Op505Patch`内の1つのi32相当(u8)フィールドへの単純ハンドル。
pub struct Op505IntField {
    pub state: Rc<RefCell<Op505Patch>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&Op505Patch) -> i32,
    pub set: fn(&mut Op505Patch, i32),
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub name: &'static str,
}

impl IntParamHandle for Op505IntField {
    fn value(&self) -> i32 {
        (self.get)(&self.state.borrow())
    }
    fn min(&self) -> i32 {
        self.min
    }
    fn max(&self) -> i32 {
        self.max
    }
    fn default(&self) -> i32 {
        self.default
    }
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        let clamped = value.clamp(self.min, self.max);
        (self.set)(&mut self.state.borrow_mut(), clamped);
        self.dirty.set(true);
    }
    fn end_edit(&self) {}
}

/// `Op505Patch`内の1つのboolフィールドへの単純ハンドル。
pub struct Op505BoolField {
    pub state: Rc<RefCell<Op505Patch>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&Op505Patch) -> bool,
    pub set: fn(&mut Op505Patch, bool),
}

impl BoolParamHandle for Op505BoolField {
    fn value(&self) -> bool {
        (self.get)(&self.state.borrow())
    }
    fn begin_edit(&self) {}
    fn set(&self, value: bool) {
        (self.set)(&mut self.state.borrow_mut(), value);
        self.dirty.set(true);
    }
    fn end_edit(&self) {}
}

/// TimeEgParams全体（Op505Patch内の7箇所どこか）への`ui_core::TimeEgHandle`実装。
/// `get_eg`/`set_eg`は非キャプチャ関数ポインタ（Op505Patch内の該当箇所を指す）。
pub struct Op505TimeEgHandle {
    state: Rc<RefCell<Op505Patch>>,
    dirty: Rc<Cell<bool>>,
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
    name: &'static str,
}

impl TimeEgHandle for Op505TimeEgHandle {
    fn params(&self) -> TimeEgParams {
        (self.get_eg)(&self.state.borrow())
    }
    fn set_params(&self, params: TimeEgParams) {
        (self.set_eg)(&mut self.state.borrow_mut(), params);
        self.dirty.set(true);
    }
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn begin_edit(&self) {}
    fn end_edit(&self) {}
}

fn time_eg_handle(
    state: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
    name: &'static str,
) -> Box<dyn TimeEgHandle> {
    Box::new(Op505TimeEgHandle { state: state.clone(), dirty: dirty.clone(), get_eg, set_eg, name })
}

/// バイポーラFG（Pitch/Cutoff/Gain）1本ぶんの`Op505BipolarFgPanelParams`を組み立てる。
/// Depthのmin/max/defaultはop505-editorの正本（`PatchInt::FgDepth(fg).spec()`）。
fn bipolar_fg_panel_params(
    state: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    get_depth: fn(&Op505Patch) -> i32,
    set_depth: fn(&mut Op505Patch, i32),
    get_eg: fn(&Op505Patch) -> TimeEgParams,
    set_eg: fn(&mut Op505Patch, TimeEgParams),
    eg_name: &'static str,
    fg: FgSlot,
) -> Op505BipolarFgPanelParams<'static> {
    let spec = PatchInt::FgDepth(fg).spec();
    Op505BipolarFgPanelParams {
        eg: time_eg_handle(state, dirty, get_eg, set_eg, eg_name),
        depth: Box::new(Op505IntField {
            state: state.clone(),
            dirty: dirty.clone(),
            get: get_depth,
            set: set_depth,
            min: spec.min,
            max: spec.max,
            default: spec.default,
            name: spec.short_name,
        }) as Box<dyn IntParamHandle>,
    }
}

fn operator_panel_params<const I: usize>(
    state: &Rc<RefCell<Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
) -> Op505OperatorPanelParams<'static> {
    macro_rules! op {
        ($field:ident, $spec:expr) => {
            Box::new(Op505IntField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.operators[I].$field as i32,
                set: |p: &mut Op505Patch, v: i32| p.operators[I].$field = v as u8,
                min: $spec.min,
                max: $spec.max,
                default: $spec.default,
                name: $spec.short_name,
            }) as Box<dyn IntParamHandle>
        };
    }
    let eg_name: &'static str = match I {
        0 => "OP1 EG",
        1 => "OP2 EG",
        2 => "OP3 EG",
        3 => "OP4 EG",
        _ => "OP EG",
    };
    Op505OperatorPanelParams {
        tl: op!(tl, OpInt::Tl.spec()),
        eg: time_eg_handle(
            state,
            dirty,
            |p: &Op505Patch| p.operators[I].eg,
            |p: &mut Op505Patch, v: TimeEgParams| p.operators[I].eg = v,
            eg_name,
        ),
        mul: op!(mul, OpInt::Mul.spec()),
        dt1: op!(dt1, OpInt::Dt1.spec()),
        ksr: op!(ksr, OpInt::Ksr.spec()),
        vel_sens: op!(velocity_sensitivity, OpInt::VelSens.spec()),
        op_fine_tune: op!(op_fine_tune, OpInt::OpFineTune.spec()),
        ame: Box::new(Op505BoolField {
            state: state.clone(),
            dirty: dirty.clone(),
            get: |p: &Op505Patch| p.operators[I].am_enable,
            set: |p: &mut Op505Patch, v: bool| p.operators[I].am_enable = v,
        }) as Box<dyn BoolParamHandle>,
        waveform: op!(waveform, OpInt::Waveform.spec()),
        eg_shift: op!(eg_shift, OpInt::EgShift.spec()),
        level_scale: op!(level_scale, OpInt::LevelScale.spec()),
        velocity_gain: op!(velocity_gain, OpInt::VelocityGain.spec()),
    }
}

/// `Op505Patch`の状態と`app.rs`から呼ぶ組み立て関数一式。
pub struct Op505State {
    pub patch: Rc<RefCell<Op505Patch>>,
    pub dirty: Rc<Cell<bool>>,
}

impl Op505State {
    /// `initial`から開始する（standaloneでは`SharedEditState`の現在値、gesture-appの
    /// `Op505Patch::default()`起点＋初期`handle_navigate`とは異なりPRESETS未実装のため
    /// 呼び出し側がここへ初期値を渡す。詳細は`app.rs`）。
    pub fn new(initial: Op505Patch) -> Self {
        Self { patch: Rc::new(RefCell::new(initial)), dirty: Rc::new(Cell::new(false)) }
    }

    pub fn build_panel_params(
        &self,
        master_state: &Rc<RefCell<MasterEffectsState>>,
        master_dirty: &Rc<Cell<bool>>,
    ) -> Op505PanelParams<'static> {
        let state = &self.patch;
        let dirty = &self.dirty;
        macro_rules! ch {
            ($field:ident, $spec:expr) => {
                Box::new(Op505IntField {
                    state: state.clone(),
                    dirty: dirty.clone(),
                    get: |p: &Op505Patch| p.channel.$field as i32,
                    set: |p: &mut Op505Patch, v: i32| p.channel.$field = v as u8,
                    min: $spec.min,
                    max: $spec.max,
                    default: $spec.default,
                    name: $spec.short_name,
                }) as Box<dyn IntParamHandle>
            };
        }
        Op505PanelParams {
            algorithm: ch!(algorithm, PatchInt::Algorithm.spec()),
            feedback: ch!(feedback, PatchInt::Feedback.spec()),
            fixed_note_enable: Box::new(Op505BoolField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.channel.fixed_note_enable,
                set: |p: &mut Op505Patch, v: bool| p.channel.fixed_note_enable = v,
            }) as Box<dyn BoolParamHandle>,
            fixed_note: ch!(fixed_note, PatchInt::FixedNote.spec()),
            fixed_note_fine: ch!(fixed_note_fine, PatchInt::FixedNoteFine.spec()),
            cutoff: ch!(filter_cutoff, PatchInt::Cutoff.spec()),
            resonance: ch!(filter_resonance, PatchInt::Resonance.spec()),
            filter_type: ch!(filter_type, PatchInt::FilterType.spec()),
            filter_self_oscillation: Box::new(Op505BoolField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.channel.filter_self_oscillation,
                set: |p: &mut Op505Patch, v: bool| p.channel.filter_self_oscillation = v,
            }) as Box<dyn BoolParamHandle>,
            pitch_fg: bipolar_fg_panel_params(
                state,
                dirty,
                |p: &Op505Patch| p.channel.pitch_fg.depth as i32,
                |p: &mut Op505Patch, v: i32| p.channel.pitch_fg.depth = v as u8,
                |p: &Op505Patch| p.channel.pitch_fg.eg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.pitch_fg.eg = v,
                "PITCH FG",
                FgSlot::Pitch,
            ),
            cutoff_fg: bipolar_fg_panel_params(
                state,
                dirty,
                |p: &Op505Patch| p.channel.cutoff_fg.depth as i32,
                |p: &mut Op505Patch, v: i32| p.channel.cutoff_fg.depth = v as u8,
                |p: &Op505Patch| p.channel.cutoff_fg.eg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.cutoff_fg.eg = v,
                "CUTOFF FG",
                FgSlot::Cutoff,
            ),
            gain_fg: bipolar_fg_panel_params(
                state,
                dirty,
                |p: &Op505Patch| p.channel.gain_fg.depth as i32,
                |p: &mut Op505Patch, v: i32| p.channel.gain_fg.depth = v as u8,
                |p: &Op505Patch| p.channel.gain_fg.eg,
                |p: &mut Op505Patch, v: TimeEgParams| p.channel.gain_fg.eg = v,
                "GAIN FG",
                FgSlot::Gain,
            ),
            gain_fg_to_master: Box::new(Op505BoolField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.channel.gain_fg_to_master,
                set: |p: &mut Op505Patch, v: bool| p.channel.gain_fg_to_master = v,
            }) as Box<dyn BoolParamHandle>,
            gain_fg_to_operators: Box::new(Op505BoolField {
                state: state.clone(),
                dirty: dirty.clone(),
                get: |p: &Op505Patch| p.channel.gain_fg_to_operators,
                set: |p: &mut Op505Patch, v: bool| p.channel.gain_fg_to_operators = v,
            }) as Box<dyn BoolParamHandle>,
            rev_send: master_int_field!(master_state, master_dirty, rev_send, FxInt::RevSend.spec()),
            reverb_type: master_int_field!(master_state, master_dirty, reverb_type, FxInt::ReverbType.spec()),
            reverb_time: master_int_field!(master_state, master_dirty, reverb_time, FxInt::ReverbTime.spec()),
            cho_send: master_int_field!(master_state, master_dirty, cho_send, FxInt::ChoSend.spec()),
            chorus_type: master_int_field!(master_state, master_dirty, chorus_type, FxInt::ChorusType.spec()),
            chorus_mod_rate: master_int_field!(master_state, master_dirty, chorus_mod_rate, FxInt::ChorusModRate.spec()),
            chorus_mod_depth: master_int_field!(master_state, master_dirty, chorus_mod_depth, FxInt::ChorusModDepth.spec()),
            chorus_feedback: master_int_field!(master_state, master_dirty, chorus_feedback, FxInt::ChorusFeedback.spec()),
            chorus_send_to_reverb: master_int_field!(master_state, master_dirty, chorus_send_to_reverb, FxInt::ChorusSendToReverb.spec()),
            operators: [
                operator_panel_params::<0>(state, dirty),
                operator_panel_params::<1>(state, dirty),
                operator_panel_params::<2>(state, dirty),
                operator_panel_params::<3>(state, dirty),
            ],
        }
    }
}
