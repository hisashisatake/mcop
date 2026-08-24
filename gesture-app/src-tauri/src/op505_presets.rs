//! OP505のプリセットレジストリ + Open/Save/Save As用Tauriコマンド。
//!
//! bank番号ごとに「担当ファイル」を覚え、Open/Save/Save Asがセッション中に更新する
//! （1バンク=1ファイルという運用が前提）。

use op505_core::{op505_presets_dir, Op505Engine, Op505Patch, Op505PresetBank, Op505PresetEntry, Op505PresetFile};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// bank番号ごとの「担当ファイル」（`op505_presets_dir()`全体の中で、そのbankを最後に定義したファイル）。
/// Open/Save/Save Asが直接更新する（音色エディタのファイル状態はpresets_dirの外/中を区別しない、
/// ym38x6側の`BankFile`と同じ意味論）。
pub struct Op505BankFile {
    path: PathBuf,
    file: Op505PresetFile,
}
pub type Op505BankRegistry = HashMap<u16, Op505BankFile>;

/// `dir`内の`.op505`ファイルをファイル名昇順で走査し、bank番号ごとに「最後に処理したファイル」を
/// レジストリへ記録する（`build_registry`のOP505版、同じ優先順位）。起動時（または
/// `op505_reload_presets`）に呼ぶ。
pub fn build_op505_registry(dir: &Path) -> Op505BankRegistry {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("op505"))
        .collect();
    paths.sort();

    let mut registry = HashMap::new();
    for path in paths {
        let Ok(json) = std::fs::read_to_string(&path) else { continue };
        let Ok(file) = Op505PresetFile::from_json(&json) else { continue };
        registry.insert(bank_of(&file), Op505BankFile { path, file });
    }
    registry
}

/// `Op505PresetFile`のPresets/Programsどちらのvariantでもエントリ一覧への参照を取り出す。
fn entries(file: &Op505PresetFile) -> &Vec<Op505PresetEntry> {
    match file {
        Op505PresetFile::Presets { presets, .. } => presets,
        Op505PresetFile::Programs { programs, .. } => programs,
    }
}

/// `entries`の可変参照版。
fn entries_mut(file: &mut Op505PresetFile) -> &mut Vec<Op505PresetEntry> {
    match file {
        Op505PresetFile::Presets { presets, .. } => presets,
        Op505PresetFile::Programs { programs, .. } => programs,
    }
}

/// `Op505PresetFile`のPresets/Programsどちらのvariantでもbank番号を取り出す。
fn bank_of(file: &Op505PresetFile) -> u16 {
    match file {
        Op505PresetFile::Presets { bank, .. } | Op505PresetFile::Programs { bank, .. } => *bank,
    }
}

/// `file`のbankフィールドを`bank`へ書き換えたものを返す（variant・エントリー内容はそのまま）。
fn with_bank(file: Op505PresetFile, bank: u16) -> Op505PresetFile {
    match file {
        Op505PresetFile::Presets { presets, .. } => Op505PresetFile::Presets { bank, presets },
        Op505PresetFile::Programs { programs, .. } => Op505PresetFile::Programs { bank, programs },
    }
}

/// Open/Save Asのネイティブダイアログの初期ディレクトリを決める。指定bankがレジストリに
/// 登録済みならその親ディレクトリ、未登録なら`op505_presets_dir()`。
fn current_open_dir(registry: &Op505BankRegistry, bank: u16) -> PathBuf {
    registry.get(&bank).and_then(|bank_file| bank_file.path.parent().map(PathBuf::from)).unwrap_or_else(op505_presets_dir)
}

/// (bank, program)に対応するパッチを解決する（`op505_set_program`用。ym38x6の
/// `resolve_patch`と異なり波形メモリ/GM2/プレースホルダーの代替は無いため、
/// レジストリ→`Op505PresetBank`の順で見つからなければ`None`を返す）。
pub fn resolve_patch(registry: &Op505BankRegistry, bank_state: &Op505PresetBank, bank: u16, program: u8) -> Option<Op505Patch> {
    registry
        .get(&bank)
        .and_then(|bank_file| entries(&bank_file.file).iter().find(|e| e.program == program).map(|e| e.patch))
        .or_else(|| bank_state.get(bank, program).map(|preset| preset.patch))
}

/// 今開いている（＝`registry`に登録済みの）bankのファイルが持つ音色一覧を返す
/// （`list_bank_entries`のOP505版。未登録なら空）。
#[tauri::command]
pub fn op505_list_bank_entries(registry: tauri::State<'_, Mutex<Op505BankRegistry>>, bank: u16) -> Vec<PresetEntryDto> {
    let reg = registry.lock().unwrap();
    reg.get(&bank)
        .map(|bank_file| entries(&bank_file.file).iter().map(|e| PresetEntryDto { bank, program: e.program, name: e.name.clone() }).collect())
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
            if let Some(entry) = entries(&bank_file.file).iter().find(|e| e.program == program) {
                let file_name = bank_file.path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string());
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
    let file = with_bank(Op505PresetFile::from_json(&json).map_err(|e| e.to_string())?, bank);
    let entry = entries(&file).first().ok_or("ファイルに音色が含まれていません")?.clone();
    let file_name = path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string());
    let dto = Op505LoadedPatchDto { patch: entry.patch, patch_name: entry.name, file_name, bank, program: entry.program };

    registry.lock().unwrap().insert(bank, Op505BankFile { path, file });
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
    let list = entries_mut(&mut bank_file.file);
    let entry = list.iter_mut().find(|e| e.program == program).ok_or("保存先エントリが見つかりません")?;
    entry.patch = patch;
    entry.name = patch_name.clone();
    let file_name = bank_file.path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
    let json = bank_file.file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&bank_file.path, json).map_err(|e| e.to_string())?;
    Ok(SavedFileDto { patch_name, file_name, bank, program })
}

/// `add_preset_entry`が新規追加した`Op505PresetEntry`。program番号は既存エントリの最大値+1
/// （空なら0）、名前は"VoiceNNN"（NNNは新program番号を3桁ゼロ埋め）。program番号はu8のため、
/// 既に255まで埋まっている場合はエラーを返す。`patch`は呼び出し元が決める（デフォルト初期化か、
/// 現在編集中のパッチのコピーか＝PRESETSリストの「+ New Voice」通常クリック/Shift+クリックの分岐）。
fn add_preset_entry(file: &mut Op505PresetFile, patch: Op505Patch) -> Result<Op505PresetEntry, String> {
    let list = entries_mut(file);
    let next_program = match list.iter().map(|e| e.program).max() {
        Some(max) => max.checked_add(1).ok_or("これ以上音色を追加できません（program番号が上限に達しました）")?,
        None => 0,
    };
    let entry = Op505PresetEntry { program: next_program, name: format!("Voice{next_program:03}"), patch };
    list.push(entry.clone());
    list.sort_by_key(|e| e.program);
    Ok(entry)
}

/// 現在のbankの担当ファイルへ新規エントリを追加して保存する（PRESETSリスト末尾の「+ New Voice」用）。
/// `patch`はフロント側が決める（通常クリック＝デフォルト初期化、Shift+クリック＝現在編集中のパッチの
/// コピー）。採番・命名は`add_preset_entry`参照。担当ファイルが無いbank（先にOpen/Save Asが必要）は
/// エラー（`op505_save_patch_overwrite`と同じ制約）。
#[tauri::command]
pub fn op505_add_preset(registry: tauri::State<'_, Mutex<Op505BankRegistry>>, bank: u16, patch: Op505Patch) -> Result<Op505LoadedPatchDto, String> {
    let mut reg = registry.lock().unwrap();
    let bank_file = reg.get_mut(&bank).ok_or("このbankにはまだファイルがありません（先にOpenかSave Asしてください）")?;
    let entry = add_preset_entry(&mut bank_file.file, patch)?;
    let file_name = bank_file.path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string());
    let json = bank_file.file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&bank_file.path, json).map_err(|e| e.to_string())?;
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
        entries(&bank_file.file)
            .iter()
            .find(|e| e.program == program)
            .map(|e| e.name.clone())
            .ok_or("削除対象のエントリが見つかりません")?
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
    let list = entries_mut(&mut bank_file.file);
    let idx = list.iter().position(|e| e.program == program).ok_or("削除対象のエントリが見つかりません")?;
    list.remove(idx);
    let json = bank_file.file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&bank_file.path, json).map_err(|e| e.to_string())?;
    Ok(Some(entries(&bank_file.file).iter().map(|e| PresetEntryDto { bank, program: e.program, name: e.name.clone() }).collect()))
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
    let mut presets: Vec<Op505PresetEntry> =
        registry.lock().unwrap().get(&bank).map(|bank_file| entries(&bank_file.file).clone()).unwrap_or_default();
    match presets.iter_mut().find(|e| e.program == program) {
        Some(entry) => {
            entry.patch = patch;
            entry.name = patch_name.clone();
        }
        None => presets.push(Op505PresetEntry { program, name: patch_name.clone(), patch }),
    }
    presets.sort_by_key(|e| e.program);
    let file = Op505PresetFile::Presets { bank, presets };
    let json = file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;

    registry.lock().unwrap().insert(bank, Op505BankFile { path, file });
    Ok(Some(SavedFileDto { patch_name, file_name, bank, program }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("op505_gesture_app_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_json(bank: u16, program: u8, name: &str) -> String {
        Op505PresetFile::Programs {
            bank,
            programs: vec![Op505PresetEntry { program, name: name.to_string(), patch: Op505Patch::default() }],
        }
        .to_json()
        .unwrap()
    }

    #[test]
    fn build_registry_maps_bank_to_its_file() {
        let dir = unique_temp_dir("registry_basic");
        std::fs::write(dir.join("a.op505"), sample_json(0, 5, "Foo")).unwrap();
        std::fs::write(dir.join("b.op505"), sample_json(1, 2, "Bar")).unwrap();

        let registry = build_op505_registry(&dir);
        assert_eq!(registry.get(&0).unwrap().path, dir.join("a.op505"));
        assert_eq!(registry.get(&1).unwrap().path, dir.join("b.op505"));
        assert!(registry.get(&2).is_none(), "存在しないbankは登録されない");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_registry_prefers_last_file_in_sorted_order() {
        let dir = unique_temp_dir("registry_precedence");
        std::fs::write(dir.join("a_first.op505"), sample_json(0, 0, "First")).unwrap();
        std::fs::write(dir.join("b_second.op505"), sample_json(0, 0, "Second")).unwrap();

        let registry = build_op505_registry(&dir);
        assert_eq!(registry.get(&0).unwrap().path, dir.join("b_second.op505"), "後読みのファイルが優先されるべき");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn with_bank_overrides_bank_and_keeps_entries() {
        let presets = Op505PresetFile::Presets {
            bank: 3,
            presets: vec![Op505PresetEntry { program: 5, name: "Foo".to_string(), patch: Op505Patch::default() }],
        };
        let rebanked = with_bank(presets, 9);
        assert_eq!(bank_of(&rebanked), 9);
        assert_eq!(entries(&rebanked).len(), 1);
        assert_eq!(entries(&rebanked)[0].program, 5);

        let programs = Op505PresetFile::Programs { bank: 3, programs: vec![] };
        assert_eq!(bank_of(&with_bank(programs, 9)), 9);
    }

    #[test]
    fn add_preset_entry_starts_at_zero_when_empty() {
        let mut file = Op505PresetFile::Presets { bank: 0, presets: vec![] };
        let entry = add_preset_entry(&mut file, Op505Patch::default()).unwrap();
        assert_eq!(entry.program, 0);
        assert_eq!(entry.name, "Voice000");
        assert_eq!(entry.patch, Op505Patch::default());
        assert_eq!(entries(&file).len(), 1);
    }

    #[test]
    fn add_preset_entry_uses_max_program_plus_one() {
        let mut file = Op505PresetFile::Presets {
            bank: 0,
            presets: vec![
                Op505PresetEntry { program: 3, name: "A".to_string(), patch: Op505Patch::default() },
                Op505PresetEntry { program: 31, name: "B".to_string(), patch: Op505Patch::default() },
            ],
        };
        let entry = add_preset_entry(&mut file, Op505Patch::default()).unwrap();
        assert_eq!(entry.program, 32);
        assert_eq!(entry.name, "Voice032");
        assert_eq!(entries(&file).last().unwrap().program, 32, "sortされ末尾に来るはず");
    }

    #[test]
    fn add_preset_entry_copies_given_patch() {
        let mut file = Op505PresetFile::Presets { bank: 0, presets: vec![] };
        let mut source = Op505Patch::default();
        source.operators[0].tl = 123;
        let entry = add_preset_entry(&mut file, source).unwrap();
        assert_eq!(entry.patch.operators[0].tl, 123, "Shift+クリックのコピー元がそのまま複製されるはず");
    }

    #[test]
    fn add_preset_entry_errors_when_program_exhausted() {
        let mut file = Op505PresetFile::Presets {
            bank: 0,
            presets: vec![Op505PresetEntry { program: 255, name: "Last".to_string(), patch: Op505Patch::default() }],
        };
        assert!(add_preset_entry(&mut file, Op505Patch::default()).is_err());
    }

    #[test]
    fn current_open_dir_falls_back_to_presets_dir_when_bank_unregistered() {
        let registry = Op505BankRegistry::new();
        assert_eq!(current_open_dir(&registry, 0), op505_presets_dir());
    }

    #[test]
    fn resolve_patch_prefers_registry_then_bank_state() {
        let mut registry = Op505BankRegistry::new();
        let mut patch_in_registry = Op505Patch::default();
        patch_in_registry.operators[0].tl = 111;
        registry.insert(
            0,
            Op505BankFile {
                path: PathBuf::from("dummy.op505"),
                file: Op505PresetFile::Presets {
                    bank: 0,
                    presets: vec![Op505PresetEntry { program: 3, name: "R".to_string(), patch: patch_in_registry }],
                },
            },
        );

        let dir = unique_temp_dir("resolve_patch");
        let mut patch_in_bank = Op505Patch::default();
        patch_in_bank.operators[0].tl = 222;
        std::fs::write(
            dir.join("b.op505"),
            Op505PresetFile::Presets {
                bank: 1,
                presets: vec![Op505PresetEntry { program: 4, name: "B".to_string(), patch: patch_in_bank }],
            }
            .to_json()
            .unwrap(),
        )
        .unwrap();
        let bank_state = Op505PresetBank::load_from_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(resolve_patch(&registry, &bank_state, 0, 3).unwrap().operators[0].tl, 111, "レジストリ優先");
        assert_eq!(resolve_patch(&registry, &bank_state, 1, 4).unwrap().operators[0].tl, 222, "レジストリに無ければbank_state");
        assert!(resolve_patch(&registry, &bank_state, 9, 9).is_none(), "どちらにも無ければNone");
    }
}
