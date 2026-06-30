use serde::Serialize;
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
")]
extern "C" {
    #[wasm_bindgen(js_name = tauri_invoke)]
    fn tauri_invoke(cmd: &str, args_json: &str) -> js_sys::Promise;
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

/// `ym38x6_dto::OperatorParamsDto`と同じフィールド名(snake_case)で送る
/// （Tauriコマンド引数の内部構造体はserdeのデフォルト規則でデシリアライズされ、
/// コマンド最上位の引数名のみがcamelCaseへ自動変換される点に注意）。
#[derive(Serialize)]
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

#[derive(Serialize)]
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

#[derive(Serialize)]
struct PatchDto {
    operators: [OperatorDto; 4],
    channel: ChannelDto,
}

#[derive(Serialize)]
struct SetPatchArgs {
    patch: PatchDto,
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

    invoke("ym38x6_set_patch", &SetPatchArgs { patch: PatchDto { operators, channel } });

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
