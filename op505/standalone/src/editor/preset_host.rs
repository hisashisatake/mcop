//! `op505_editor::preset_panel::PresetHost`のstandalone実装。`panel_params::Op505State`と
//! 同じ`Rc<RefCell<Op505Patch>>`+`Rc<Cell<bool>>`表現と、`SharedEditState`への発行を束ねる。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use op505_core::{Op505BankFile, Op505Patch};
use op505_editor::preset_panel::PresetHost;

use crate::shared::SharedEditState;

pub(crate) struct StandalonePresetHost<'a> {
    pub(crate) patch: &'a Rc<RefCell<Op505Patch>>,
    pub(crate) dirty: &'a Rc<Cell<bool>>,
    pub(crate) shared: &'a Arc<SharedEditState>,
}

impl PresetHost for StandalonePresetHost<'_> {
    fn current_patch(&self) -> Op505Patch {
        *self.patch.borrow()
    }

    fn apply_patch(&self, patch: &Op505Patch) {
        *self.patch.borrow_mut() = *patch;
        self.dirty.set(true);
    }

    fn publish_bank(&self, bank_file: &Op505BankFile) {
        self.shared.publish_bank_file(bank_file);
    }
}
