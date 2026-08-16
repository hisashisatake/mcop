#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engines;
mod op505_presets;
mod ym38x6_dto;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engines::{ActiveEngine, Engines};
use op505_core::{op505_presets_dir, Op505Patch, Op505PresetBank};
use op505_presets::{build_op505_registry, Op505BankRegistry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri_plugin_dialog::DialogExt;
use ym38x6_core::{cc76_to_rate_scale, cutoff_depth, pitch_depth_cents, presets_dir, volume_depth, AudioProcessor,
    BipolarFg, ChorusType, EgParams, MasterEffects, PresetBank, PresetEntry, PresetFile, ReverbType, TextureLfo,
    Ym38x6LfoDestination};
use ym38x6_dto::{LoadedPatchDto, PresetEntryDto, SavedFileDto, Ym38x6PatchDto};

/// 指定チャンネルIDへキーオンする。チャンネルIDは呼び出し側（フロントエンド）が
/// 安定したスロット番号として供給する。発音中/リリース中のチャンネルが既にあっても、
/// エンベロープを即座にカットしてAttackから再開する（実機Key-On挙動に準拠＝同音チョーク）。
/// 押し直し時に同じスロットIDを渡すことで、直前のリリーステールがチョークされる。
/// 音色は`ym38x6_set_program`/`ym38x6_set_patch`で設定した保存済みパッチ（`current_patch`）を使う。
#[tauri::command]
fn note_on(
    engine: tauri::State<'_, Arc<Mutex<Engines>>>,
    channel: usize,
    frequency: f32,
    velocity: u8,
) {
    // 音色は各エンジンの音色コマンドで設定済みのカレントパッチを使う。
    // どちらのエンジンが鳴るかは`set_active_engine`で選択済みのactive側。
    engine.lock().unwrap().note_on(channel, frequency, velocity);
}

#[tauri::command]
fn note_off(engine: tauri::State<'_, Arc<Mutex<Engines>>>, channel: usize) {
    engine.lock().unwrap().note_off(channel);
}

/// 以降のNote-Onで使われるカレントパッチを設定すると同時に、現在発音中の全チャンネルへも
/// 変更内容を即座に反映する（音色エディタのノブ操作向け。VSTのDAWオートメーション/NRPNが
/// 発音中の音にも即座に効くのと同じ扱い）。Bank/Program切り替え（`ym38x6_set_program`）は
/// これとは別に「次のnote-onから適用」のままとする。
#[tauri::command]
fn ym38x6_set_patch(engine: tauri::State<'_, Arc<Mutex<Engines>>>, patch: Ym38x6PatchDto) {
    engine.lock().unwrap().ym38x6.set_patch_live(patch.into());
}

/// (bank, program)に対応するプリセットへ切り替える。ym38x6-vstのProgramパラメーターと
/// 同じ解決順序（レジストリ→フォールバック）を使うため、音はVSTと基本的に同一になる
/// （レジストリはgesture-appセッション中のOpen/Save/Save Asで更新されるため、VST起動時の
/// 状態からは変わりうる）。波形メモリ音色は`bank == WAVEFORM_MEMORY_BANK`でprogramを
/// 波形スロットとして選ぶ。ディスクI/Oは行わない（`resolve_patch`参照）。
#[tauri::command]
fn ym38x6_set_program(
    engine: tauri::State<'_, Arc<Mutex<Engines>>>,
    registry: tauri::State<'_, Mutex<BankRegistry>>,
    fallback: tauri::State<'_, PresetBank>,
    bank: u16,
    program: u8,
) {
    let patch = resolve_patch(&registry.lock().unwrap(), &fallback, bank, program);
    engine.lock().unwrap().ym38x6.set_patch(patch);
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

/// Pitch FGループのAR/D1R基準値（rate_scale=1.0＝CC76無補正のとき、AR≈85ms・D1R≈85msで
/// 往復1周期≈170ms≈5.9Hzのビブラートになる初期案。CLAUDE.mdのテスト方針に従い、
/// 実機で聴いてから調整する）。
const PITCH_FG_VIBRATO_AR: u8 = 136;
const PITCH_FG_VIBRATO_D1R: u8 = 199;

/// 38x6エンジンの演奏系モジュレーション（V キーのPitch⇔Volume切替、既存トグルUXを維持）を設定する。
/// `destination`: 0=Pitch（ビブラート、Pitch FGへ配線） / 1=Volume（トレモロ）
/// / 2=TL（キャリア一括、38x6拡張） / 3=Cutoff（オートワウ、38x6拡張）
/// Volume/TL/Cutoffは従来通り質感LFO（`ChannelParams.texture_lfo`、焼き込み専用でCC補正は
/// 受けない）へ書き込む。Pitchのみ演奏CC（CC1/76/77/78）による補正を受ける唯一のFGスロットである
/// Pitch FG（`ChannelParams.pitch_fg`）へ書き込む（spec-sound.md「演奏層による補正」節）。
/// `rate`/`delay`はmain.jsのC/Bキー・Vキー由来の0〜255値、`cc77`/`cc1`/`mod_depth_range`は
/// destination別の実単位へ変換するための入力（既存の`pitch_depth_cents`/`volume_depth`/
/// `cutoff_depth`を流用）。
///
/// 質感LFO・Pitch FGともステップ6の再編で完全にパッチ所有になり、チャンネル単体のランタイムAPI
/// （`Ym38x6Engine::set_performance_lfo`）は廃止された。そのため`current_patch()`を書き換えて
/// `set_patch_live`で全発音中チャンネル＋以降のnote-onへ反映する（gesture-appは元々「エンジン
/// 全体で1つのカレントパッチ」前提のため、この一本化は既存アーキテクチャと整合する）。
/// Pitch destinationのみ、CC76由来の速さスケールをVSTと同じ`set_pitch_fg_rate_scale`
/// （`channel`引数を使う唯一の経路、AR/D1Rの生値ではなくrate_scale経由でスケールする理由は
/// `cc76_to_rate_scale`のドキュメント参照）で発音中の該当チャンネルへ個別に反映する。
#[tauri::command]
fn ym38x6_set_performance_lfo(
    engine: tauri::State<'_, Arc<Mutex<Engines>>>,
    channel: usize,
    rate: u8,
    delay: u8,
    destination: u8,
    cc77: u8,
    cc1: u8,
    mod_depth_range: u8,
) {
    let dest = Ym38x6LfoDestination::from_u8(destination);
    let mut engines = engine.lock().unwrap();
    let engine = &mut engines.ym38x6;
    let mut patch = engine.current_patch();

    if dest == Ym38x6LfoDestination::Pitch {
        let cents = pitch_depth_cents(cc77, cc1, mod_depth_range);
        let depth = (128.0 + cents / 1200.0 * 128.0).round().clamp(0.0, 255.0) as u8;
        patch.channel.pitch_fg = BipolarFg {
            eg: EgParams {
                ar: PITCH_FG_VIBRATO_AR,
                d1r: PITCH_FG_VIBRATO_D1R,
                delay,
                floor: 0,
                loop_enabled: 1,
                curve: 1,
                ..patch.channel.pitch_fg.eg
            },
            depth,
        };
        engine.set_patch_live(patch);
        // CC76(Vibrato Rate)相当をrate_scaleへ変換して発音中の該当チャンネルへ即時反映する
        // （AR/D1Rの生値0〜255をmain.jsのC/Bキー(rate)域からCC76の0〜127域へ線形換算）。
        let cc76 = (rate as u32 * 127 / 255) as u8;
        engine.set_pitch_fg_rate_scale(channel, cc76_to_rate_scale(cc76));
        return;
    }

    // dest == Pitchは上のearly returnで処理済み。
    let depth = match dest {
        Ym38x6LfoDestination::Volume | Ym38x6LfoDestination::TlCarrier => volume_depth(cc77, cc1) * 255.0,
        Ym38x6LfoDestination::Cutoff => cutoff_depth(cc77, cc1),
        Ym38x6LfoDestination::Pitch | Ym38x6LfoDestination::Unplugged => 0.0,
    }
    .round()
    .clamp(0.0, 255.0) as u8;

    patch.channel.texture_lfo = TextureLfo { rate, delay, destination, depth, ..patch.channel.texture_lfo };
    engine.set_patch_live(patch);
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

// ---------------------------------------------------------------------------
// OP505（エンジン切替 + 音色コマンド）
// ---------------------------------------------------------------------------

/// 演奏入力（note_on）を受けるエンジンを切り替える（0=OP505 / 1=38x6）。
/// 非アクティブ側のリリース尾はrenderで鳴り続けるため、切替は無音を挟まない。
#[tauri::command]
fn set_active_engine(engine: tauri::State<'_, Arc<Mutex<Engines>>>, engine_id: u8) {
    engine.lock().unwrap().active = ActiveEngine::from_u8(engine_id);
}

#[tauri::command]
fn get_active_engine(engine: tauri::State<'_, Arc<Mutex<Engines>>>) -> u8 {
    engine.lock().unwrap().active.to_u8()
}

/// OP505のカレントパッチを設定し、発音中のチャンネルへも即座に反映する
/// （`ym38x6_set_patch`のOP505版。音色エディタのノブ操作向け）。
/// `Op505Patch`は専用DTOを介さず直接シリアライズする（op505-coreの型が既にserde対応で、
/// `.38x6`のような後方互換の負債も無いため。フィールド名の安定性は
/// `op505_patch_json_keys_are_stable`テストで担保する）。
#[tauri::command]
fn op505_set_patch(engine: tauri::State<'_, Arc<Mutex<Engines>>>, patch: Op505Patch) {
    engine.lock().unwrap().op505.set_patch_live(patch);
}

/// OP505エンジンの現在のカレントパッチを読み取る（読み取り専用、エンジンへは反映しない）。
/// 音色エディタ起動時、main.js側のデモ選択/Bank・Program変換で既に設定済みのパッチを
/// エディタのローカル状態へ同期するために使う（`get_bank_program`の38x6版に相当）。
#[tauri::command]
fn op505_get_current_patch(engine: tauri::State<'_, Arc<Mutex<Engines>>>) -> Op505Patch {
    engine.lock().unwrap().op505.current_patch()
}

/// (bank, program)に対応する`.op505`プリセットをカレントパッチに設定する
/// （次のnote-onから適用。`ym38x6_set_program`のOP505版）。解決順位はレジストリ→`Op505PresetBank`
/// （`op505_presets::resolve_patch`参照）。`.38x6`側と違い波形メモリ/GM2/プレースホルダーの
/// フォールバックチェーンを持たない。見つからなければエンジンには触れず`None`を返す
/// （`Op505Patch::default()`はtl=0で無音のため、黙って無音へ切り替えるより「見つからない」を
/// 呼び出し側に伝えて現在の音を維持するほうが安全）。
/// 既存`.38x6`をOP505で鳴らしたい場合は`op505_probe --convert-bank`で`.op505`へ変換してから
/// `op505_presets_dir()`へ置く（Adapterでのその場変換は廃止。EGの近似変換警告が出る変換は
/// 「一度変換して確認する」明示的な手順にすべき、という判断）。
#[tauri::command]
fn op505_set_program(
    engine: tauri::State<'_, Arc<Mutex<Engines>>>,
    registry: tauri::State<'_, Mutex<Op505BankRegistry>>,
    bank_state: tauri::State<'_, Mutex<Op505PresetBank>>,
    bank: u16,
    program: u8,
) -> Option<Op505Patch> {
    let patch = op505_presets::resolve_patch(&registry.lock().unwrap(), &bank_state.lock().unwrap(), bank, program)?;
    engine.lock().unwrap().op505.set_patch(patch);
    Some(patch)
}

/// `op505_presets_dir()`から`.op505`プリセットを読み直し、読み込んだプリセット総数を返す。
/// アプリ起動中に外部（op505_probe/opz2op505等の変換ツールや手編集）でプリセットファイルが
/// 追加・更新された場合に、再起動せず反映するためのコマンド。`Op505PresetBank`（フォールバック用）
/// と`Op505BankRegistry`（Open/Save/Save Asの担当ファイル管理）の両方を作り直す
/// （セッション中のOpen/Save/Save Asによる担当ファイルの変更はこの操作で失われる）。
#[tauri::command]
fn op505_reload_presets(bank_state: tauri::State<'_, Mutex<Op505PresetBank>>, registry: tauri::State<'_, Mutex<Op505BankRegistry>>) -> usize {
    let dir = op505_presets_dir();
    let reloaded = Op505PresetBank::load_from_dir(&dir);
    let count = reloaded.sorted_entries().len();
    *bank_state.lock().unwrap() = reloaded;
    *registry.lock().unwrap() = build_op505_registry(&dir);
    count
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

    // 38x6/OP505の両エンジンを常時保持する共存ラッパー（`engines.rs`参照）。
    let engine = Arc::new(Mutex::new(Engines::new(sample_rate)));
    let engine_audio = Arc::clone(&engine);
    let effects = Arc::new(Mutex::new(MasterEffects::new(sample_rate)));
    let effects_audio = Arc::clone(&effects);

    // presets_dir()の読み込みは起動時にここで1回だけ行う。
    // - fallback: 波形メモリ/GM2/プレースホルダーのフォールバックチェーン専用（読み取り専用、以後更新しない）。
    // - registry: bank番号ごとの担当ファイル。Open/Save/Save Asがセッション中に直接更新する。
    let fallback = PresetBank::load_from_dir(&presets_dir());
    let registry = build_registry(&presets_dir());
    // op505_presets_dir()はym38x6のpresets_dir()とは別ディレクトリ（%APPDATA%\op505\presets）。
    // - op505_bank: 波形メモリ/GM2に相当するフォールバックの無い読み取り専用集合（起動時1回）。
    // - op505_registry: bank番号ごとの担当ファイル。ym38x6のregistry同様、Open/Save/Save Asが
    //   セッション中に直接更新する。
    let op505_bank = Op505PresetBank::load_from_dir(&op505_presets_dir());
    let op505_registry = build_op505_registry(&op505_presets_dir());

    let stream = device
        .build_output_stream::<f32, _, _>(
            &stream_config,
            move |output: &mut [f32], _| {
                output.fill(0.0);
                if let Ok(mut eng) = engine_audio.try_lock() {
                    // Engines::render（両エンジン加算。非アクティブ側のリリース尾も鳴り続ける）。
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
        .manage(Mutex::new(op505_bank))
        .manage(Mutex::new(op505_registry))
        .invoke_handler(tauri::generate_handler![
            note_on,
            note_off,
            set_master_effects,
            ym38x6_set_patch,
            ym38x6_set_program,
            ym38x6_set_performance_lfo,
            set_active_engine,
            get_active_engine,
            op505_set_patch,
            op505_get_current_patch,
            op505_set_program,
            op505_reload_presets,
            list_bank_entries,
            get_bank_program,
            open_patch_file,
            save_patch_overwrite,
            save_patch_as,
            op505_presets::op505_list_bank_entries,
            op505_presets::op505_get_bank_program,
            op505_presets::op505_open_patch_file,
            op505_presets::op505_save_patch_overwrite,
            op505_presets::op505_save_patch_as,
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

    /// `Op505Patch`は専用DTOを介さず直接シリアライズしてIPCへ流すため、op505-core側の
    /// フィールド名がそのままワイヤーフォーマットになる。フィールドがリネーム・削除されると
    /// editor-wasm側とのIPCが「エラーも出さずに」壊れるので、キー構成をここで固定して検出する。
    #[test]
    fn op505_patch_json_keys_are_stable() {
        let v = serde_json::to_value(Op505Patch::default()).unwrap();
        assert!(v.get("operators").is_some() && v.get("channel").is_some());

        let ch = &v["channel"];
        for k in [
            "algorithm", "feedback", "chip_lfo_freq", "chip_lfo_pmd", "chip_lfo_amd", "chip_lfo_delay",
            "pms", "ams", "filter_cutoff", "filter_resonance", "filter_type", "filter_self_oscillation",
            "pitch_fg", "cutoff_fg", "gain_fg", "texture_lfo",
        ] {
            assert!(ch.get(k).is_some(), "channel.{k} が消えている");
        }

        let op = &v["operators"][0];
        for k in [
            "tl", "eg", "mul", "dt1", "ksr", "am_enable", "velocity_sensitivity", "waveform",
            "op_fine_tune", "eg_shift", "level_scale", "velocity_gain",
        ] {
            assert!(op.get(k).is_some(), "operators[0].{k} が消えている");
        }

        // TimeEgParams（オペレーターEGとFGの両方が使う）とTimeStageのキー。
        let eg = &op["eg"];
        for k in ["stages", "stage_count", "loop_enabled", "loop_start", "release_point"] {
            assert!(eg.get(k).is_some(), "eg.{k} が消えている");
        }
        let stage = &eg["stages"][0];
        for k in ["time", "level", "curve"] {
            assert!(stage.get(k).is_some(), "stages[0].{k} が消えている");
        }
        // Pitch FGはTimeEgParams+depthのバイポーラ型。
        assert!(ch["pitch_fg"].get("eg").is_some() && ch["pitch_fg"].get("depth").is_some());
    }
}
