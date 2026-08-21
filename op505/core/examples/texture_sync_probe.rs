//! S&H(質感=1)+テンポ同期(SYNC)が実際にBPMへロックすることを実証するプローブ。
//!
//! Gain FGへ「stage0=time0の飛び段＋stage1=raw 0.37秒のホールド段」の2段ループを組み、
//! texture=S&H・sync_enabled=1・sync_rate=1/16で駆動する。rawのtime値は意図的にSYNC対象の
//! 音価とズラしてあり、BPMを変えても実測ホールド長がSYNC音価どおり（raw値ではなく）に
//! 追従することを数値的に検証しつつ、実際に鳴らしたWAVも書き出す
//! （質感LFO退役の当初動機の実証、memory `project_texture_lfo_retirement.md`参照）。
//!
//! 実行: cargo run -p op505-core --example texture_sync_probe -- <出力ディレクトリ>

use std::path::Path;

use op505_core::{Op505ChannelParams, Op505Engine, Op505OperatorParams, Op505Patch};
use sound_core::{
    seconds_to_time, sync_note_anchor, sync_rate_beats, tempo_speed_scale, TimeEg, TimeEgParams,
    TimeStage, Vco, MAX_STAGES, TEXTURE_SAMPLE_HOLD,
};

const SAMPLE_RATE: f32 = 44100.0;
const NOTE_FREQ: f32 = 440.0; // A4
const SYNC_RATE_INDEX: u8 = 4; // 1/16
const HOLD_SECS: f32 = 3.0;

/// Gain FG: stage0=time0の飛び段(level=40) → stage1=raw 0.37秒のホールド段(level=220)を
/// loop_start=0..=release_point=1で周回。rawの0.37秒はSYNC対象の1/16音価と意図的にズラして
/// あり、実測ホールド長がこのraw値ではなくSYNC音価どおりになることを確認する。
fn build_gain_fg() -> TimeEgParams {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    stages[0] = TimeStage { time: 0, level: 40, curve: 0 };
    stages[1] = TimeStage { time: seconds_to_time(0.37), level: 220, curve: 0 };
    TimeEgParams {
        stages,
        stage_count: 2,
        loop_enabled: 1,
        loop_start: 0,
        release_point: 1,
        sync_enabled: 1,
        sync_rate: sync_note_anchor(SYNC_RATE_INDEX),
        texture: TEXTURE_SAMPLE_HOLD,
        ..TimeEgParams::default()
    }
}

/// OP1のみが鳴るキャリア1本（`loop_drift_probe`と同じ組み方）。OP1自体のEGは
/// 即アタック・無限サスティンで固定し、Gain FGだけがS&H+SYNCの主役になるようにする。
fn build_patch(gain_fg: TimeEgParams) -> Op505Patch {
    let mut op_stages = [TimeStage::default(); MAX_STAGES];
    op_stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
    op_stages[1] = TimeStage { time: 0, level: 0, curve: 0 };
    let op_eg = TimeEgParams { stages: op_stages, stage_count: 2, release_point: 0, ..TimeEgParams::default() };

    let mut patch = Op505Patch::default();
    patch.operators[0] =
        Op505OperatorParams { tl: 255, mul: 1, waveform: 0, eg: op_eg, ..Op505OperatorParams::default() };
    patch.channel = Op505ChannelParams::default();
    patch.channel.algorithm = 7;
    patch.channel.gain_fg = gain_fg;
    patch
}

fn render(patch: Op505Patch, bpm: f32) -> Vec<f32> {
    let mut engine = Op505Engine::new(SAMPLE_RATE);
    engine.set_patch(patch);
    engine.set_tempo(bpm);
    engine.note_on(0, NOTE_FREQ, 100);
    let n = (HOLD_SECS * SAMPLE_RATE) as usize;
    let mut out = vec![0.0f32; n];
    engine.render(&mut out, 1);
    out
}

/// `TimeEg`を直接ドライブし、S&Hのホールド区間長(サンプル数)を実測してSYNC音価どおりに
/// なっているか検証する。段境界の1サンプルだけの「飛び段」フラッシュは除外する。
fn verify_sync_lock(params: &TimeEgParams, bpm: f32) {
    let scale = tempo_speed_scale(params, bpm);
    let target_seconds = sync_rate_beats(params.sync_rate) * 60.0 / bpm;
    let expected_samples = (target_seconds * SAMPLE_RATE) as i64;

    let mut eg = TimeEg::new();
    eg.note_on();

    let mut hold_start_level = eg.tick(SAMPLE_RATE, *params, scale);
    let mut hold_samples: i64 = 0;
    let mut measured: Vec<i64> = Vec::new();

    for _ in 0..((HOLD_SECS * SAMPLE_RATE) as usize * 2) {
        let level = eg.tick(SAMPLE_RATE, *params, scale);
        if (level - hold_start_level).abs() > 1e-6 {
            if hold_samples > 2 {
                measured.push(hold_samples);
            }
            hold_start_level = level;
            hold_samples = 0;
        } else {
            hold_samples += 1;
        }
        if measured.len() >= 6 {
            break;
        }
    }

    println!(
        "[texture_sync_probe] bpm={bpm} target={target_seconds:.4}s ({expected_samples}samples) 実測(秒)={:?}",
        measured.iter().map(|&s| s as f32 / SAMPLE_RATE).collect::<Vec<_>>()
    );
    for &m in &measured {
        let diff = (m - expected_samples).abs();
        assert!(
            diff <= 2,
            "bpm={bpm}: ホールド長がSYNC音価とズレている expected={expected_samples} actual={m} diff={diff}"
        );
    }
    assert!(measured.len() >= 4, "bpm={bpm}: 観測できた周期数が少なすぎる: {}", measured.len());
    println!("[texture_sync_probe] bpm={bpm}: OK — {}周期ぶんSYNC音価(1/16)と一致（raw値0.37秒には一致しない）", measured.len());
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| ".".to_string());
    let out_dir = Path::new(&out_dir);
    std::fs::create_dir_all(out_dir).expect("出力ディレクトリ作成失敗");

    let gain_fg = build_gain_fg();

    for &bpm in &[90.0f32, 120.0f32, 160.0f32] {
        verify_sync_lock(&gain_fg, bpm);

        let patch = build_patch(gain_fg);
        let samples = render(patch, bpm);
        let path = out_dir.join(format!("sh_sync_1_16_bpm{:03}.wav", bpm as u32));
        write_wav(&path, &samples);
        println!("[texture_sync_probe] wrote {} (BPM {bpm}, 1/16 SYNC)", path.display());
    }
}

fn write_wav(path: &Path, samples: &[f32]) {
    let mut bytes = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("WAV書き込み失敗");
}
