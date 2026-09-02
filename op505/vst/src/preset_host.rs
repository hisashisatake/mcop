//! `op505_editor::preset_panel::PresetHost`のop505-vst実装。DAWパラメーター
//! （`ParamSetter`経由）とTimeEg 7本のpersist状態（`Op505EgBank`）を束ねる。

use nice_plug::prelude::*;
use op505_core::{Op505BankFile, Op505Patch, Op505PresetBank};
use op505_editor::preset_panel::PresetHost;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::params::{apply_patch as apply_daw_patch, apply_patch_egs, build_patch, Op505VstParams};

pub(crate) struct VstPresetHost<'a> {
    pub(crate) params: &'a Op505VstParams,
    pub(crate) setter: &'a ParamSetter<'a>,
    pub(crate) shared_bank: &'a Arc<RwLock<Op505PresetBank>>,
    pub(crate) dirty: &'a Arc<AtomicBool>,
}

impl PresetHost for VstPresetHost<'_> {
    fn current_patch(&self) -> Op505Patch {
        build_patch(self.params, &self.params.egs.read().expect("Poisoned RwLock on read"))
    }

    fn apply_patch(&self, patch: &Op505Patch) {
        apply_daw_patch(self.params, self.setter, patch);
        let mut egs = self.params.egs.write().expect("Poisoned RwLock on write");
        apply_patch_egs(&mut egs, patch);
    }

    /// 保存操作の結果を、音声スレッド（MIDI Program Change解決）へ即座に反映する。GUIはblocking
    /// writeで良い（`params.egs.write()`と同じ土俵）。書き込み完了後にdirtyを立てる順序が重要
    /// （先に立てるとオーディオスレッドが未反映のバンクを読みに行く隙が生まれる）。
    fn publish_bank(&self, bank_file: &Op505BankFile) {
        self.shared_bank.write().expect("Poisoned RwLock on write").merge_file(bank_file.as_presets_file());
        self.dirty.store(true, Ordering::Release);
    }
}
