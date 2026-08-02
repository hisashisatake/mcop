use nice_plug::prelude::*;
use ym38x6_ui::{BoolParamHandle, IntParamHandle};

/// `ym38x6-ui`の`IntParamHandle`をnice-plugの`IntParam`+`ParamSetter`で実装するアダプタ。
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

/// `ym38x6-ui`の`BoolParamHandle`をnice-plugの`BoolParam`+`ParamSetter`で実装するアダプタ。
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
