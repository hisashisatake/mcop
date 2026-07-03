use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::state::EditorState;

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
    wave_slot: u8,
    frequency: f32,
}

#[derive(Serialize)]
struct NoteOffArgs {
    channel: usize,
}

/// `src-tauri/src/main.rs`の`note_on`コマンドを呼ぶ。音色は直前の`send_patch`で
/// 設定済みのcurrent_patchが使われるため`wave_slot`は常に0（未使用、トレイト互換のためのダミー）。
pub fn note_on(channel: usize, frequency: f32) {
    invoke("note_on", &NoteOnArgs { channel, wave_slot: 0, frequency });
}

/// `src-tauri/src/main.rs`の`note_off`コマンドを呼ぶ。
pub fn note_off(channel: usize) {
    invoke("note_off", &NoteOffArgs { channel });
}

/// `ym38x6_dto::OperatorParamsDto`と同じフィールド名(snake_case)で送る/受け取る
/// （Tauriコマンド引数・戻り値の内部構造体はserdeのデフォルト規則でやり取りされ、
/// コマンド最上位の引数名のみがcamelCaseへ自動変換される点に注意）。
#[derive(Serialize, Deserialize)]
struct OperatorDto {
    tl: u8,
    ar: u8,
    d1r: u8,
    d2r: u8,
    d1l: u8,
    rr: u8,
    mul: u8,
    dt1: u8,
    ksr: u8,
    am_enable: bool,
    velocity_sensitivity: u8,
    waveform: u8,
    op_fine_tune: u8,
}

#[derive(Serialize, Deserialize)]
struct ChannelDto {
    algorithm: u8,
    feedback: u8,
    tone_lfo_freq: u8,
    tone_lfo_pmd: u8,
    tone_lfo_amd: u8,
    tone_lfo_delay: u8,
    pms: u8,
    ams: u8,
    filter_cutoff: u8,
    filter_resonance: u8,
    filter_type: u8,
    filter_self_oscillation: bool,
    filter_eg_attack: u8,
    filter_eg_decay: u8,
    filter_eg_sustain: u8,
    filter_eg_release: u8,
    filter_eg_depth: u8,
}

#[derive(Serialize, Deserialize)]
struct PatchDto {
    operators: [OperatorDto; 4],
    channel: ChannelDto,
}

impl PatchDto {
    /// 受信したパッチ内容を`EditorState`へ書き込む（プリセット選択時の同期に使う）。
    /// PERF LFO（lfo_rate/depth/delay）はパッチに含まれないため変更しない
    /// （main.jsのホイール/Vキー制御とは別系統という既存方針のまま）。
    fn apply_to(&self, state: &mut EditorState) {
        for (i, op) in self.operators.iter().enumerate() {
            state.operators[i] = crate::state::OperatorState {
                tl: op.tl as i32,
                ar: op.ar as i32,
                d1r: op.d1r as i32,
                d2r: op.d2r as i32,
                d1l: op.d1l as i32,
                rr: op.rr as i32,
                mul: op.mul as i32,
                dt1: op.dt1 as i32,
                ksr: op.ksr as i32,
                ame: op.am_enable,
                vel_sens: op.velocity_sensitivity as i32,
                op_fine_tune: op.op_fine_tune as i32,
                waveform: op.waveform as i32,
            };
        }
        let ch = &self.channel;
        state.algorithm = ch.algorithm as i32;
        state.feedback = ch.feedback as i32;
        state.tone_freq = ch.tone_lfo_freq as i32;
        state.tone_pmd = ch.tone_lfo_pmd as i32;
        state.tone_amd = ch.tone_lfo_amd as i32;
        state.tone_delay = ch.tone_lfo_delay as i32;
        state.pms = ch.pms as i32;
        state.ams = ch.ams as i32;
        state.cutoff = ch.filter_cutoff as i32;
        state.resonance = ch.filter_resonance as i32;
        state.filter_type = ch.filter_type as i32;
        state.filter_self_oscillation = ch.filter_self_oscillation;
        state.feg_a = ch.filter_eg_attack as i32;
        state.feg_d = ch.filter_eg_decay as i32;
        state.feg_s = ch.filter_eg_sustain as i32;
        state.feg_r = ch.filter_eg_release as i32;
        state.feg_depth = ch.filter_eg_depth as i32;
    }
}

#[derive(Serialize)]
struct SetPatchArgs {
    patch: PatchDto,
}

/// `EditorState`から`PatchDto`を構築する（`send_patch`とセーブ系関数で共有）。
fn patch_dto_from_state(state: &EditorState) -> PatchDto {
    let operators = std::array::from_fn(|i| {
        let op = &state.operators[i];
        OperatorDto {
            tl: op.tl as u8,
            ar: op.ar as u8,
            d1r: op.d1r as u8,
            d2r: op.d2r as u8,
            d1l: op.d1l as u8,
            rr: op.rr as u8,
            mul: op.mul as u8,
            dt1: op.dt1 as u8,
            ksr: op.ksr as u8,
            am_enable: op.ame,
            velocity_sensitivity: op.vel_sens as u8,
            waveform: op.waveform as u8,
            op_fine_tune: op.op_fine_tune as u8,
        }
    });
    let channel = ChannelDto {
        algorithm: state.algorithm as u8,
        feedback: state.feedback as u8,
        tone_lfo_freq: state.tone_freq as u8,
        tone_lfo_pmd: state.tone_pmd as u8,
        tone_lfo_amd: state.tone_amd as u8,
        tone_lfo_delay: state.tone_delay as u8,
        pms: state.pms as u8,
        ams: state.ams as u8,
        filter_cutoff: state.cutoff as u8,
        filter_resonance: state.resonance as u8,
        filter_type: state.filter_type as u8,
        filter_self_oscillation: state.filter_self_oscillation,
        filter_eg_attack: state.feg_a as u8,
        filter_eg_decay: state.feg_d as u8,
        filter_eg_sustain: state.feg_s as u8,
        filter_eg_release: state.feg_r as u8,
        filter_eg_depth: state.feg_depth as u8,
    };
    PatchDto { operators, channel }
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

/// 現在の`EditorState`を`ym38x6_set_patch`/`set_master_effects`へ送信する。
/// `app.rs`からdirtyフラグが立ったフレームでのみ呼ばれる。
pub fn send_patch(state: &EditorState) {
    invoke("ym38x6_set_patch", &SetPatchArgs { patch: patch_dto_from_state(state) });

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

/// `list_bank_entries`コマンドが返す、今のbankの担当ファイルが持つ音色一覧の1件。
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
    invoke_query("list_bank_entries", &BankArgs { bank }).await.unwrap_or_default()
}

/// ファイル/音色の識別情報（ファイル名・音色名・bank/program）。ロード/保存系の関数が
/// 呼び出し側へ返す共通の戻り値型。
pub struct PatchIdentity {
    pub file_name: Option<String>,
    pub patch_name: String,
    pub bank: u16,
    pub program: u8,
}

/// 指定のbank/programをレジストリから読み込んで`state`へ書き込み、`dirty`を立てる
/// （次フレームの`send_patch()`でエンジンへも反映される。エンジン側へ直接`ym38x6_set_program`は
/// 呼ばない。`state`を単一の真実の情報源に保つことで、その後のノブ操作によるsend_patchが
/// プリセット内容を上書きしてしまう不整合を避けるため）。バックエンド側はレジストリを引くだけで
/// ディスクの再検索は行わない（`get_bank_program`）。Bank/Programスピンの変更・PRESETSリストの
/// クリックの両方から呼ぶ（レジストリがOpen/Save/Save Asで正しく更新されているため、
/// 常にこれだけで一貫した結果になる。別ファイルへ飛ぶ経路とファイル内に留まる経路を
/// 分ける必要はない）。戻り値の`bank`/`program`は常に問い合わせた値をそのまま返す
/// （見つからなくても、ユーザーが指定した値を表示に反映するため）。
pub async fn load_preset(
    state: Rc<RefCell<EditorState>>,
    dirty: Rc<Cell<bool>>,
    bank: u16,
    program: u8,
) -> Option<PatchIdentity> {
    let loaded =
        invoke_query::<LoadedPatchDto>("get_bank_program", &PresetPatchArgs { bank, program }).await?;
    loaded.patch.apply_to(&mut state.borrow_mut());
    dirty.set(true);
    Some(PatchIdentity {
        file_name: loaded.file_name,
        patch_name: loaded.patch_name,
        bank: loaded.bank,
        program: loaded.program,
    })
}

/// `open_patch_file`/`get_bank_program`が返す、読み込んだ音色の内容。
/// `file_name`（実ファイル名）と`patch_name`（音色名、`PresetEntry.name`）は別概念のため
/// 分けて持つ（1ファイルに複数音色が入りうる以上、ファイル名と音色名は一致するとは限らない）。
#[derive(Deserialize)]
struct LoadedPatchDto {
    patch: PatchDto,
    patch_name: String,
    file_name: Option<String>,
    bank: u16,
    program: u8,
}

/// `save_patch_overwrite`/`save_patch_as`が成功時に返す保存結果。
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
    patch: PatchDto,
    patch_name: String,
    bank: u16,
    program: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveAsArgs {
    patch: PatchDto,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
}

/// presets_dir内/外を区別しない任意の`.38x6`ファイルをネイティブOpenダイアログで選び、`state`へ書き込む。
/// PRESETSパネルの`load_preset`とは別経路（レジストリを経由しない直接ファイル読み込み）。
/// `bank`は「今エディタで選択中のbank」を渡す。ダイアログの初期ディレクトリ決定に使うだけでなく、
/// ファイル自身が宣言するbank番号は**無視され**、この`bank`へファイルの全音色が丸ごとロードされる
/// （ファイル自身のbankをそのまま採用するのではない。ユーザー確認済みの仕様）。
/// 戻り値の`bank`はこの引数と同じ値になる。
pub async fn open_patch_file(state: Rc<RefCell<EditorState>>, dirty: Rc<Cell<bool>>, bank: u16) -> Option<PatchIdentity> {
    let loaded = invoke_query::<Option<LoadedPatchDto>>("open_patch_file", &BankArgs { bank }).await.flatten()?;
    loaded.patch.apply_to(&mut state.borrow_mut());
    dirty.set(true);
    Some(PatchIdentity {
        file_name: loaded.file_name,
        patch_name: loaded.patch_name,
        bank: loaded.bank,
        program: loaded.program,
    })
}

/// 現在のbank/programの担当ファイルへ現在の`state`を上書き保存する。`patch_name`は名前入力欄の
/// 内容で、保存の都度エントリ名を更新する。そのbankにまだファイルが無ければ
/// （バックエンド側がエラーを返すため）Noneが返る。
pub async fn save_patch_overwrite(
    state: Rc<RefCell<EditorState>>,
    bank: u16,
    program: u8,
    patch_name: String,
) -> Option<PatchIdentity> {
    let patch = patch_dto_from_state(&state.borrow());
    let saved =
        invoke_query::<SavedFileDto>("save_patch_overwrite", &SavePatchArgs { patch, patch_name, bank, program }).await?;
    Some(PatchIdentity {
        file_name: Some(saved.file_name),
        patch_name: saved.patch_name,
        bank: saved.bank,
        program: saved.program,
    })
}

/// ネイティブSaveダイアログで保存先を選び、現在の`state`を新規`.38x6`として書き出す。
/// `patch_name`（名前入力欄の内容）をそのままエントリ名として使い、`bank`/`program`は今表示されている
/// スピンの値をそのまま書き込む（自動採番はしない＝見た目と動作を一致させる）。`default_file_name`は
/// ダイアログの提案ファイル名にのみ使う。以後の`save_patch_overwrite`はこの新しいファイルに対して行われる。
pub async fn save_patch_as(
    state: Rc<RefCell<EditorState>>,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
) -> Option<PatchIdentity> {
    let patch = patch_dto_from_state(&state.borrow());
    let saved = invoke_query::<Option<SavedFileDto>>(
        "save_patch_as",
        &SaveAsArgs { patch, patch_name, bank, program, default_file_name },
    )
    .await
    .flatten()?;
    Some(PatchIdentity {
        file_name: Some(saved.file_name),
        patch_name: saved.patch_name,
        bank: saved.bank,
        program: saved.program,
    })
}
