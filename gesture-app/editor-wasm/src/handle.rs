use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ym38x6_ui::{BoolParamHandle, IntParamHandle};

use crate::state::EditorState;

/// `EditorState`内の1つのi32フィールドへのハンドル。`get`/`set`はフィールドアクセサ関数ポインタ。
/// `dirty`は値変更時にセットし、`app.rs`側で1フレームに1回まとめてIPC送信するために使う。
pub struct IntField {
    pub state: Rc<RefCell<EditorState>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&EditorState) -> i32,
    pub set: fn(&mut EditorState, i32),
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

/// `EditorState`内の1つのboolフィールドへのハンドル。`IntField`と同じ仕組み。
pub struct BoolField {
    pub state: Rc<RefCell<EditorState>>,
    pub dirty: Rc<Cell<bool>>,
    pub get: fn(&EditorState) -> bool,
    pub set: fn(&mut EditorState, bool),
}

impl BoolParamHandle for BoolField {
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

/// チャンネル単位フィールド用の`Box<dyn IntParamHandle>`を組み立てる。
macro_rules! int_field {
    ($state:expr, $dirty:expr, $field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
        Box::new(IntField {
            state: $state.clone(),
            dirty: $dirty.clone(),
            get: |s: &EditorState| s.$field,
            set: |s: &mut EditorState, v: i32| s.$field = v,
            min: $min,
            max: $max,
            default: $default,
            name: $name,
        }) as Box<dyn ym38x6_ui::IntParamHandle>
    };
}

/// オペレーター単位フィールド(`operators[i].field`)用の`Box<dyn IntParamHandle>`を組み立てる。
macro_rules! op_int_field {
    ($state:expr, $dirty:expr, $idx:expr, $field:ident, $name:literal, $min:expr, $max:expr, $default:expr) => {
        Box::new(IntField {
            state: $state.clone(),
            dirty: $dirty.clone(),
            get: |s: &EditorState| s.operators[$idx].$field,
            set: |s: &mut EditorState, v: i32| s.operators[$idx].$field = v,
            min: $min,
            max: $max,
            default: $default,
            name: $name,
        }) as Box<dyn ym38x6_ui::IntParamHandle>
    };
}

pub(crate) use int_field;
pub(crate) use op_int_field;
