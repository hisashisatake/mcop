use nice_plug::prelude::*;
use op505_editor::panel_source::{MeterField, PanelParamSource};
use op505_editor::param_spec::{BoolField, EgSlot, FgSlot, IntField, OpIndex};
use op505_editor::undo::UndoStack;
use op505_ui::{BoolParamHandle, IntParamHandle, MeterHandle, TimeEgHandle};
use sound_core::{MeterBridge, Measurement, TimeEgParams};
use std::cell::RefCell;
use std::sync::{Arc, RwLock};

use crate::params::{bool_param_ref, int_param_ref, Op505EgBank, Op505VstParams};

/// `op505-ui`の`IntParamHandle`をnice-plugの`IntParam`+`ParamSetter`で実装するアダプタ
/// （`ym38x6-vst/src/param_adapter.rs`の`VstInt`と同型）。`undo`はUndo/Redoスタックへの
/// フック（`begin_edit`/`end_edit`から`note_begin_edit`/`note_end_edit`を呼ぶ）。
pub(crate) struct VstInt<'a> {
    pub param: &'a IntParam,
    pub setter: &'a ParamSetter<'a>,
    pub undo: &'a RefCell<UndoStack>,
}

impl IntParamHandle for VstInt<'_> {
    fn value(&self) -> i32 {
        self.param.modulated_plain_value()
    }

    fn min(&self) -> i32 {
        match self.param.range() {
            IntRange::Linear { min, .. } => min,
            IntRange::Reversed(range) => match range {
                IntRange::Linear { min, .. } => *min,
                IntRange::Reversed(_) => unreachable!("二重反転レンジは使用しない"),
            },
        }
    }

    fn max(&self) -> i32 {
        match self.param.range() {
            IntRange::Linear { max, .. } => max,
            IntRange::Reversed(range) => match range {
                IntRange::Linear { max, .. } => *max,
                IntRange::Reversed(_) => unreachable!("二重反転レンジは使用しない"),
            },
        }
    }

    fn default(&self) -> i32 {
        self.param.default_plain_value()
    }

    fn name(&self) -> String {
        self.param.name().to_string()
    }

    fn display(&self) -> String {
        self.param.to_string()
    }

    fn begin_edit(&self) {
        self.setter.begin_set_parameter(self.param);
        self.undo.borrow_mut().note_begin_edit();
    }

    fn set(&self, value: i32) {
        self.setter.set_parameter(self.param, value);
    }

    fn end_edit(&self) {
        self.setter.end_set_parameter(self.param);
        self.undo.borrow_mut().note_end_edit();
    }
}

/// `op505-ui`の`BoolParamHandle`をnice-plugの`BoolParam`+`ParamSetter`で実装するアダプタ。
pub(crate) struct VstBool<'a> {
    pub param: &'a BoolParam,
    pub setter: &'a ParamSetter<'a>,
    pub undo: &'a RefCell<UndoStack>,
}

impl BoolParamHandle for VstBool<'_> {
    fn value(&self) -> bool {
        self.param.modulated_plain_value()
    }

    fn begin_edit(&self) {
        self.setter.begin_set_parameter(self.param);
        self.undo.borrow_mut().note_begin_edit();
    }

    fn set(&self, value: bool) {
        self.setter.set_parameter(self.param, value);
    }

    fn end_edit(&self) {
        self.setter.end_set_parameter(self.param);
        self.undo.borrow_mut().note_end_edit();
    }
}

/// `op505-ui`の`TimeEgHandle`を、nice-plugの`#[persist]`状態（`Arc<RwLock<Op505EgBank>>`）
/// 上の1本のTimeEgへ実装するアダプタ。DAWパラメーターではないため`begin_edit`/`end_edit`は
/// `ParamSetter`を経由しない（オートメーションgestureは発生しない、`gesture-app`側の
/// `Op505TimeEgHandle`と同型）が、Undoスタックへのフックは行う。
/// `get`/`set`は非キャプチャ関数ポインタ（`Op505EgBank`内の該当箇所を指す）。
pub(crate) struct VstTimeEg<'a> {
    pub egs: &'a Arc<RwLock<Op505EgBank>>,
    pub get: fn(&Op505EgBank) -> TimeEgParams,
    pub set: fn(&mut Op505EgBank, TimeEgParams),
    pub name: &'static str,
    pub undo: &'a RefCell<UndoStack>,
}

impl TimeEgHandle for VstTimeEg<'_> {
    fn params(&self) -> TimeEgParams {
        (self.get)(&self.egs.read().expect("Poisoned RwLock on read"))
    }

    fn set_params(&self, params: TimeEgParams) {
        let mut bank = self.egs.write().expect("Poisoned RwLock on write");
        (self.set)(&mut bank, params);
    }

    fn name(&self) -> String {
        self.name.to_string()
    }

    fn begin_edit(&self) {
        self.undo.borrow_mut().note_begin_edit();
    }
    fn end_edit(&self) {
        self.undo.borrow_mut().note_end_edit();
    }
}

/// `&IntParam` + `&ParamSetter` + `&RefCell<UndoStack>` から `Box<dyn IntParamHandle>` を
/// 組み立てる短縮ヘルパー。
pub(crate) fn vi<'a>(
    param: &'a IntParam,
    setter: &'a ParamSetter<'a>,
    undo: &'a RefCell<UndoStack>,
) -> Box<dyn IntParamHandle + 'a> {
    Box::new(VstInt { param, setter, undo })
}

/// `&BoolParam` + `&ParamSetter` + `&RefCell<UndoStack>` から `Box<dyn BoolParamHandle>` を
/// 組み立てる短縮ヘルパー。
pub(crate) fn vb<'a>(
    param: &'a BoolParam,
    setter: &'a ParamSetter<'a>,
    undo: &'a RefCell<UndoStack>,
) -> Box<dyn BoolParamHandle + 'a> {
    Box::new(VstBool { param, setter, undo })
}

/// `Arc<RwLock<Op505EgBank>>` + get/set関数ポインタ + `&RefCell<UndoStack>` から
/// `Box<dyn TimeEgHandle>` を組み立てる短縮ヘルパー。
pub(crate) fn vt<'a>(
    egs: &'a Arc<RwLock<Op505EgBank>>,
    get: fn(&Op505EgBank) -> TimeEgParams,
    set: fn(&mut Op505EgBank, TimeEgParams),
    name: &'static str,
    undo: &'a RefCell<UndoStack>,
) -> Box<dyn TimeEgHandle + 'a> {
    Box::new(VstTimeEg { egs, get, set, name, undo })
}

/// `op505_editor::panel_source::PanelParamSource`のop505-vst実装。DAWパラメーター（`IntParam`/
/// `BoolParam`＋`ParamSetter`）とpersist状態（`Op505EgBank`）を、`vi`/`vb`/`vt`アダプタ経由で
/// `op505-editor`の`build_panel_params`へ渡せる形にする。`undo`はUndo/Redoスタックへの参照
/// （`op505-editor::patch_source::PatchPanelSource`の`Rc<RefCell<UndoStack>>`に相当、VSTは
/// `Send`制約のため`Rc`が使えず`RefCell`のみで保持する。詳細は`editor.rs`の`EditorState`）。
pub(crate) struct VstPanelSource<'a> {
    pub(crate) params: &'a Op505VstParams,
    pub(crate) setter: &'a ParamSetter<'a>,
    pub(crate) undo: &'a RefCell<UndoStack>,
    pub(crate) master_meter: &'a Arc<MeterBridge>,
}

/// マスター出力のレベルメーターへの読み取り専用ハンドル
/// （`op505_editor::patch_source::MasterMeterHandle`のVST版、同じ理由でUndo/dirty通知は不要）。
struct VstMeter<'a> {
    bridge: &'a Arc<MeterBridge>,
}

impl MeterHandle for VstMeter<'_> {
    fn snapshot(&self) -> Measurement {
        self.bridge.read()
    }
    fn reset_clip(&self) {
        self.bridge.reset_clip();
    }
    fn name(&self) -> String {
        "Master Output".to_string()
    }
}

impl PanelParamSource for VstPanelSource<'_> {
    fn int(&self, field: IntField) -> Box<dyn IntParamHandle + '_> {
        vi(int_param_ref(self.params, field), self.setter, self.undo)
    }

    fn boolean(&self, field: BoolField) -> Box<dyn BoolParamHandle + '_> {
        vb(bool_param_ref(self.params, field), self.setter, self.undo)
    }

    fn eg(&self, slot: EgSlot) -> Box<dyn TimeEgHandle + '_> {
        let egs = &self.params.egs;
        let name = slot.name();
        let undo = self.undo;
        match slot {
            EgSlot::Op(OpIndex::Op1) => vt(egs, |b| b.operators[0], |b, v| b.operators[0] = v, name, undo),
            EgSlot::Op(OpIndex::Op2) => vt(egs, |b| b.operators[1], |b, v| b.operators[1] = v, name, undo),
            EgSlot::Op(OpIndex::Op3) => vt(egs, |b| b.operators[2], |b, v| b.operators[2] = v, name, undo),
            EgSlot::Op(OpIndex::Op4) => vt(egs, |b| b.operators[3], |b, v| b.operators[3] = v, name, undo),
            EgSlot::Fg(FgSlot::Pitch) => vt(egs, |b| b.pitch_fg, |b, v| b.pitch_fg = v, name, undo),
            EgSlot::Fg(FgSlot::Cutoff) => vt(egs, |b| b.cutoff_fg, |b, v| b.cutoff_fg = v, name, undo),
            EgSlot::Fg(FgSlot::Gain) => vt(egs, |b| b.gain_fg, |b, v| b.gain_fg = v, name, undo),
        }
    }

    fn meter(&self, field: MeterField) -> Box<dyn MeterHandle + '_> {
        match field {
            MeterField::MasterOutput => Box::new(VstMeter { bridge: self.master_meter }),
        }
    }
}
