//! OP505のプリセット管理用Tauriコマンド。
//!
//! バンクレジストリ本体（`Op505BankFile`/`Op505BankRegistry`とその操作）は
//! `op505_core::preset_registry`へ昇格済み（op505-vstのPRESETSパネルと仕様を共有するため、
//! 詳細はCLAUDE.md op505/vst節・spec-fm.md 8章⑥参照）。このファイルに残るのはTauri結合部
//! （`#[tauri::command]`・`tauri::State`・ネイティブダイアログ）とDTO変換だけ。

use op505_core::{current_open_dir, Op505BankFile, Op505BankRegistry, Op505Engine, Op505Patch, Op505PresetBank, Op505PresetEntry, Op505PresetFile};
use std::sync::{Arc, Mutex};
use tauri_plugin_dialog::DialogExt;

/// `op505_list_bank_entries`が返すプリセット一覧の1件。
#[derive(serde::Serialize)]
pub struct PresetEntryDto {
    pub bank: u16,
    pub program: u8,
    pub name: String,
}

/// `op505_save_patch_overwrite`/`op505_save_patch_as`が成功時に返す保存結果。
/// `file_name`（実ファイル名、例: "organ_family.op505"）と`patch_name`（音色名、
/// バンクファイル内の`Op505PresetEntry.name`）は別概念のため分けて返す
/// （1ファイルに複数音色が入っている場合、ファイル名と音色名は一致しない）。
#[derive(serde::Serialize)]
pub struct SavedFileDto {
    pub patch_name: String,
    pub file_name: String,
    pub bank: u16,
    pub program: u8,
}

/// `op505_open_patch_file`/`op505_get_bank_program`が返す、読み込んだ音色の内容
/// （`Op505Patch`は専用DTOを介さず直接シリアライズする、`op505_set_patch`と同じ方針）。
#[derive(serde::Serialize)]
pub struct Op505LoadedPatchDto {
    pub patch: Op505Patch,
    pub patch_name: String,
    pub file_name: Option<String>,
    pub bank: u16,
    pub program: u8,
}

/// 今開いている（＝`registry`に登録済みの）bankのファイルが持つ音色一覧を返す
/// （`list_bank_entries`のOP505版。未登録なら空）。
#[tauri::command]
pub fn op505_list_bank_entries(registry: tauri::State<'_, Mutex<Op505BankRegistry>>, bank: u16) -> Vec<PresetEntryDto> {
    let reg = registry.lock().unwrap();
    reg.get(&bank)
        .map(|bank_file| bank_file.entries().iter().map(|e| PresetEntryDto { bank, program: e.program, name: e.name.clone() }).collect())
        .unwrap_or_default()
}

/// (bank, program)のプリセット内容を返す（エンジンへは反映しない読み取り専用、
/// `get_bank_program`のOP505版）。解決順位はレジストリ→`Op505PresetBank`→エンジンのカレントパッチ
/// （`.op505`には波形メモリ/GM2のようなフォールバックが無いため、`Op505Patch::default()`
/// （tl=0で無音）を黙って返すより、今鳴っている音を維持する方が安全という判断）。
#[tauri::command]
pub fn op505_get_bank_program(
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank_state: tauri::State<'_, Mutex<Op505PresetBank>>,
    engine: tauri::State<'_, Arc<Mutex<Op505Engine>>>,
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
    let patch = engine.lock().unwrap().current_patch();
    Op505LoadedPatchDto { patch, patch_name: String::new(), file_name: None, bank, program }
}

/// ネイティブOpenダイアログで`.op505`ファイルを選び、全音色を読み込む（`open_patch_file`のOP505版）。
/// ファイル自身が宣言しているbank番号は無視し、**今エディタで選択中のbank**へそのファイルの
/// 全音色を丸ごとロードする（ym38x6と同じ、ユーザー確認済みの仕様）。先頭エントリを画面へ反映する。
#[tauri::command]
pub async fn op505_open_patch_file(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank: u16,
) -> Result<Option<Op505LoadedPatchDto>, String> {
    let start_dir = current_open_dir(&registry.lock().unwrap(), bank);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().add_filter("op505", &["op505"]).set_directory(start_dir).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;

    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file = Op505PresetFile::from_json(&json).map_err(|e| e.to_string())?;
    let bank_file = Op505BankFile::from_loaded(path, file, bank);
    let entry = bank_file.entries().first().ok_or("ファイルに音色が含まれていません")?.clone();
    let file_name = bank_file.file_name().map(|s| s.to_string());
    let dto = Op505LoadedPatchDto { patch: entry.patch, patch_name: entry.name, file_name, bank, program: entry.program };

    registry.lock().unwrap().insert(bank, bank_file);
    Ok(Some(dto))
}

/// 現在のbankの担当ファイルへ上書き保存する（`save_patch_overwrite`のOP505版、未登録ならエラー）。
#[tauri::command]
pub fn op505_save_patch_overwrite(
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank: u16,
    program: u8,
    patch: Op505Patch,
    patch_name: String,
) -> Result<SavedFileDto, String> {
    let mut reg = registry.lock().unwrap();
    let bank_file = reg.get_mut(&bank).ok_or("このbankにはまだファイルがありません（先にOpenかSave Asしてください）")?;
    bank_file.upsert(program, patch_name.clone(), patch)?;
    bank_file.save()?;
    let file_name = bank_file.file_name().unwrap_or("?").to_string();
    Ok(SavedFileDto { patch_name, file_name, bank, program })
}

/// 現在のbankの担当ファイルへ新規エントリを追加して保存する（PRESETSリスト末尾の「+ New Voice」用）。
/// `patch`はフロント側が決める（通常クリック＝デフォルト初期化、Shift+クリック＝現在編集中のパッチの
/// コピー）。採番・命名は`Op505BankFile::add_new_voice`参照。担当ファイルが無いbank（先にOpen/Save Asが
/// 必要）はエラー（`op505_save_patch_overwrite`と同じ制約）。
#[tauri::command]
pub fn op505_add_preset(registry: tauri::State<'_, Mutex<Op505BankRegistry>>, bank: u16, patch: Op505Patch) -> Result<Op505LoadedPatchDto, String> {
    let mut reg = registry.lock().unwrap();
    let bank_file = reg.get_mut(&bank).ok_or("このbankにはまだファイルがありません（先にOpenかSave Asしてください）")?;
    let entry = bank_file.add_new_voice(patch)?;
    bank_file.save()?;
    let file_name = bank_file.file_name().map(|s| s.to_string());
    Ok(Op505LoadedPatchDto { patch: entry.patch, patch_name: entry.name, file_name, bank, program: entry.program })
}

/// 現在選択中の音色をDELETEキーで削除する（PRESETSリストの選択行＝ハイライト表示中の音色が対象、
/// マウスカーソルの位置には依存しない）。ネイティブのYes/No確認ダイアログを表示し、Yesが押されたときのみ
/// 削除してファイルへ保存し、削除後の音色一覧を返す。No/ダイアログを閉じた場合は`Ok(None)`
/// （削除は起きていない）。ダイアログ表示中に他コマンドがブロックされないよう、レジストリのロックは
/// ダイアログ表示の前後で一度ずつ短く取る（`op505_open_patch_file`と同じ方針）。
#[tauri::command]
pub async fn op505_delete_preset(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank: u16,
    program: u8,
) -> Result<Option<Vec<PresetEntryDto>>, String> {
    let name = {
        let reg = registry.lock().unwrap();
        let bank_file = reg.get(&bank).ok_or("このbankにはまだファイルがありません（先にOpenかSave Asしてください）")?;
        bank_file.entries().iter().find(|e| e.program == program).map(|e| e.name.clone()).ok_or("削除対象のエントリが見つかりません")?
    };

    let confirmed = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!("音色「{program:03} {name}」を削除しますか？"))
            .title("音色の削除")
            .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
            .buttons(tauri_plugin_dialog::MessageDialogButtons::YesNo)
            .blocking_show()
    })
    .await
    .map_err(|e| e.to_string())?;
    if !confirmed {
        return Ok(None);
    }

    let mut reg = registry.lock().unwrap();
    let bank_file = reg.get_mut(&bank).ok_or("このbankにはまだファイルがありません（先にOpenかSave Asしてください）")?;
    bank_file.remove(program)?;
    bank_file.save()?;
    Ok(Some(bank_file.entries().iter().map(|e| PresetEntryDto { bank, program: e.program, name: e.name.clone() }).collect()))
}

/// ネイティブSaveダイアログで保存先を選び、新規`.op505`ファイルとして書き出す
/// （`save_patch_as`のOP505版）。今のbankの担当ファイル（レジストリ）の全エントリーを複製元とし、
/// 今編集中のprogramだけ最新の内容に差し替えて丸ごと書き出す。保存後はそのbankの担当ファイルが
/// この新しいファイルに置き換わる。
#[tauri::command]
pub async fn op505_save_patch_as(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    patch: Op505Patch,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
) -> Result<Option<SavedFileDto>, String> {
    let start_dir = current_open_dir(&registry.lock().unwrap(), bank);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("op505", &["op505"])
            .set_directory(start_dir)
            .set_file_name(format!("{default_file_name}.op505"))
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;

    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("patch.op505").to_string();
    let base_entries: Vec<Op505PresetEntry> = registry.lock().unwrap().get(&bank).map(|bank_file| bank_file.entries().to_vec()).unwrap_or_default();
    let bank_file = Op505BankFile::write_as(patch, patch_name.clone(), bank, program, &base_entries, path)?;

    registry.lock().unwrap().insert(bank, bank_file);
    Ok(Some(SavedFileDto { patch_name, file_name, bank, program }))
}

// このファイルはTauriコマンド（結合部）のみを持つ。レジストリの優先順位ロジック等の実体は
// `op505_core::preset_registry`側で単体テスト済み（移設元: このファイルが元々持っていた
// `#[cfg(test)] mod tests`8本）。
