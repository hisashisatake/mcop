use nice_plug::prelude::*;
use op505_ui::{BoolParamHandle, IntParamHandle, TimeEgHandle};
use sound_core::TimeEgParams;
use std::sync::{Arc, RwLock};

use crate::params::Op505EgBank;

/// `op505-ui`の`IntParamHandle`をnice-plugの`IntParam`+`ParamSetter`で実装するアダプタ
/// （`ym38x6-vst/src/param_adapter.rs`の`VstInt`と同型）。
pub(crate) struct VstInt<'a> {
    pub param: &'a IntParam,
    pub setter: &'a ParamSetter<'a>,
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
    }

    fn set(&self, value: i32) {
        self.setter.set_parameter(self.param, value);
    }

    fn end_edit(&self) {
        self.setter.end_set_parameter(self.param);
    }
}

/// `op505-ui`の`BoolParamHandle`をnice-plugの`BoolParam`+`ParamSetter`で実装するアダプタ。
pub(crate) struct VstBool<'a> {
    pub param: &'a BoolParam,
    pub setter: &'a ParamSetter<'a>,
}

impl BoolParamHandle for VstBool<'_> {
    fn value(&self) -> bool {
        self.param.modulated_plain_value()
    }

    fn begin_edit(&self) {
        self.setter.begin_set_parameter(self.param);
    }

    fn set(&self, value: bool) {
        self.setter.set_parameter(self.param, value);
    }

    fn end_edit(&self) {
        self.setter.end_set_parameter(self.param);
    }
}

/// `op505-ui`の`TimeEgHandle`を、nice-plugの`#[persist]`状態（`Arc<RwLock<Op505EgBank>>`）
/// 上の1本のTimeEgへ実装するアダプタ。DAWパラメーターではないため`begin_edit`/`end_edit`は
/// 空実装（オートメーションgestureが発生しない、`gesture-app`側の`Op505TimeEgHandle`と同型）。
/// `get`/`set`は非キャプチャ関数ポインタ（`Op505EgBank`内の該当箇所を指す）。
pub(crate) struct VstTimeEg<'a> {
    pub egs: &'a Arc<RwLock<Op505EgBank>>,
    pub get: fn(&Op505EgBank) -> TimeEgParams,
    pub set: fn(&mut Op505EgBank, TimeEgParams),
    pub name: &'static str,
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

    fn begin_edit(&self) {}
    fn end_edit(&self) {}
}

/// `&IntParam` + `&ParamSetter` から `Box<dyn IntParamHandle>` を組み立てる短縮ヘルパー。
pub(crate) fn vi<'a>(
    param: &'a IntParam,
    setter: &'a ParamSetter<'a>,
) -> Box<dyn IntParamHandle + 'a> {
    Box::new(VstInt { param, setter })
}

/// `&BoolParam` + `&ParamSetter` から `Box<dyn BoolParamHandle>` を組み立てる短縮ヘルパー。
pub(crate) fn vb<'a>(
    param: &'a BoolParam,
    setter: &'a ParamSetter<'a>,
) -> Box<dyn BoolParamHandle + 'a> {
    Box::new(VstBool { param, setter })
}

/// `Arc<RwLock<Op505EgBank>>` + get/set関数ポインタ から `Box<dyn TimeEgHandle>` を組み立てる
/// 短縮ヘルパー。
pub(crate) fn vt<'a>(
    egs: &'a Arc<RwLock<Op505EgBank>>,
    get: fn(&Op505EgBank) -> TimeEgParams,
    set: fn(&mut Op505EgBank, TimeEgParams),
    name: &'static str,
) -> Box<dyn TimeEgHandle + 'a> {
    Box::new(VstTimeEg { egs, get, set, name })
}
