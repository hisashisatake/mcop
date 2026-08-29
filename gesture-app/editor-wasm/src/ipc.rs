use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::state::MasterEffectsState;

#[wasm_bindgen(inline_js = "
export function tauri_invoke(cmd, argsJson) {
    const args = JSON.parse(argsJson);
    if (window.__TAURI__ && window.__TAURI__.core) {
        return window.__TAURI__.core.invoke(cmd, args);
    }
    console.warn('__TAURI__ not available, dropping invoke:', cmd);
    return Promise.resolve();
}
export async function tauri_invoke_json(cmd, argsJson) {
    const args = JSON.parse(argsJson);
    if (window.__TAURI__ && window.__TAURI__.core) {
        const result = await window.__TAURI__.core.invoke(cmd, args);
        return JSON.stringify(result);
    }
    console.warn('__TAURI__ not available, dropping invoke:', cmd);
    return 'null';
}
")]
extern "C" {
    #[wasm_bindgen(js_name = tauri_invoke)]
    fn tauri_invoke(cmd: &str, args_json: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = tauri_invoke_json)]
    fn tauri_invoke_json(cmd: &str, args_json: &str) -> js_sys::Promise;
}

// main.js（メイン画面）の`#program-bank`/`#program-num`欄の現在値を同期的に読む。
// エディタ起動直後の初期Bank/Programを、メイン画面の選択と一致させるために使う
// （main.jsとeditor-wasmは別々にコンパイルされたJS/WASMなので、DOM経由で橋渡しする）。
#[wasm_bindgen(inline_js = "
export function read_program_fields() {
    const bankEl = document.getElementById('program-bank');
    const numEl = document.getElementById('program-num');
    const bank = bankEl ? (parseInt(bankEl.value, 10) || 0) : 0;
    const program = numEl ? (parseInt(numEl.value, 10) || 0) : 0;
    return JSON.stringify({ bank, program });
}
")]
extern "C" {
    #[wasm_bindgen(js_name = read_program_fields)]
    fn read_program_fields_js() -> String;
}

#[derive(Deserialize)]
struct ProgramFields {
    bank: u16,
    program: u8,
}

/// メイン画面のBank/Program欄から初期値を読み取る（無ければ0/0）。
pub fn read_program_fields() -> (u16, u8) {
    let fields: ProgramFields = serde_json::from_str(&read_program_fields_js()).unwrap_or(ProgramFields {
        bank: 0,
        program: 0,
    });
    (fields.bank, fields.program)
}

/// 指定コマンドをTauri IPC経由で呼ぶ（fire-and-forget、戻り値は無視する）。
fn invoke(cmd: &'static str, args: &impl Serialize) {
    let json = match serde_json::to_string(args) {
        Ok(j) => j,
        Err(_) => return,
    };
    let promise = tauri_invoke(cmd, &json);
    wasm_bindgen_futures::spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    });
}

/// 指定コマンドをTauri IPC経由で呼び、戻り値をデシリアライズして返す（応答を待つ版）。
async fn invoke_query<T: serde::de::DeserializeOwned>(cmd: &'static str, args: &impl Serialize) -> Option<T> {
    let json = serde_json::to_string(args).ok()?;
    let promise = tauri_invoke_json(cmd, &json);
    let result = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
    let s = result.as_string()?;
    serde_json::from_str(&s).ok()
}

/// `note_on`/`note_off`コマンドの引数。トップレベル引数名はTauriの規約でcamelCaseへ変換される
/// （ネストしたDTOのフィールド名とは異なり、ここはコマンドの直接の引数なので対象になる）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NoteOnArgs {
    channel: usize,
    frequency: f32,
    velocity: u8,
}

#[derive(Serialize)]
struct NoteOffArgs {
    channel: usize,
}

/// `src-tauri/src/main.rs`の`note_on`コマンドを呼ぶ。音色は直前の`send_op505_patch`で
/// 設定済みの保存済みパッチ（current_patch）が使われる。
/// velocityはキーボード試奏用の固定値（100＝普通に弾いた相当。gesture-appはまだ演奏動作から
/// ベロシティを可変化していない）。
pub fn note_on(channel: usize, frequency: f32) {
    invoke("note_on", &NoteOnArgs { channel, frequency, velocity: 100 });
}

/// `src-tauri/src/main.rs`の`note_off`コマンドを呼ぶ。
pub fn note_off(channel: usize) {
    invoke("note_off", &NoteOffArgs { channel });
}

/// `set_master_effects`コマンドの引数。最上位引数名はTauriの規約でcamelCaseへ変換される。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MasterEffectsArgs {
    reverb_send: u8,
    reverb_type: u8,
    reverb_time: u8,
    chorus_send: u8,
    chorus_type: u8,
    chorus_mod_rate: u8,
    chorus_mod_depth: u8,
    chorus_feedback: u8,
    chorus_send_to_reverb: u8,
}

/// `op505_set_patch`コマンドの引数。`Op505Patch`は専用DTOを介さず直接シリアライズする
/// （op505-coreの型が既にserde対応で、後方互換の負債も無いため。plan「2. パッチのフロントへの
/// 受け渡し」参照）。
#[derive(Serialize)]
struct SetOp505PatchArgs {
    patch: op505_core::Op505Patch,
}

/// 現在のOP505パッチを`op505_set_patch`へ送信する。`app.rs`からdirtyフラグが立ったフレームでのみ呼ばれる。
pub fn send_op505_patch(patch: &op505_core::Op505Patch) {
    invoke("op505_set_patch", &SetOp505PatchArgs { patch: *patch });
}

/// 現在の`MasterEffectsState`を`set_master_effects`へ送信する。
/// `app.rs`からdirtyフラグが立ったフレームでのみ呼ばれる。
pub fn send_master_effects(state: &MasterEffectsState) {
    invoke(
        "set_master_effects",
        &MasterEffectsArgs {
            reverb_send: state.rev_send as u8,
            reverb_type: state.reverb_type as u8,
            reverb_time: state.reverb_time as u8,
            chorus_send: state.cho_send as u8,
            chorus_type: state.chorus_type as u8,
            chorus_mod_rate: state.chorus_mod_rate as u8,
            chorus_mod_depth: state.chorus_mod_depth as u8,
            chorus_feedback: state.chorus_feedback as u8,
            chorus_send_to_reverb: state.chorus_send_to_reverb as u8,
        },
    );
}

/// `op505_list_bank_entries`コマンドが返す、今のbankの担当ファイルが持つ音色一覧の1件。
/// エントリーは常に同一bank（問い合わせたbank）のため`bank`は保持しない
/// （DTOには含まれるが、ここでは無視して構わない）。
#[derive(Deserialize, Clone)]
pub struct PresetEntry {
    pub program: u8,
    pub name: String,
}

#[derive(Serialize)]
struct PresetPatchArgs {
    bank: u16,
    program: u8,
}

#[derive(Serialize)]
struct BankArgs {
    bank: u16,
}

/// 今のbankの担当ファイルが持つ音色一覧を取得する（presets_dir全体ではない。未登録なら空）。
pub async fn fetch_bank_entries(bank: u16) -> Vec<PresetEntry> {
    invoke_query("op505_list_bank_entries", &BankArgs { bank }).await.unwrap_or_default()
}

/// bankの担当ファイル名だけを取得する（パッチはロードしない）。エディタのBank欄を
/// 切り替えたときの表示更新専用（`app.rs::handle_bank_changed`参照、`op505_get_bank_program`と
/// 違い今のパッチには一切触れない）。
pub async fn fetch_bank_file_name(bank: u16) -> Option<String> {
    invoke_query::<Option<String>>("op505_get_bank_file_name", &BankArgs { bank }).await.flatten()
}

/// ファイル/音色の識別情報（ファイル名・音色名・bank/program）。ロード/保存系の関数が
/// 呼び出し側へ返す共通の戻り値型。
pub struct PatchIdentity {
    pub file_name: Option<String>,
    pub patch_name: String,
    pub bank: u16,
    pub program: u8,
}

/// `op505_open_patch_file`/`op505_get_bank_program`が返す、読み込んだ音色の内容。
/// `file_name`（実ファイル名）と`patch_name`（音色名、`PresetEntry.name`）は別概念のため
/// 分けて持つ（1ファイルに複数音色が入りうる以上、ファイル名と音色名は一致するとは限らない）。
#[derive(Deserialize)]
struct LoadedPatchDto {
    patch: op505_core::Op505Patch,
    patch_name: String,
    file_name: Option<String>,
    bank: u16,
    program: u8,
}

/// `op505_save_patch_overwrite`/`op505_save_patch_as`が成功時に返す保存結果
/// （パッチ本体を含まない）。
#[derive(Deserialize)]
struct SavedFileDto {
    patch_name: String,
    file_name: String,
    bank: u16,
    program: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavePatchArgs {
    patch: op505_core::Op505Patch,
    patch_name: String,
    bank: u16,
    program: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveAsArgs {
    patch: op505_core::Op505Patch,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
}

/// 指定のbank/programをレジストリから読み込んで`patch`へ書き込み、`dirty`を立てる
/// （次フレームの`send_op505_patch()`でエンジンへも反映される。エンジン側へ直接
/// `op505_set_program`は呼ばない。`patch`を単一の真実の情報源に保つことで、その後のノブ操作による
/// 送信がプリセット内容を上書きしてしまう不整合を避けるため）。
/// バックエンド側はレジストリを引くだけでディスクの再検索は行わない（`op505_get_bank_program`）。
/// Bank/Programスピンの変更・PRESETSリストのクリックの全経路がこれを呼ぶ（レジストリが
/// Open/Save/Save Asで正しく更新されているため、常にこれだけで一貫した結果になる）。
/// 戻り値の`bank`/`program`は常に問い合わせた値をそのまま返す（見つからなくても、ユーザーが
/// 指定した値を表示に反映するため）。
///
/// `keep_fg=true`（PRESETSリストをShift+クリックしたとき）なら、読み込む前の
/// PITCH FG/CUTOFF FG/GAIN FGを保ったまま、それ以外（オペレーター・アルゴリズム等）だけを
/// 新しいプリセットの値へ差し替える（お気に入りのFG設定を保ったまま音色だけ切り替えたい用途）。
pub async fn load_preset(
    patch: &Rc<RefCell<op505_core::Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    bank: u16,
    program: u8,
    keep_fg: bool,
) -> Option<PatchIdentity> {
    let loaded = invoke_query::<LoadedPatchDto>("op505_get_bank_program", &PresetPatchArgs { bank, program }).await?;
    let mut new_patch = loaded.patch;
    if keep_fg {
        let current = patch.borrow();
        new_patch.channel.pitch_fg = current.channel.pitch_fg;
        new_patch.channel.cutoff_fg = current.channel.cutoff_fg;
        new_patch.channel.gain_fg = current.channel.gain_fg;
    }
    *patch.borrow_mut() = new_patch;
    dirty.set(true);
    Some(PatchIdentity { file_name: loaded.file_name, patch_name: loaded.patch_name, bank: loaded.bank, program: loaded.program })
}

/// presets_dir内/外を区別しない任意の`.op505`ファイルをネイティブOpenダイアログで選び、`patch`へ書き込む。
/// PRESETSパネルの`load_preset`とは別経路（レジストリを経由しない直接ファイル読み込み）。
/// `bank`は「今エディタで選択中のbank」を渡す。ダイアログの初期ディレクトリ決定に使うだけでなく、
/// ファイル自身が宣言するbank番号は**無視され**、この`bank`へファイルの全音色が丸ごとロードされる。
/// 戻り値の`bank`はこの引数と同じ値になる。
pub async fn open_patch_file(
    patch: &Rc<RefCell<op505_core::Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    bank: u16,
) -> Option<PatchIdentity> {
    let loaded = invoke_query::<Option<LoadedPatchDto>>("op505_open_patch_file", &BankArgs { bank }).await.flatten()?;
    *patch.borrow_mut() = loaded.patch;
    dirty.set(true);
    Some(PatchIdentity { file_name: loaded.file_name, patch_name: loaded.patch_name, bank: loaded.bank, program: loaded.program })
}

/// 現在のbank/programの担当ファイルへ現在の`patch`の内容を上書き保存する。`patch_name`は名前入力欄の
/// 内容で、保存の都度エントリ名を更新する。そのbankにまだファイルが無ければ
/// （バックエンド側がエラーを返すため）Noneが返る。
pub async fn save_patch_overwrite(
    patch: &Rc<RefCell<op505_core::Op505Patch>>,
    bank: u16,
    program: u8,
    patch_name: String,
) -> Option<PatchIdentity> {
    let patch = *patch.borrow();
    let saved =
        invoke_query::<SavedFileDto>("op505_save_patch_overwrite", &SavePatchArgs { patch, patch_name, bank, program })
            .await?;
    Some(PatchIdentity { file_name: Some(saved.file_name), patch_name: saved.patch_name, bank: saved.bank, program: saved.program })
}

#[derive(Serialize)]
struct AddPresetArgs {
    bank: u16,
    patch: op505_core::Op505Patch,
}

/// 現在のbankの担当ファイルへ新規エントリ（"+ New Voice"）を追加して`patch`へ書き込み、`dirty`を立てる
/// （`load_preset`と同じくエンジンへも反映されるようにするため）。`source_patch`は通常クリックなら
/// `Op505Patch::default()`、Shift+クリックなら現在編集中のパッチのコピーを呼び出し側が渡す
/// （app.rs参照）。program番号・名前（"VoiceNNN"）はバックエンド側（`op505_add_preset`）が決める。
/// そのbankにまだファイルが無ければ（`op505_save_patch_overwrite`と同じ制約でエラーを返すため）Noneが返る。
pub async fn add_preset(
    patch: &Rc<RefCell<op505_core::Op505Patch>>,
    dirty: &Rc<Cell<bool>>,
    bank: u16,
    source_patch: op505_core::Op505Patch,
) -> Option<PatchIdentity> {
    let loaded = invoke_query::<LoadedPatchDto>("op505_add_preset", &AddPresetArgs { bank, patch: source_patch }).await?;
    *patch.borrow_mut() = loaded.patch;
    dirty.set(true);
    Some(PatchIdentity { file_name: loaded.file_name, patch_name: loaded.patch_name, bank: loaded.bank, program: loaded.program })
}

#[derive(Serialize)]
struct DeletePresetArgs {
    bank: u16,
    program: u8,
}

/// 現在選択中(current_program)の音色をDELETEキーで削除する。確認ダイアログはバックエンド側
/// （`op505_delete_preset`）がネイティブYes/Noダイアログとして表示する。No/キャンセルなら
/// `None`が返る（削除は起きていない）。Yesで削除されれば、削除後の音色一覧を返す。
pub async fn delete_preset(bank: u16, program: u8) -> Option<Vec<PresetEntry>> {
    invoke_query::<Option<Vec<PresetEntry>>>("op505_delete_preset", &DeletePresetArgs { bank, program }).await.flatten()
}

/// ネイティブSaveダイアログで保存先を選び、現在の`patch`の内容を新規ファイルとして書き出す。
/// `patch_name`（名前入力欄の内容）をそのままエントリ名として使い、`bank`/`program`は今表示されている
/// スピンの値をそのまま書き込む（自動採番はしない＝見た目と動作を一致させる）。`default_file_name`は
/// ダイアログの提案ファイル名にのみ使う。以後の`save_patch_overwrite`はこの新しいファイルに対して行われる。
pub async fn save_patch_as(
    patch: &Rc<RefCell<op505_core::Op505Patch>>,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
) -> Option<PatchIdentity> {
    let patch = *patch.borrow();
    let saved = invoke_query::<Option<SavedFileDto>>(
        "op505_save_patch_as",
        &SaveAsArgs { patch, patch_name, bank, program, default_file_name },
    )
    .await
    .flatten()?;
    Some(PatchIdentity { file_name: Some(saved.file_name), patch_name: saved.patch_name, bank: saved.bank, program: saved.program })
}
