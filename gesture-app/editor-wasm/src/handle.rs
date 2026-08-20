use std::cell::{Cell, RefCell};
use std::rc::Rc;

use op505_ui::IntParamHandle;

use crate::state::MasterEffectsState;

/// `MasterEffectsState`内の1つのi32フィールドへのハンドル。`get`/`set`はフィールドアクセサ関数ポインタ。
/// `dirty`は値変更時にセットし、`app.rs`側で1フレームに1回まとめてIPC送信するために使う。
pub struct IntField {
    pub state: Rc<RefCell<MasterEffectsState>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&MasterEffectsState) -> i32,
    pub set: fn(&mut MasterEffectsState, i32),
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub name: &'static str,
}

impl IntParamHandle for IntField {
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
macro_rules! int_field {
    ($state:expr, $dirty:expr, $field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
        Box::new(crate::handle::IntField {
            state: $state.clone(),
            dirty: $dirty.clone(),
            get: |s: &MasterEffectsState| s.$field,
            set: |s: &mut MasterEffectsState, v: i32| s.$field = v,
            min: $min,
            max: $max,
            default: $default,
            name: $name,
        }) as Box<dyn op505_ui::IntParamHandle>
    };
}

pub(crate) use int_field;
