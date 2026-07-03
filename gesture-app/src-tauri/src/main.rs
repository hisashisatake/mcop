#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ym38x6_dto;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri_plugin_dialog::DialogExt;
use ym38x6_core::{pitch_depth_cents, presets_dir, volume_depth, AdsrParams, ChorusType,
    LfoWaveform, MasterEffects, PresetBank, PresetEntry, PresetFile, ReverbType, SoundEngine,
    Ym38x6Engine, Ym38x6LfoDestination};
use ym38x6_dto::{LoadedPatchDto, PresetEntryDto, SavedFileDto, Ym38x6PatchDto};

/// 指定チャンネルIDへキーオンする。チャンネルIDは呼び出し側（フロントエンド）が
/// 安定したスロット番号として供給する。発音中/リリース中のチャンネルが既にあっても、
/// エンベロープを即座にカットしてAttackから再開する（実機Key-On挙動に準拠＝同音チョーク）。
/// 押し直し時に同じスロットIDを渡すことで、直前のリリーステールがチョークされる。
/// 音色は`ym38x6_set_program`/`ym38x6_set_patch`で設定したcurrent_patchを使う
/// （`wave_slot`はトレイト互換のため受け取るが未使用）。
#[tauri::command]
fn note_on(
    engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>,
    channel: usize,
    wave_slot: u8,
    frequency: f32,
) {
    engine.lock().unwrap().note_on(channel, wave_slot, frequency, AdsrParams::default());
}

#[tauri::command]
fn note_off(engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>, channel: usize) {
    engine.lock().unwrap().note_off(channel);
}

/// 38x6エンジンで指定チャンネルIDへNote-Onする。`patch`は4オペレーター分のパラメーターと
/// チャンネルパラメーター一式。チャンネルIDの扱い（同音チョーク）は`note_on`と同じ。
#[tauri::command]
fn ym38x6_note_on(
    engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>,
    channel: usize,
    frequency: f32,
    velocity: u8,
    patch: Ym38x6PatchDto,
) {
    engine.lock().unwrap().note_on_with_velocity(channel, frequency, velocity, patch.into());
}

#[tauri::command]
fn ym38x6_note_off(engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>, channel: usize) {
    engine.lock().unwrap().note_off(channel);
}

/// 以降のNote-Onで使われるカレントパッチを設定する。
#[tauri::command]
fn ym38x6_set_patch(engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>, patch: Ym38x6PatchDto) {
    engine.lock().unwrap().set_patch(patch.into());
}

/// (bank, program)に対応するプリセットへ切り替える。ym38x6-vstのProgramパラメーターと
/// 同じ`PresetBank::patch_for_program`を使うため、音はVSTと完全に同一になる。
/// 波形メモリ音色は`bank == WAVEFORM_MEMORY_BANK`でprogramを波形スロットとして選ぶ。
#[tauri::command]
fn ym38x6_set_program(
    engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>,
    preset_bank: tauri::State<'_, PresetBank>,
    bank: u16,
    program: u8,
) {
    let patch = preset_bank.patch_for_program(bank, program);
    engine.lock().unwrap().set_patch(patch);
}

/// プリセット一覧を返す（ym38x6-vstのPRESETSサイドバーと同じ`presets_dir()`から
/// 読み込んだ`PresetBank`をソート済みで返す。フロントエンドの音色エディタが一覧表示に使う）。
#[tauri::command]
fn list_presets(preset_bank: tauri::State<'_, PresetBank>) -> Vec<PresetEntryDto> {
    preset_bank.sorted_entries().into_iter().map(PresetEntryDto::from).collect()
}

/// (bank, program)のプリセット内容を返す（エンジンへは反映しない読み取り専用）。
/// 音色エディタがプリセット選択時に自身のローカル状態を同期するために使う
/// （`ym38x6_set_patch`で送り返すことで結果的にエンジンへも反映される）。
/// 併せて`open_state`をこのプリセットの実ファイルに合わせて更新する。これにより
/// PRESETSパネルから選んだプリセットも（Open/Save Asと同じく）そのまま上書き保存できる
/// （presets_dir内のファイルをOpenダイアログを介さず直接選んだだけ、という扱い）。
#[tauri::command]
fn get_preset_patch(
    preset_bank: tauri::State<'_, PresetBank>,
    open_state: tauri::State<'_, Mutex<Option<OpenPatchFile>>>,
    bank: u16,
    program: u8,
) -> LoadedPatchDto {
    let located = locate_preset_file(&presets_dir(), bank, program);
    let (patch, patch_name, file_name) = match &located {
        Some(open) => {
            let entry = preset_entries(&open.file).get(open.entry_index);
            let patch = entry.map(|e| e.patch).unwrap_or_default();
            let patch_name = entry.map(|e| e.name.clone()).unwrap_or_default();
            let file_name = open.path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string());
            (patch, patch_name, file_name)
        }
        None => (preset_bank.patch_for_program(bank, program), String::new(), None),
    };
    *open_state.lock().unwrap() = located;
    LoadedPatchDto { patch: patch.into(), patch_name, file_name, bank, program }
}

/// 38x6エンジンのパフォーマンスLFOを設定する。
/// `waveform`: 0=Triangle / 1=Sine / 2=Square / 3=S&H（Performance LFO Waveform enum準拠）
/// `destination`: 0=Pitch（ビブラート） / 1=Volume（トレモロ） / 2=TL（キャリア一括、38x6拡張）
/// `cc77`/`cc1`/`mod_depth_range`は仕様の実効Depth計算式（CC77/CC1/RPN0,5）への入力。
#[tauri::command]
fn ym38x6_set_performance_lfo(
    engine: tauri::State<'_, Arc<Mutex<Ym38x6Engine>>>,
    channel: usize,
    rate: u8,
    delay: u8,
    waveform: u8,
    destination: u8,
    cc77: u8,
    cc1: u8,
    mod_depth_range: u8,
) {
    let waveform = match waveform {
        1 => LfoWaveform::Sine,
        2 => LfoWaveform::Square,
        3 => LfoWaveform::SampleHold,
        _ => LfoWaveform::Triangle,
    };
    let destination = match destination {
        1 => Ym38x6LfoDestination::Volume,
        2 => Ym38x6LfoDestination::TlCarrier,
        _ => Ym38x6LfoDestination::Pitch,
    };
    let depth = match destination {
        Ym38x6LfoDestination::Pitch => pitch_depth_cents(cc77, cc1, mod_depth_range),
        Ym38x6LfoDestination::Volume | Ym38x6LfoDestination::TlCarrier => volume_depth(cc77, cc1),
    };
    engine.lock().unwrap().set_performance_lfo(channel, rate, delay, waveform, destination, depth);
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

/// 現在Openされている`.38x6`ファイルの状態（presets_dirのプリセットとは別系統、
/// 音色エディタのOpen/Save/Save Asが任意パスに対して操作する対象）。
/// 上書き保存時は同じファイル内の他エントリを保ったまま、編集対象エントリだけを差し替えて書き戻す。
struct OpenPatchFile {
    path: PathBuf,
    file: PresetFile,
    entry_index: usize,
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

/// Open/Save Asのネイティブダイアログの初期ディレクトリを決める。現在Openしているファイルが
/// あればその親ディレクトリ、無ければ`presets_dir()`（何も開いていない起動直後はここが自然な
/// デフォルト）。presets_dir内/外を区別する特別扱いはしない（単に「今開いているファイルの場所」）。
fn current_open_dir(open_state: &Mutex<Option<OpenPatchFile>>) -> PathBuf {
    open_state
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|open| open.path.parent().map(PathBuf::from))
        .unwrap_or_else(presets_dir)
}

/// `dir`内の`.38x6`ファイルを`PresetBank::load_from_dir`と同じ順序（ファイル名昇順）で走査し、
/// (bank, program)を実際に定義しているファイルを`OpenPatchFile`として返す。
/// 複数ファイルが同じ(bank, program)を定義する場合は最後に処理したファイルを採用する
/// （`load_from_dir`が後読みのファイルで上書きするのと同じ優先順位）。
fn locate_preset_file(dir: &Path, bank: u16, program: u8) -> Option<OpenPatchFile> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("38x6"))
        .collect();
    paths.sort();

    let mut found = None;
    for path in paths {
        let Ok(json) = std::fs::read_to_string(&path) else { continue };
        let Ok(file) = PresetFile::from_json(&json) else { continue };
        if preset_bank_of(&file) == bank {
            if let Some(entry_index) = preset_entries(&file).iter().position(|e| e.program == program) {
                found = Some(OpenPatchFile { path, file, entry_index });
            }
        }
    }
    found
}

/// ネイティブOpenダイアログで`.38x6`ファイルを選び、先頭エントリを読み込む
/// （複数エントリを持つバンクファイルでも先頭のみを対象とする。単一パッチの手動調整が主目的のため）。
/// エンジンへの反映はフロントエンド側の既存dirty経路に任せ、このコマンドはengineに触らない。
#[tauri::command]
async fn open_patch_file(
    app: tauri::AppHandle,
    open_state: tauri::State<'_, Mutex<Option<OpenPatchFile>>>,
) -> Result<Option<LoadedPatchDto>, String> {
    let start_dir = current_open_dir(&open_state);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().add_filter("38x6", &["38x6"]).set_directory(start_dir).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;

    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file = PresetFile::from_json(&json).map_err(|e| e.to_string())?;
    let bank = preset_bank_of(&file);
    let entry = preset_entries(&file).first().ok_or("ファイルに音色が含まれていません")?.clone();
    let file_name = path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string());
    let dto =
        LoadedPatchDto { patch: entry.patch.into(), patch_name: entry.name, file_name, bank, program: entry.program };

    *open_state.lock().unwrap() = Some(OpenPatchFile { path, file, entry_index: 0 });
    Ok(Some(dto))
}

/// 現在Openしているファイルへ上書き保存する（`open_patch_file`/`save_patch_as`で
/// 開いた/保存した対象に対してのみ有効。未Openならエラーを返す）。
/// `patch_name`は音色エディタの名前入力欄の内容で、保存の都度エントリ名を更新する
/// （＝Save時に名前を変更したい場合はここで反映される）。
#[tauri::command]
fn save_patch_overwrite(
    open_state: tauri::State<'_, Mutex<Option<OpenPatchFile>>>,
    patch: Ym38x6PatchDto,
    patch_name: String,
) -> Result<SavedFileDto, String> {
    let mut guard = open_state.lock().unwrap();
    let open = guard.as_mut().ok_or("ファイルが開かれていません（先にOpenかSave Asしてください）")?;
    let bank = preset_bank_of(&open.file);
    let entries = preset_entries_mut(&mut open.file);
    let entry = entries.get_mut(open.entry_index).ok_or("保存先エントリが見つかりません")?;
    entry.patch = patch.into();
    entry.name = patch_name.clone();
    let program = entry.program;
    let file_name = open.path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
    let json = open.file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&open.path, json).map_err(|e| e.to_string())?;
    Ok(SavedFileDto { patch_name, file_name, bank, program })
}

/// ネイティブSaveダイアログで保存先を選び、新規`.38x6`ファイル（単一エントリ）として書き出す。
/// 音色名（`PresetEntry.name`）は名前入力欄の`patch_name`をそのまま使う（ファイル名とは独立。
/// 1ファイルに複数音色が入りうる以上、ファイル名と音色名は別概念のため）。
/// `default_file_name`はSaveダイアログの提案ファイル名にのみ使う。
/// 保存後は以後の`save_patch_overwrite`がこの新しいファイルに対して行われる。
#[tauri::command]
async fn save_patch_as(
    app: tauri::AppHandle,
    open_state: tauri::State<'_, Mutex<Option<OpenPatchFile>>>,
    patch: Ym38x6PatchDto,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
) -> Result<Option<SavedFileDto>, String> {
    let start_dir = current_open_dir(&open_state);
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
    let file = PresetFile::Presets {
        bank,
        presets: vec![PresetEntry { program, name: patch_name.clone(), patch: patch.into() }],
    };
    let json = file.to_json().map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;

    *open_state.lock().unwrap() = Some(OpenPatchFile { path, file, entry_index: 0 });
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
    let preset_bank = PresetBank::load_from_dir(&presets_dir());

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
        .manage(preset_bank)
        .manage(Mutex::new(None::<OpenPatchFile>))
        .invoke_handler(tauri::generate_handler![
            note_on,
            note_off,
            set_master_effects,
            ym38x6_note_on,
            ym38x6_note_off,
            ym38x6_set_patch,
            ym38x6_set_program,
            ym38x6_set_performance_lfo,
            list_presets,
            get_preset_patch,
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
    fn locate_preset_file_matches_bank_and_program() {
        let dir = unique_temp_dir("locate_match");
        std::fs::write(dir.join("a.38x6"), sample_json(0, 5, "Foo")).unwrap();

        let found = locate_preset_file(&dir, 0, 5).expect("should locate entry");
        assert_eq!(found.entry_index, 0);
        assert_eq!(found.path, dir.join("a.38x6"));

        assert!(locate_preset_file(&dir, 0, 6).is_none(), "programが一致しなければNone");
        assert!(locate_preset_file(&dir, 1, 5).is_none(), "bankが一致しなければNone");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn locate_preset_file_prefers_last_file_in_sorted_order() {
        // PresetBank::load_from_dirと同じ「ファイル名昇順で後読みが勝つ」優先順位を確認する。
        let dir = unique_temp_dir("locate_precedence");
        std::fs::write(dir.join("a_first.38x6"), sample_json(0, 0, "First")).unwrap();
        std::fs::write(dir.join("b_second.38x6"), sample_json(0, 0, "Second")).unwrap();

        let found = locate_preset_file(&dir, 0, 0).expect("should locate entry");
        assert_eq!(found.path, dir.join("b_second.38x6"), "後読みのファイルが優先されるべき");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preset_bank_of_reads_bank_from_either_variant() {
        let presets = PresetFile::Presets { bank: 7, presets: vec![] };
        let programs = PresetFile::Programs { bank: 9, programs: vec![] };
        assert_eq!(preset_bank_of(&presets), 7);
        assert_eq!(preset_bank_of(&programs), 9);
    }
}
