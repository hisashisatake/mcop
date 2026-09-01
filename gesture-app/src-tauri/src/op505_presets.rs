//! OP505のプリセット一覧・読み取り用Tauriコマンド（読み取り専用）。
//!
//! gesture-appはエンジンを持たないMIDIコントローラーのため、`.op505`ファイルの編集
//! （Open/Save/Save As/+ New Voice/Delete）は行わない——それらは`op505-standalone`の
//! トレイ起動音色エディタが担う（詳細はCLAUDE.md op505/standalone節）。ここに残るのは
//! Bank/Programの名前解決だけを目的にした読み取りコマンドで、`op505_core::preset_registry`
//! （op505-vst/standaloneのPRESETSパネルと共有）をTauri結合部から呼ぶ薄いラッパー。

use op505_core::{Op505BankRegistry, Op505Patch, Op505PresetBank};
use std::sync::Mutex;

/// `op505_list_bank_entries`が返すプリセット一覧の1件。
#[derive(serde::Serialize)]
pub struct PresetEntryDto {
    pub bank: u16,
    pub program: u8,
    pub name: String,
}

/// `op505_get_bank_program`が返す、読み込んだ音色の内容
/// （`Op505Patch`は専用DTOを介さず直接シリアライズする）。
#[derive(serde::Serialize)]
pub struct Op505LoadedPatchDto {
    pub patch: Op505Patch,
    pub patch_name: String,
    pub file_name: Option<String>,
    pub bank: u16,
    pub program: u8,
}

/// 今開いている（＝`registry`に登録済みの）bankのファイルが持つ音色一覧を返す
/// （未登録なら空）。
#[tauri::command]
pub fn op505_list_bank_entries(registry: tauri::State<'_, Mutex<Op505BankRegistry>>, bank: u16) -> Vec<PresetEntryDto> {
    let reg = registry.lock().unwrap();
    reg.get(&bank)
        .map(|bank_file| bank_file.entries().iter().map(|e| PresetEntryDto { bank, program: e.program, name: e.name.clone() }).collect())
        .unwrap_or_default()
}

/// bankの担当ファイル名だけを返す（読み込みはしない）。
#[tauri::command]
pub fn op505_get_bank_file_name(registry: tauri::State<'_, Mutex<Op505BankRegistry>>, bank: u16) -> Option<String> {
    registry.lock().unwrap().get(&bank).and_then(|bank_file| bank_file.file_name().map(str::to_string))
}

/// (bank, program)のプリセット内容を返す（読み取り専用）。解決順位はレジストリ→
/// `Op505PresetBank`。どちらにも無ければ`Op505Patch::default()`（tl=0で無音）を返す
/// （gesture-appは生きたエンジンを持たないため、旧来の「現在鳴っている音を維持」という
/// フォールバックは意味を持たない。呼び出し側は`patch_name`が空文字かで未検出を判定できる）。
#[tauri::command]
pub fn op505_get_bank_program(
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank_state: tauri::State<'_, Mutex<Op505PresetBank>>,
    bank: u16,
    program: u8,
) -> Op505LoadedPatchDto {
    {
        let reg = registry.lock().unwrap();
        if let Some(bank_file) = reg.get(&bank) {
            if let Some(entry) = bank_file.entries().iter().find(|e| e.program == program) {
                let file_name = bank_file.file_name().map(|s| s.to_string());
                return Op505LoadedPatchDto { patch: entry.patch, patch_name: entry.name.clone(), file_name, bank, program };
            }
        }
    }
    if let Some(preset) = bank_state.lock().unwrap().get(bank, program) {
        return Op505LoadedPatchDto { patch: preset.patch, patch_name: preset.name.clone(), file_name: None, bank, program };
    }
    Op505LoadedPatchDto { patch: Op505Patch::default(), patch_name: String::new(), file_name: None, bank, program }
}
