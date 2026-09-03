//! `op505_editor::preset_panel::PresetHost`のstandalone実装。`EditorApp`が持つ
//! `Rc<RefCell<Op505Patch>>`/`Rc<Cell<bool>>`のRefCell/Cell部分だけを借用する
//! （`PresetHost`は`&self`メソッドのみで完結し`PatchPanelSource`のようにハンドルを
//! `'static`で持ち出す必要が無いため、`Rc`の二重間接参照は不要——deref coercionで
//! `&Rc<RefCell<_>>`から素通しできる）。

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use op505_core::{Op505BankFile, Op505Patch};
use op505_editor::preset_panel::PresetHost;

use crate::shared::SharedEditState;

pub(crate) struct StandalonePresetHost<'a> {
    pub(crate) patch: &'a RefCell<Op505Patch>,
    pub(crate) dirty: &'a Cell<bool>,
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

    /// standaloneはUndoが効くよう、+ New Voice/Deleteでの即時ディスク保存を行わない
    /// （Save/Save Asでのみ書き込む。`op505-editor::preset_panel::PresetHost`のdoc参照）。
    fn auto_save_bank_edits(&self) -> bool {
        false
    }
}
