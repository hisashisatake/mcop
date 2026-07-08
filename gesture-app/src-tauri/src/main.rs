#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ym38x6_dto;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri_plugin_dialog::DialogExt;
use ym38x6_core::{pitch_depth_cents, presets_dir, volume_depth, AudioProcessor, ChorusType,
    MasterEffects, PresetBank, PresetEntry, PresetFile, ReverbType,
    Vco, Ym38x6Engine, Ym38x6LfoDestination};
use ym38x6_dto::{LoadedPatchDto, PresetEntryDto, SavedFileDto, Ym38x6PatchDto};

/// 指定チャンネルIDへキーオンする。チャンネルIDは呼び出し側（フロントエンド）が
/// 安定したスロット番号として供給する。発音中/リリース中のチャンネルが既にあっても、
/// エンベロープを即座にカットしてAttackから再開する（実機Key-On挙動に準拠＝同音チョーク）。
/// 押し直し時に同じスロットIDを渡すことで、直前のリリーステールがチョークされる。
/// 音色は`ym38x6_set_program`/`ym38x6_set_patch`で設定した保存済みパッチ（`current_patch`）を使う。
#[tauri::command]
fn note_on(
    engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>,
    channel: usize,
    frequency: f32,
    velocity: u8,
) {
    // 音色は`ym38x6_set_patch`で設定済みのカレントパッチを使う（`Vco::note_on`）。
    engine.lock().unwrap().note_on(channel, frequency, velocity);
}

#[tauri::command]
fn note_off(engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>, channel: usize) {
    engine.lock().unwrap().note_off(channel);
}

/// 以降のNote-Onで使われるカレントパッチを設定する。
#[tauri::command]
fn ym38x6_set_patch(engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>, patch: Ym38x6PatchDto) {
    engine.lock().unwrap().set_patch(patch.into());
}

/// (bank, program)に対応するプリセットへ切り替える。ym38x6-vstのProgramパラメーターと
/// 同じ解決順序（レジストリ→フォールバック）を使うため、音はVSTと基本的に同一になる
/// （レジストリはgesture-appセッション中のOpen/Save/Save Asで更新されるため、VST起動時の
/// 状態からは変わりうる）。波形メモリ音色は`bank == WAVEFORM_MEMORY_BANK`でprogramを
/// 波形スロットとして選ぶ。ディスクI/Oは行わない（`resolve_patch`参照）。
#[tauri::command]
fn ym38x6_set_program(
    engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>,
    registry: tauri::State<'_, Mutex<BankRegistry>>,
    fallback: tauri::State<'_, PresetBank>,
    bank: u16,
    program: u8,
) {
    let patch = resolve_patch(&registry.lock().unwrap(), &fallback, bank, program);
    engine.lock().unwrap().set_patch(patch);
}

/// 今開いている（＝`registry`に登録済みの）bankのファイルが持つ音色一覧を返す
/// （未登録なら空）。PRESETSパネルは「presets_dir全体のブラウザ」ではなく
/// 「今のbankの担当ファイルの中身」を表示する。
#[tauri::command]
fn list_bank_entries(registry: tauri::State<'_, Mutex<BankRegistry>>, bank: u16) -> Vec<PresetEntryDto> {
    let reg = registry.lock().unwrap();
    reg.get(&bank)
        .map(|bank_file| {
            preset_entries(&bank_file.file).iter().map(|e| PresetEntryDto { bank, program: e.program, name: e.name.clone() }).collect()
        })
        .unwrap_or_default()
}

/// (bank, program)のプリセット内容を返す（エンジンへは反映しない読み取り専用）。
/// 音色エディタがプリセット選択時に自身のローカル状態を同期するために使う
/// （`ym38x6_set_patch`で送り返すことで結果的にエンジンへも反映される）。
/// Bank/Programスピンの変更・PRESETSリストのクリックの両方がこれを呼ぶ
/// （レジストリを引くだけで、ディスクの再検索は行わない。レジストリ自体がOpen/Save/Save Asで
/// 正しく更新されているため、常にこれだけで一貫した結果になる）。
#[tauri::command]
fn get_bank_program(
    registry: tauri::State<'_, Mutex<BankRegistry>>,
    fallback: tauri::State<'_, PresetBank>,
    bank: u16,
    program: u8,
) -> LoadedPatchDto {
    let reg = registry.lock().unwrap();
    if let Some(bank_file) = reg.get(&bank) {
        if let Some(entry) = preset_entries(&bank_file.file).iter().find(|e| e.program == program) {
            let file_name = bank_file.path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string());
            return LoadedPatchDto { patch: entry.patch.into(), patch_name: entry.name.clone(), file_name, bank, program };
        }
    }
    let patch = fallback.patch_for_program(bank, program);
    LoadedPatchDto { patch: patch.into(), patch_name: String::new(), file_name: None, bank, program }
}

/// `ym38x6_set_program`/`get_bank_program`が共有する解決ロジック（レジストリ→フォールバック）。
fn resolve_patch(
    registry: &BankRegistry,
    fallback: &PresetBank,
    bank: u16,
    program: u8,
) -> ym38x6_core::Ym38x6Patch {
    registry
        .get(&bank)
        .and_then(|bank_file| preset_entries(&bank_file.file).iter().find(|e| e.program == program).map(|e| e.patch))
        .unwrap_or_else(|| fallback.patch_for_program(bank, program))
}

/// 38x6エンジンのパフォーマンスLFOを設定する。
/// `destination`: 0=Pitch（ビブラート） / 1=Volume（トレモロ） / 2=TL（キャリア一括、38x6拡張）
/// `cc77`/`cc1`/`mod_depth_range`は仕様の実効Depth計算式（CC77/CC1/RPN0,5）への入力。
/// 波形/Fade/Offset（`ChannelParams.perf_lfo_shape`）はTauri IPC未配線（フェーズ7次ステップ、
/// ym38x6-vstのNRPN配線を参照）。
#[tauri::command]
fn ym38x6_set_performance_lfo(
    engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>,
    channel: usize,
    rate: u8,
    delay: u8,
    destination: u8,
    cc77: u8,
    cc1: u8,
    mod_depth_range: u8,
) {
    let destination = match destination {
        1 => Ym38x6LfoDestination::Volume,
        2 => Ym38x6LfoDestination::TlCarrier,
        _ => Ym38x6LfoDestination::Pitch,
    };
    let depth = match destination {
        Ym38x6LfoDestination::Pitch => pitch_depth_cents(cc77, cc1, mod_depth_range),
        Ym38x6LfoDestination::Volume | Ym38x6LfoDestination::TlCarrier => volume_depth(cc77, cc1),
    };
    engine.lock().unwrap().set_performance_lfo(channel, rate, delay, destination, depth);
}

/// マスターエフェクト（Reverb/Chorus）を設定する。
/// `reverb_type`/`chorus_type`は0〜7（spec.md マスターエフェクトセクションのenum参照）
#[tauri::command]
fn set_master_effects(
    effects: tauri::State<'_, Arc<Mutex<MasterEffects>>>,
    reverb_send: u8,
    reverb_type: u8,
    reverb_time: u8,
    chorus_send: u8,
    chorus_type: u8,
    chorus_mod_rate: u8,
    chorus_mod_depth: u8,
    chorus_feedback: u8,
    chorus_send_to_reverb: u8,
) {
    let mut fx = effects.lock().unwrap();
    fx.set_reverb_send(reverb_send);
    fx.set_reverb_type(ReverbType::from_u8(reverb_type));
    fx.set_reverb_time(reverb_time);
    fx.set_chorus_send(chorus_send);
    fx.set_chorus_type(ChorusType::from_u8(chorus_type));
    fx.set_chorus_mod_rate(chorus_mod_rate);
    fx.set_chorus_mod_depth(chorus_mod_depth);
    fx.set_chorus_feedback(chorus_feedback);
    fx.set_chorus_send_to_reverb(chorus_send_to_reverb);
}

/// bank番号ごとの「担当ファイル」（presets_dir全体の中で、そのbankを最後に定義したファイル）。
/// Open/Save/Save Asが直接更新する（音色エディタのファイル状態はpresets_dirの外/中を区別しない）。
struct BankFile {
    path: PathBuf,
    file: PresetFile,
}
type BankRegistry = HashMap<u16, BankFile>;

/// `dir`内の`.38x6`ファイルをファイル名昇順で走査し、bank番号ごとに「最後に処理したファイル」を
/// レジストリへ記録する（`PresetBank::load_from_dir`と同じ優先順位。Presetsは全リセット、
/// Programsは差分マージという単位ではなく、ファイル単位で「最後に触れたファイル」を丸ごと覚える
/// 簡略版。1バンク=1ファイルという運用を前提にしている）。起動時に1回だけ呼ぶ。
fn build_registry(dir: &Path) -> BankRegistry {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("38x6"))
        .collect();
    paths.sort();

    let mut registry = HashMap::new();
    for path in paths {
        let Ok(json) = std::fs::read_to_string(&path) else { continue };
        let Ok(file) = PresetFile::from_json(&json) else { continue };
        registry.insert(preset_bank_of(&file), BankFile { path, file });
    }
    registry
}

/// `PresetFile`のPresets/Programsどちらのvariantでもエントリ一覧への参照を取り出す。
fn preset_entries(file: &PresetFile) -> &Vec<PresetEntry> {
    match file {
        PresetFile::Presets { presets, .. } => presets,
        PresetFile::Programs { programs, .. } => programs,
    }
}

/// `preset_entries`の可変参照版。
fn preset_entries_mut(file: &mut PresetFile) -> &mut Vec<PresetEntry> {
    match file {
        PresetFile::Presets { presets, .. } => presets,
        PresetFile::Programs { programs, .. } => programs,
    }
}

/// `PresetFile`のPresets/Programsどちらのvariantでもbank番号を取り出す。
fn preset_bank_of(file: &PresetFile) -> u16 {
    match file {
        PresetFile::Presets { bank, .. } | PresetFile::Programs { bank, .. } => *bank,
    }
}

/// Open/Save Asのネイティブダイアログの初期ディレクトリを決める。指定bankがレジストリに
/// 登録済みならその親ディレクトリ、未登録（起動直後にそのbankへまだ触れていない等）なら
/// `presets_dir()`。presets_dir内/外を区別する特別扱いはしない（単に「今のbankのファイルの場所」）。
fn current_open_dir(registry: &BankRegistry, bank: u16) -> PathBuf {
    registry.get(&bank).and_then(|bank_file| bank_file.path.parent().map(PathBuf::from)).unwrap_or_else(presets_dir)
}

/// ネイティブOpenダイアログで`.38x6`ファイルを選び、全音色を読み込む。
/// ファイル自身が宣言しているbank番号は無視し、**今エディタで選択中のbank**へ
/// そのファイルの全音色を丸ごとロードする（＝レジストリにはそのbank番号で登録し、
/// `PresetFile`内のbankフィールドも選択中のbankへ書き換える。ユーザー確認済みの仕様）。
/// 先頭エントリを画面へ反映する（複数エントリを持つバンクファイルでも先頭のみを対象とする。
/// 単一パッチの手動調整が主目的のため）。
/// エンジンへの反映はフロントエンド側の既存dirty経路に任せ、このコマンドはengineに触らない。
#[tauri::command]
async fn open_patch_file(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Mutex<BankRegistry>>,
    bank: u16,
) -> Result<Option<LoadedPatchDto>, String> {
    let start_dir = current_open_dir(&registry.lock().unwrap(), bank);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().add_filter("38x6", &["38x6"]).set_directory(start_dir).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;

    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file = with_bank(PresetFile::from_json(&json).map_err(|e| e.to_string())?, bank);
    let entry = preset_entries(&file).first().ok_or("ファイルに音色が含まれていません")?.clone();
    let file_name = path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string());
    let dto =
        LoadedPatchDto { patch: entry.patch.into(), patch_name: entry.name, file_name, bank, program: entry.program };

    registry.lock().unwrap().insert(bank, BankFile { path, file });
    Ok(Some(dto))
}

/// `file`のbankフィールドを`bank`へ書き換えたものを返す（variant・エントリー内容はそのまま）。
fn with_bank(file: PresetFile, bank: u16) -> PresetFile {
    match file {
        PresetFile::Presets { presets, .. } => PresetFile::Presets { bank, presets },
        PresetFile::Programs { programs, .. } => PresetFile::Programs { bank, programs },
    }
}

/// 現在のbankの担当ファイルへ上書き保存する（未登録ならエラー）。
/// `patch_name`は音色エディタの名前入力欄の内容で、保存の都度エントリ名を更新する
/// （＝Save時に名前を変更したい場合はここで反映される）。
#[tauri::command]
fn save_patch_overwrite(
    registry: tauri::State<'_, Mutex<BankRegistry>>,
    bank: u16,
    program: u8,
    patch: Ym38x6PatchDto,
    patch_name: String,
) -> Result<SavedFileDto, String> {
    let mut reg = registry.lock().unwrap();
    let bank_file = reg.get_mut(&bank).ok_or("このbankにはまだファイルがありません（先にOpenかSave Asしてください）")?;
    let entries = preset_entries_mut(&mut bank_file.file);
    let entry = entries.iter_mut().find(|e| e.program == program).ok_or("保存先エントリが見つかりません")?;
    entry.patch = patch.into();
    entry.name = patch_name.clone();
    let file_name = bank_file.path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
    let json = bank_file.file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&bank_file.path, json).map_err(|e| e.to_string())?;
    Ok(SavedFileDto { patch_name, file_name, bank, program })
}

/// ネイティブSaveダイアログで保存先を選び、新規`.38x6`ファイルとして書き出す。`Presets`形式は
/// 「そのbankを丸ごと定義する」形式のため、今編集中の1音色だけを書くと、presets_dirで読み込んだ際に
/// 同bankの他の全プログラムが失われてしまう。そのため、**今のbankの担当ファイル（レジストリ）**の
/// 全エントリーを複製元とし、今編集中のprogramだけ最新の内容に差し替えて丸ごと書き出す
/// （presets_dir全体の再検索はしない。何も登録されていない真っさらな状態なら、今の1音色のみになる）。
/// 音色名（`PresetEntry.name`）は名前入力欄の`patch_name`をそのまま使う。
/// `default_file_name`はSaveダイアログの提案ファイル名にのみ使う。
/// 保存後は**そのbankの担当ファイルがこの新しいファイルに置き換わる**
/// （以後の`save_patch_overwrite`はこの新しいファイルに対して行われる）。
#[tauri::command]
async fn save_patch_as(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Mutex<BankRegistry>>,
    patch: Ym38x6PatchDto,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
) -> Result<Option<SavedFileDto>, String> {
    let start_dir = current_open_dir(&registry.lock().unwrap(), bank);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("38x6", &["38x6"])
            .set_directory(start_dir)
            .set_file_name(format!("{default_file_name}.38x6"))
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;

    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("patch.38x6").to_string();
    let mut presets: Vec<PresetEntry> =
        registry.lock().unwrap().get(&bank).map(|bank_file| preset_entries(&bank_file.file).clone()).unwrap_or_default();
    match presets.iter_mut().find(|e| e.program == program) {
        Some(entry) => {
            entry.patch = patch.into();
            entry.name = patch_name.clone();
        }
        None => presets.push(PresetEntry { program, name: patch_name.clone(), patch: patch.into() }),
    }
    presets.sort_by_key(|e| e.program);
    let file = PresetFile::Presets { bank, presets };
    let json = file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;

    registry.lock().unwrap().insert(bank, BankFile { path, file });
    Ok(Some(SavedFileDto { patch_name, file_name, bank, program }))
}

fn main() {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no output device available");
    let supported = device
        .default_output_config()
        .expect("no default output config");

    let num_channels = supported.channels() as usize;
    let sample_rate = supported.sample_rate().0 as f32;
    let stream_config: cpal::StreamConfig = supported.into();

    let engine = Arc::new(Mutex::new(Ym38x6Engine::new(sample_rate)));
    let engine_audio = Arc::clone(&engine);
    let effects = Arc::new(Mutex::new(MasterEffects::new(sample_rate)));
    let effects_audio = Arc::clone(&effects);

    // presets_dir()の読み込みは起動時にここで1回だけ行う。
    // - fallback: 波形メモリ/GM2/プレースホルダーのフォールバックチェーン専用（読み取り専用、以後更新しない）。
    // - registry: bank番号ごとの担当ファイル。Open/Save/Save Asがセッション中に直接更新する。
    let fallback = PresetBank::load_from_dir(&presets_dir());
    let registry = build_registry(&presets_dir());

    let stream = device
        .build_output_stream::<f32, _, _>(
            &stream_config,
            move |output: &mut [f32], _| {
                output.fill(0.0);
                if let Ok(mut eng) = engine_audio.try_lock() {
                    eng.render(output, num_channels);
                }
                if let Ok(mut fx) = effects_audio.try_lock() {
                    fx.process(output, num_channels);
                }
            },
            |err| eprintln!("audio error: {err}"),
            None,
        )
        .expect("failed to build output stream");

    stream.play().expect("failed to start audio stream");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(engine)
        .manage(effects)
        .manage(fallback)
        .manage(Mutex::new(registry))
        .invoke_handler(tauri::generate_handler![
            note_on,
            note_off,
            set_master_effects,
            ym38x6_set_patch,
            ym38x6_set_program,
            ym38x6_set_performance_lfo,
            list_bank_entries,
            get_bank_program,
            open_patch_file,
            save_patch_overwrite,
            save_patch_as,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    drop(stream);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ym38x6_core::Ym38x6Patch;

    /// テスト用に一意な一時ディレクトリを作り直す（既存があれば削除してから作成）。
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ym38x6_gesture_app_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_json(bank: u16, program: u8, name: &str) -> String {
        PresetFile::Programs {
            bank,
            programs: vec![PresetEntry { program, name: name.to_string(), patch: Ym38x6Patch::default() }],
        }
        .to_json()
        .unwrap()
    }

    #[test]
    fn build_registry_maps_bank_to_its_file() {
        let dir = unique_temp_dir("registry_basic");
        std::fs::write(dir.join("a.38x6"), sample_json(0, 5, "Foo")).unwrap();
        std::fs::write(dir.join("b.38x6"), sample_json(1, 2, "Bar")).unwrap();

        let registry = build_registry(&dir);
        assert_eq!(registry.get(&0).unwrap().path, dir.join("a.38x6"));
        assert_eq!(registry.get(&1).unwrap().path, dir.join("b.38x6"));
        assert!(registry.get(&2).is_none(), "存在しないbankは登録されない");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_registry_prefers_last_file_in_sorted_order() {
        // PresetBank::load_from_dirと同じ「ファイル名昇順で後読みが勝つ」優先順位を確認する。
        let dir = unique_temp_dir("registry_precedence");
        std::fs::write(dir.join("a_first.38x6"), sample_json(0, 0, "First")).unwrap();
        std::fs::write(dir.join("b_second.38x6"), sample_json(0, 0, "Second")).unwrap();

        let registry = build_registry(&dir);
        assert_eq!(registry.get(&0).unwrap().path, dir.join("b_second.38x6"), "後読みのファイルが優先されるべき");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn with_bank_overrides_bank_and_keeps_entries() {
        // 回帰テスト: Openしたファイルが宣言するbank番号は無視し、今エディタで選択中のbankへ
        // 全音色をロードする仕様（ファイル自身のbankをそのまま採用するのは誤りだった）。
        let presets = PresetFile::Presets {
            bank: 3,
            presets: vec![PresetEntry { program: 5, name: "Foo".to_string(), patch: Ym38x6Patch::default() }],
        };
        let rebanked = with_bank(presets, 9);
        assert_eq!(preset_bank_of(&rebanked), 9);
        assert_eq!(preset_entries(&rebanked).len(), 1);
        assert_eq!(preset_entries(&rebanked)[0].program, 5);

        let programs = PresetFile::Programs { bank: 3, programs: vec![] };
        assert_eq!(preset_bank_of(&with_bank(programs, 9)), 9);
    }

    #[test]
    fn preset_bank_of_reads_bank_from_either_variant() {
        let presets = PresetFile::Presets { bank: 7, presets: vec![] };
        let programs = PresetFile::Programs { bank: 9, programs: vec![] };
        assert_eq!(preset_bank_of(&presets), 7);
        assert_eq!(preset_bank_of(&programs), 9);
    }

    #[test]
    fn current_open_dir_falls_back_to_presets_dir_when_bank_unregistered() {
        let registry = BankRegistry::new();
        assert_eq!(current_open_dir(&registry, 0), presets_dir());
    }
}
