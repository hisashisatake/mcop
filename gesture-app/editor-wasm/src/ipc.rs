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
    frequency: f32,
    velocity: u8,
}

#[derive(Serialize)]
struct NoteOffArgs {
    channel: usize,
}

/// `src-tauri/src/main.rs`の`note_on`コマンドを呼ぶ。音色は直前の`send_patch`で
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
    floor: u8,
    loop_enabled: u8,
    curve: u8,
    eg_shift: u8,
    level_scale: u8,
    velocity_gain: u8,
}

#[derive(Serialize, Deserialize)]
struct ChannelDto {
    algorithm: u8,
    feedback: u8,
    chip_lfo_freq: u8,
    chip_lfo_pmd: u8,
    chip_lfo_amd: u8,
    chip_lfo_delay: u8,
    pms: u8,
    ams: u8,
    filter_cutoff: u8,
    filter_resonance: u8,
    filter_type: u8,
    filter_self_oscillation: bool,
    // Pitch FG（新規、共通EG＋バイポーラDepth）。
    #[serde(default)]
    pitch_fg_ar: u8,
    #[serde(default)]
    pitch_fg_d1r: u8,
    #[serde(default)]
    pitch_fg_d1l: u8,
    #[serde(default)]
    pitch_fg_d2r: u8,
    #[serde(default)]
    pitch_fg_rr: u8,
    #[serde(default = "default_bipolar_depth")]
    pitch_fg_depth: u8,
    #[serde(default)]
    pitch_fg_floor: u8,
    #[serde(default)]
    pitch_fg_delay: u8,
    #[serde(default)]
    pitch_fg_loop: u8,
    #[serde(default)]
    pitch_fg_curve: u8,
    // Cutoff FG（旧Filter EG。depthはバイポーラの直接値）。
    cutoff_fg_ar: u8,
    cutoff_fg_d1r: u8,
    cutoff_fg_d1l: u8,
    cutoff_fg_d2r: u8,
    cutoff_fg_rr: u8,
    #[serde(default = "default_bipolar_depth")]
    cutoff_fg_depth: u8,
    #[serde(default)]
    cutoff_fg_floor: u8,
    #[serde(default)]
    cutoff_fg_delay: u8,
    #[serde(default)]
    cutoff_fg_loop: u8,
    #[serde(default)]
    cutoff_fg_curve: u8,
    // Gain FG（旧VCA EG。音量に負値は無いためDepthを持たずFloorが深さ役）。
    gain_fg_ar: u8,
    gain_fg_d1r: u8,
    gain_fg_d1l: u8,
    gain_fg_d2r: u8,
    gain_fg_rr: u8,
    #[serde(default)]
    gain_fg_floor: u8,
    #[serde(default)]
    gain_fg_delay: u8,
    #[serde(default)]
    gain_fg_loop: u8,
    #[serde(default)]
    gain_fg_curve: u8,
    // 質感LFO（旧パフォーマンスLFO。宣言順インデックス waveform=0〜4/fade_mode=0〜3、
    // destinationは0=Pitch/1=Volume/2=TL/3=Cutoff/4=未接続。offsetは中心128＝オフセットなし）。
    #[serde(default)]
    texture_lfo_waveform: u8,
    #[serde(default)]
    texture_lfo_destination: u8,
    #[serde(default)]
    texture_lfo_rate: u8,
    #[serde(default)]
    texture_lfo_depth: u8,
    #[serde(default)]
    texture_lfo_delay: u8,
    #[serde(default)]
    texture_lfo_fade_mode: u8,
    #[serde(default)]
    texture_lfo_fade_time: u8,
    #[serde(default = "default_texture_lfo_offset")]
    texture_lfo_offset: u8,
}

fn default_bipolar_depth() -> u8 {
    128
}

fn default_texture_lfo_offset() -> u8 {
    128
}

#[derive(Serialize, Deserialize)]
struct PatchDto {
    operators: [OperatorDto; 4],
    channel: ChannelDto,
}

impl PatchDto {
    /// 受信したパッチ内容を`EditorState`へ書き込む（プリセット選択時の同期に使う）。
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
                floor: op.floor as i32,
                op_loop: op.loop_enabled != 0,
                curve: op.curve != 0,
                eg_shift: op.eg_shift as i32,
                level_scale: op.level_scale as i32,
                velocity_gain: op.velocity_gain as i32,
            };
        }
        let ch = &self.channel;
        state.algorithm = ch.algorithm as i32;
        state.feedback = ch.feedback as i32;
        state.chip_lfo_freq = ch.chip_lfo_freq as i32;
        state.chip_lfo_pmd = ch.chip_lfo_pmd as i32;
        state.chip_lfo_amd = ch.chip_lfo_amd as i32;
        state.chip_lfo_delay = ch.chip_lfo_delay as i32;
        state.pms = ch.pms as i32;
        state.ams = ch.ams as i32;
        state.pitch_fg_ar = ch.pitch_fg_ar as i32;
        state.pitch_fg_d1r = ch.pitch_fg_d1r as i32;
        state.pitch_fg_d1l = ch.pitch_fg_d1l as i32;
        state.pitch_fg_d2r = ch.pitch_fg_d2r as i32;
        state.pitch_fg_rr = ch.pitch_fg_rr as i32;
        state.pitch_fg_depth = ch.pitch_fg_depth as i32;
        state.pitch_fg_floor = ch.pitch_fg_floor as i32;
        state.pitch_fg_delay = ch.pitch_fg_delay as i32;
        state.pitch_fg_loop = ch.pitch_fg_loop != 0;
        state.pitch_fg_curve = ch.pitch_fg_curve != 0;
        state.cutoff = ch.filter_cutoff as i32;
        state.resonance = ch.filter_resonance as i32;
        state.filter_type = ch.filter_type as i32;
        state.filter_self_oscillation = ch.filter_self_oscillation;
        state.cutoff_fg_ar = ch.cutoff_fg_ar as i32;
        state.cutoff_fg_d1r = ch.cutoff_fg_d1r as i32;
        state.cutoff_fg_d1l = ch.cutoff_fg_d1l as i32;
        state.cutoff_fg_d2r = ch.cutoff_fg_d2r as i32;
        state.cutoff_fg_rr = ch.cutoff_fg_rr as i32;
        state.cutoff_fg_depth = ch.cutoff_fg_depth as i32;
        state.cutoff_fg_floor = ch.cutoff_fg_floor as i32;
        state.cutoff_fg_delay = ch.cutoff_fg_delay as i32;
        state.cutoff_fg_loop = ch.cutoff_fg_loop != 0;
        state.cutoff_fg_curve = ch.cutoff_fg_curve != 0;
        state.gain_fg_ar = ch.gain_fg_ar as i32;
        state.gain_fg_d1r = ch.gain_fg_d1r as i32;
        state.gain_fg_d1l = ch.gain_fg_d1l as i32;
        state.gain_fg_d2r = ch.gain_fg_d2r as i32;
        state.gain_fg_rr = ch.gain_fg_rr as i32;
        state.gain_fg_floor = ch.gain_fg_floor as i32;
        state.gain_fg_delay = ch.gain_fg_delay as i32;
        state.gain_fg_loop = ch.gain_fg_loop != 0;
        state.gain_fg_curve = ch.gain_fg_curve != 0;
        state.texture_lfo_waveform = ch.texture_lfo_waveform as i32;
        state.texture_lfo_destination = ch.texture_lfo_destination as i32;
        state.texture_lfo_rate = ch.texture_lfo_rate as i32;
        state.texture_lfo_depth = ch.texture_lfo_depth as i32;
        state.texture_lfo_delay = ch.texture_lfo_delay as i32;
        state.texture_lfo_fade_mode = ch.texture_lfo_fade_mode as i32;
        state.texture_lfo_fade_time = ch.texture_lfo_fade_time as i32;
        state.texture_lfo_offset = ch.texture_lfo_offset as i32;
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
            floor: op.floor as u8,
            loop_enabled: op.op_loop as u8,
            curve: op.curve as u8,
            eg_shift: op.eg_shift as u8,
            level_scale: op.level_scale as u8,
            velocity_gain: op.velocity_gain as u8,
        }
    });
    let channel = ChannelDto {
        algorithm: state.algorithm as u8,
        feedback: state.feedback as u8,
        chip_lfo_freq: state.chip_lfo_freq as u8,
        chip_lfo_pmd: state.chip_lfo_pmd as u8,
        chip_lfo_amd: state.chip_lfo_amd as u8,
        chip_lfo_delay: state.chip_lfo_delay as u8,
        pms: state.pms as u8,
        ams: state.ams as u8,
        pitch_fg_ar: state.pitch_fg_ar as u8,
        pitch_fg_d1r: state.pitch_fg_d1r as u8,
        pitch_fg_d1l: state.pitch_fg_d1l as u8,
        pitch_fg_d2r: state.pitch_fg_d2r as u8,
        pitch_fg_rr: state.pitch_fg_rr as u8,
        pitch_fg_depth: state.pitch_fg_depth as u8,
        pitch_fg_floor: state.pitch_fg_floor as u8,
        pitch_fg_delay: state.pitch_fg_delay as u8,
        pitch_fg_loop: state.pitch_fg_loop as u8,
        pitch_fg_curve: state.pitch_fg_curve as u8,
        filter_cutoff: state.cutoff as u8,
        filter_resonance: state.resonance as u8,
        filter_type: state.filter_type as u8,
        filter_self_oscillation: state.filter_self_oscillation,
        cutoff_fg_ar: state.cutoff_fg_ar as u8,
        cutoff_fg_d1r: state.cutoff_fg_d1r as u8,
        cutoff_fg_d1l: state.cutoff_fg_d1l as u8,
        cutoff_fg_d2r: state.cutoff_fg_d2r as u8,
        cutoff_fg_rr: state.cutoff_fg_rr as u8,
        cutoff_fg_depth: state.cutoff_fg_depth as u8,
        cutoff_fg_floor: state.cutoff_fg_floor as u8,
        cutoff_fg_delay: state.cutoff_fg_delay as u8,
        cutoff_fg_loop: state.cutoff_fg_loop as u8,
        cutoff_fg_curve: state.cutoff_fg_curve as u8,
        gain_fg_ar: state.gain_fg_ar as u8,
        gain_fg_d1r: state.gain_fg_d1r as u8,
        gain_fg_d1l: state.gain_fg_d1l as u8,
        gain_fg_d2r: state.gain_fg_d2r as u8,
        gain_fg_rr: state.gain_fg_rr as u8,
        gain_fg_floor: state.gain_fg_floor as u8,
        gain_fg_delay: state.gain_fg_delay as u8,
        gain_fg_loop: state.gain_fg_loop as u8,
        gain_fg_curve: state.gain_fg_curve as u8,
        texture_lfo_waveform: state.texture_lfo_waveform as u8,
        texture_lfo_destination: state.texture_lfo_destination as u8,
        texture_lfo_rate: state.texture_lfo_rate as u8,
        texture_lfo_depth: state.texture_lfo_depth as u8,
        texture_lfo_delay: state.texture_lfo_delay as u8,
        texture_lfo_fade_mode: state.texture_lfo_fade_mode as u8,
        texture_lfo_fade_time: state.texture_lfo_fade_time as u8,
        texture_lfo_offset: state.texture_lfo_offset as u8,
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

/// `op505_set_patch`コマンドの引数。`Op505Patch`は専用DTOを介さず直接シリアライズする
/// （op505-coreの型が既にserde対応で、`.38x6`のような後方互換の負債も無いため。
/// plan「2. パッチのフロントへの受け渡し」参照）。
#[derive(Serialize)]
struct SetOp505PatchArgs {
    patch: op505_core::Op505Patch,
}

/// 現在のOP505パッチを`op505_set_patch`へ送信する。`app.rs`からdirtyフラグが立ったフレームでのみ呼ばれる。
pub fn send_op505_patch(patch: &op505_core::Op505Patch) {
    invoke("op505_set_patch", &SetOp505PatchArgs { patch: *patch });
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

/// `list_bank_entries`/`op505_list_bank_entries`コマンドが返す、今のbankの担当ファイルが持つ
/// 音色一覧の1件。エントリーは常に同一bank（問い合わせたbank）のため`bank`は保持しない
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

/// PRESETSサイドバーのOpen/Save/Save As/Bank切替/プリセット選択が、どちらのエンジンの状態へ
/// 読み書きするかを表す。`app.rs`が`engine_sync::active_engine()`を見て都度組み立て、
/// ここから先の関数（`fetch_bank_entries`/`load_preset`/`open_patch_file`/
/// `save_patch_overwrite`/`save_patch_as`）はこれ1つでym38x6/OP505を出し分ける
/// （ym38x6版とOP505版を別関数として二重実装しないための共通化）。
#[derive(Clone)]
pub enum PatchTarget {
    Ym38x6 { state: Rc<RefCell<EditorState>>, dirty: Rc<Cell<bool>> },
    Op505 { patch: Rc<RefCell<op505_core::Op505Patch>>, dirty: Rc<Cell<bool>> },
}

impl PatchTarget {
    /// Save Asダイアログの拡張子（先頭のドット無し）。
    pub fn file_ext(&self) -> &'static str {
        match self {
            PatchTarget::Ym38x6 { .. } => "38x6",
            PatchTarget::Op505 { .. } => "op505",
        }
    }
}

/// 今のbankの担当ファイルが持つ音色一覧を取得する（presets_dir全体ではない。未登録なら空）。
pub async fn fetch_bank_entries(target: &PatchTarget, bank: u16) -> Vec<PresetEntry> {
    let cmd = match target {
        PatchTarget::Ym38x6 { .. } => "list_bank_entries",
        PatchTarget::Op505 { .. } => "op505_list_bank_entries",
    };
    invoke_query(cmd, &BankArgs { bank }).await.unwrap_or_default()
}

/// ファイル/音色の識別情報（ファイル名・音色名・bank/program）。ロード/保存系の関数が
/// 呼び出し側へ返す共通の戻り値型。
pub struct PatchIdentity {
    pub file_name: Option<String>,
    pub patch_name: String,
    pub bank: u16,
    pub program: u8,
}

/// `open_patch_file`/`get_bank_program`（およびOP505版）が返す、読み込んだ音色の内容。
/// `file_name`（実ファイル名）と`patch_name`（音色名、`PresetEntry.name`）は別概念のため
/// 分けて持つ（1ファイルに複数音色が入りうる以上、ファイル名と音色名は一致するとは限らない）。
/// `P`は`PatchDto`(ym38x6)または`op505_core::Op505Patch`（`PatchTarget`参照）。
#[derive(Deserialize)]
struct LoadedPatchDto<P> {
    patch: P,
    patch_name: String,
    file_name: Option<String>,
    bank: u16,
    program: u8,
}

/// `save_patch_overwrite`/`save_patch_as`が成功時に返す保存結果（ym38x6/OP505共通、
/// パッチ本体を含まないため非ジェネリック）。
#[derive(Deserialize)]
struct SavedFileDto {
    patch_name: String,
    file_name: String,
    bank: u16,
    program: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavePatchArgs<P> {
    patch: P,
    patch_name: String,
    bank: u16,
    program: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveAsArgs<P> {
    patch: P,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
}

/// ym38x6の`EditorState`から、PITCH FG/CUTOFF FG/GAIN FGに属するフィールドだけを
/// 抜き出したスナップショット（`load_preset`の`keep_fg`用）。
struct SavedFgYm38x6 {
    pitch_fg_ar: i32, pitch_fg_d1r: i32, pitch_fg_d1l: i32, pitch_fg_d2r: i32, pitch_fg_rr: i32,
    pitch_fg_depth: i32, pitch_fg_floor: i32, pitch_fg_delay: i32, pitch_fg_loop: bool, pitch_fg_curve: bool,
    cutoff_fg_ar: i32, cutoff_fg_d1r: i32, cutoff_fg_d1l: i32, cutoff_fg_d2r: i32, cutoff_fg_rr: i32,
    cutoff_fg_depth: i32, cutoff_fg_floor: i32, cutoff_fg_delay: i32, cutoff_fg_loop: bool, cutoff_fg_curve: bool,
    gain_fg_ar: i32, gain_fg_d1r: i32, gain_fg_d1l: i32, gain_fg_d2r: i32, gain_fg_rr: i32,
    gain_fg_floor: i32, gain_fg_delay: i32, gain_fg_loop: bool, gain_fg_curve: bool,
}

impl SavedFgYm38x6 {
    fn capture(s: &EditorState) -> Self {
        Self {
            pitch_fg_ar: s.pitch_fg_ar, pitch_fg_d1r: s.pitch_fg_d1r, pitch_fg_d1l: s.pitch_fg_d1l,
            pitch_fg_d2r: s.pitch_fg_d2r, pitch_fg_rr: s.pitch_fg_rr, pitch_fg_depth: s.pitch_fg_depth,
            pitch_fg_floor: s.pitch_fg_floor, pitch_fg_delay: s.pitch_fg_delay,
            pitch_fg_loop: s.pitch_fg_loop, pitch_fg_curve: s.pitch_fg_curve,
            cutoff_fg_ar: s.cutoff_fg_ar, cutoff_fg_d1r: s.cutoff_fg_d1r, cutoff_fg_d1l: s.cutoff_fg_d1l,
            cutoff_fg_d2r: s.cutoff_fg_d2r, cutoff_fg_rr: s.cutoff_fg_rr, cutoff_fg_depth: s.cutoff_fg_depth,
            cutoff_fg_floor: s.cutoff_fg_floor, cutoff_fg_delay: s.cutoff_fg_delay,
            cutoff_fg_loop: s.cutoff_fg_loop, cutoff_fg_curve: s.cutoff_fg_curve,
            gain_fg_ar: s.gain_fg_ar, gain_fg_d1r: s.gain_fg_d1r, gain_fg_d1l: s.gain_fg_d1l,
            gain_fg_d2r: s.gain_fg_d2r, gain_fg_rr: s.gain_fg_rr,
            gain_fg_floor: s.gain_fg_floor, gain_fg_delay: s.gain_fg_delay,
            gain_fg_loop: s.gain_fg_loop, gain_fg_curve: s.gain_fg_curve,
        }
    }

    fn restore(self, s: &mut EditorState) {
        s.pitch_fg_ar = self.pitch_fg_ar; s.pitch_fg_d1r = self.pitch_fg_d1r; s.pitch_fg_d1l = self.pitch_fg_d1l;
        s.pitch_fg_d2r = self.pitch_fg_d2r; s.pitch_fg_rr = self.pitch_fg_rr; s.pitch_fg_depth = self.pitch_fg_depth;
        s.pitch_fg_floor = self.pitch_fg_floor; s.pitch_fg_delay = self.pitch_fg_delay;
        s.pitch_fg_loop = self.pitch_fg_loop; s.pitch_fg_curve = self.pitch_fg_curve;
        s.cutoff_fg_ar = self.cutoff_fg_ar; s.cutoff_fg_d1r = self.cutoff_fg_d1r; s.cutoff_fg_d1l = self.cutoff_fg_d1l;
        s.cutoff_fg_d2r = self.cutoff_fg_d2r; s.cutoff_fg_rr = self.cutoff_fg_rr; s.cutoff_fg_depth = self.cutoff_fg_depth;
        s.cutoff_fg_floor = self.cutoff_fg_floor; s.cutoff_fg_delay = self.cutoff_fg_delay;
        s.cutoff_fg_loop = self.cutoff_fg_loop; s.cutoff_fg_curve = self.cutoff_fg_curve;
        s.gain_fg_ar = self.gain_fg_ar; s.gain_fg_d1r = self.gain_fg_d1r; s.gain_fg_d1l = self.gain_fg_d1l;
        s.gain_fg_d2r = self.gain_fg_d2r; s.gain_fg_rr = self.gain_fg_rr;
        s.gain_fg_floor = self.gain_fg_floor; s.gain_fg_delay = self.gain_fg_delay;
        s.gain_fg_loop = self.gain_fg_loop; s.gain_fg_curve = self.gain_fg_curve;
    }
}

/// 指定のbank/programをレジストリから読み込んで`target`へ書き込み、`target`のdirtyを立てる
/// （次フレームの`send_patch()`/`send_op505_patch()`でエンジンへも反映される。エンジン側へ直接
/// `ym38x6_set_program`/`op505_set_program`は呼ばない。`target`を単一の真実の情報源に保つことで、
/// その後のノブ操作による送信がプリセット内容を上書きしてしまう不整合を避けるため）。
/// バックエンド側はレジストリを引くだけでディスクの再検索は行わない
/// （`get_bank_program`/`op505_get_bank_program`）。Bank/Programスピンの変更・PRESETSリストの
/// クリック・エンジン切替の全経路がこれを呼ぶ（レジストリがOpen/Save/Save Asで正しく
/// 更新されているため、常にこれだけで一貫した結果になる）。戻り値の`bank`/`program`は
/// 常に問い合わせた値をそのまま返す（見つからなくても、ユーザーが指定した値を表示に反映するため）。
///
/// `keep_fg=true`（PRESETSリストをShift+クリックしたとき）なら、読み込む前の
/// PITCH FG/CUTOFF FG/GAIN FGを保ったまま、それ以外（オペレーター・アルゴリズム等）だけを
/// 新しいプリセットの値へ差し替える（お気に入りのFG設定を保ったまま音色だけ切り替えたい用途）。
pub async fn load_preset(target: &PatchTarget, bank: u16, program: u8, keep_fg: bool) -> Option<PatchIdentity> {
    match target {
        PatchTarget::Ym38x6 { state, dirty } => {
            let loaded =
                invoke_query::<LoadedPatchDto<PatchDto>>("get_bank_program", &PresetPatchArgs { bank, program }).await?;
            let saved_fg = keep_fg.then(|| SavedFgYm38x6::capture(&state.borrow()));
            loaded.patch.apply_to(&mut state.borrow_mut());
            if let Some(saved_fg) = saved_fg {
                saved_fg.restore(&mut state.borrow_mut());
            }
            dirty.set(true);
            Some(PatchIdentity { file_name: loaded.file_name, patch_name: loaded.patch_name, bank: loaded.bank, program: loaded.program })
        }
        PatchTarget::Op505 { patch, dirty } => {
            let loaded = invoke_query::<LoadedPatchDto<op505_core::Op505Patch>>(
                "op505_get_bank_program",
                &PresetPatchArgs { bank, program },
            )
            .await?;
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
    }
}

/// presets_dir内/外を区別しない任意のファイルをネイティブOpenダイアログで選び、`target`へ書き込む。
/// PRESETSパネルの`load_preset`とは別経路（レジストリを経由しない直接ファイル読み込み）。
/// `bank`は「今エディタで選択中のbank」を渡す。ダイアログの初期ディレクトリ決定に使うだけでなく、
/// ファイル自身が宣言するbank番号は**無視され**、この`bank`へファイルの全音色が丸ごとロードされる
/// （ファイル自身のbankをそのまま採用するのではない。ym38x6で確認済みの仕様をOP505にも適用する）。
/// 戻り値の`bank`はこの引数と同じ値になる。
pub async fn open_patch_file(target: &PatchTarget, bank: u16) -> Option<PatchIdentity> {
    match target {
        PatchTarget::Ym38x6 { state, dirty } => {
            let loaded =
                invoke_query::<Option<LoadedPatchDto<PatchDto>>>("open_patch_file", &BankArgs { bank }).await.flatten()?;
            loaded.patch.apply_to(&mut state.borrow_mut());
            dirty.set(true);
            Some(PatchIdentity { file_name: loaded.file_name, patch_name: loaded.patch_name, bank: loaded.bank, program: loaded.program })
        }
        PatchTarget::Op505 { patch, dirty } => {
            let loaded = invoke_query::<Option<LoadedPatchDto<op505_core::Op505Patch>>>("op505_open_patch_file", &BankArgs { bank })
                .await
                .flatten()?;
            *patch.borrow_mut() = loaded.patch;
            dirty.set(true);
            Some(PatchIdentity { file_name: loaded.file_name, patch_name: loaded.patch_name, bank: loaded.bank, program: loaded.program })
        }
    }
}

/// 現在のbank/programの担当ファイルへ現在の`target`の内容を上書き保存する。`patch_name`は名前入力欄の
/// 内容で、保存の都度エントリ名を更新する。そのbankにまだファイルが無ければ
/// （バックエンド側がエラーを返すため）Noneが返る。
pub async fn save_patch_overwrite(target: &PatchTarget, bank: u16, program: u8, patch_name: String) -> Option<PatchIdentity> {
    match target {
        PatchTarget::Ym38x6 { state, .. } => {
            let patch = patch_dto_from_state(&state.borrow());
            let saved = invoke_query::<SavedFileDto>(
                "save_patch_overwrite",
                &SavePatchArgs { patch, patch_name, bank, program },
            )
            .await?;
            Some(PatchIdentity { file_name: Some(saved.file_name), patch_name: saved.patch_name, bank: saved.bank, program: saved.program })
        }
        PatchTarget::Op505 { patch, .. } => {
            let patch = *patch.borrow();
            let saved = invoke_query::<SavedFileDto>(
                "op505_save_patch_overwrite",
                &SavePatchArgs { patch, patch_name, bank, program },
            )
            .await?;
            Some(PatchIdentity { file_name: Some(saved.file_name), patch_name: saved.patch_name, bank: saved.bank, program: saved.program })
        }
    }
}

/// ネイティブSaveダイアログで保存先を選び、現在の`target`の内容を新規ファイルとして書き出す。
/// `patch_name`（名前入力欄の内容）をそのままエントリ名として使い、`bank`/`program`は今表示されている
/// スピンの値をそのまま書き込む（自動採番はしない＝見た目と動作を一致させる）。`default_file_name`は
/// ダイアログの提案ファイル名にのみ使う。以後の`save_patch_overwrite`はこの新しいファイルに対して行われる。
pub async fn save_patch_as(
    target: &PatchTarget,
    patch_name: String,
    bank: u16,
    program: u8,
    default_file_name: String,
) -> Option<PatchIdentity> {
    match target {
        PatchTarget::Ym38x6 { state, .. } => {
            let patch = patch_dto_from_state(&state.borrow());
            let saved = invoke_query::<Option<SavedFileDto>>(
                "save_patch_as",
                &SaveAsArgs { patch, patch_name, bank, program, default_file_name },
            )
            .await
            .flatten()?;
            Some(PatchIdentity { file_name: Some(saved.file_name), patch_name: saved.patch_name, bank: saved.bank, program: saved.program })
        }
        PatchTarget::Op505 { patch, .. } => {
            let patch = *patch.borrow();
            let saved = invoke_query::<Option<SavedFileDto>>(
                "op505_save_patch_as",
                &SaveAsArgs { patch, patch_name, bank, program, default_file_name },
            )
            .await
            .flatten()?;
            Some(PatchIdentity { file_name: Some(saved.file_name), patch_name: saved.patch_name, bank: saved.bank, program: saved.program })
        }
    }
}
