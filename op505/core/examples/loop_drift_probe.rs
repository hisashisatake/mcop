//! TimeEgループドリフト（`level_drift`/`depth_drift`）の聴感プローブ。
//!
//! OP1のみを鳴らすキャリア1つに「アタック→2段ループ（振れ幅トレモロ）→リリース」の
//! TimeEgを組み、`level_drift`/`depth_drift`を多段に振ったWAV群を書き出す。
//! `level_drift_per_cycle`/`depth_drift_per_cycle`（`sound-core/src/time_eg.rs`）の
//! 指数カーブの実用域を耳で判定するための使い捨てツール（memory
//! `project_timeeg_loop_drift.md`参照）。
//!
//! 実行: cargo run -p op505-core --example loop_drift_probe -- <出力ディレクトリ>

use std::path::Path;

use op505_core::{Op505ChannelParams, Op505Engine, Op505OperatorParams, Op505Patch};
use sound_core::{seconds_to_time, TimeEgParams, TimeStage, Vco, MAX_STAGES};

const SAMPLE_RATE: f32 = 44100.0;
const NOTE_FREQ: f32 = 220.0; // A3
const HOLD_SECS: f32 = 9.0; // ループ0.4秒/周 x 約22周
const RELEASE_TAIL_SECS: f32 = 1.5;

// bpf一本に絞れたので、level_driftの上昇方向（128超）も含めてもう少し広く網羅する。
const LEVEL_DRIFT_FOCUS: &[u8] = &[8, 32, 64, 96, 112, 120, 136, 144, 160, 192, 224];
const DEPTH_DRIFT_FOCUS: &[u8] = &[64, 96, 112, 120, 136, 144, 160, 192];

/// 音色バリエーション（フィルター設定）。`label`はファイル名prefix。
struct Timbre {
    label: &'static str,
    filter_type: u8,
    filter_cutoff: u8,
    filter_resonance: u8,
}

const TIMBRES: &[Timbre] = &[
    // BPF（≈1.1kHz、レゾナンスあり）。聴感プローブでraw/lpf/hpfより判別しやすいと確認済み。
    Timbre { label: "bpf", filter_type: 2, filter_cutoff: 150, filter_resonance: 100 },
];

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| ".".to_string());
    let out_dir = Path::new(&out_dir);
    std::fs::create_dir_all(out_dir).expect("出力ディレクトリ作成失敗");

    for timbre in TIMBRES {
        println!("[loop_drift_probe] timbre={} level_drift sweep（depth_drift=128中立固定）", timbre.label);
        for &level_drift in LEVEL_DRIFT_FOCUS {
            let patch = build_patch(timbre, level_drift, 128);
            let path = out_dir.join(format!("{}_level_drift_{level_drift:03}.wav", timbre.label));
            write_wav(&path, &render(patch));
            println!("  wrote {}", path.display());
        }

        println!("[loop_drift_probe] timbre={} depth_drift sweep（level_drift=128中立固定）", timbre.label);
        for &depth_drift in DEPTH_DRIFT_FOCUS {
            let patch = build_patch(timbre, 128, depth_drift);
            let path = out_dir.join(format!("{}_depth_drift_{depth_drift:03}.wav", timbre.label));
            write_wav(&path, &render(patch));
            println!("  wrote {}", path.display());
        }
    }
}

/// OP1のみが鳴るキャリア1本のパッチ。TimeEgは
/// stage0=アタック(255) → stage1=谷(180) → stage2=山(255、ここがループ折り返し点) →
/// stage3=リリース(0)。`loop_start=1..=release_point=2`を周回する2段ループの振れ幅トレモロに、
/// 指定した`level_drift`/`depth_drift`を掛ける。
fn build_patch(timbre: &Timbre, level_drift: u8, depth_drift: u8) -> Op505Patch {
    let attack_time = seconds_to_time(0.05);
    let leg_time = seconds_to_time(0.2); // 1周=2レグ=0.4秒
    let release_time = seconds_to_time(1.2);

    // 山(220)・谷(140)とも0/255から離しておく。どちらかがクランプ端(0.0/1.0)に
    // 張り付いていると、その側へのドリフトが天井/床で見えなくなってしまうため。
    let mut stages = [TimeStage::default(); MAX_STAGES];
    stages[0] = TimeStage { time: attack_time, level: 220, curve: 0 };
    stages[1] = TimeStage { time: leg_time, level: 140, curve: 1 };
    stages[2] = TimeStage { time: leg_time, level: 220, curve: 1 };
    stages[3] = TimeStage { time: release_time, level: 0, curve: 0 };

    let eg = TimeEgParams {
        stages,
        stage_count: 4,
        loop_enabled: 1,
        loop_start: 1,
        release_point: 2,
        level_drift,
        depth_drift,
        ..TimeEgParams::default()
    };

    let mut patch = Op505Patch::default();
    // waveform=8はノコギリ波（サイン波よりトレモロ/ドリフトの振幅変化が判別しやすい）。
    patch.operators[0] =
        Op505OperatorParams { tl: 255, mul: 1, waveform: 8, eg, ..Op505OperatorParams::default() };
    patch.channel = Op505ChannelParams::default();
    // Algorithm 7 = 全OPが独立キャリア（加算合成）。OP2〜4はtl=0のまま無音なので、
    // OP1のみが鳴る（`loud_patch`と同じ組み方。デフォルトのAlgorithm 0はOP1が
    // キャリアでないため、TL=255にしても出力に現れず無音になる）。
    patch.channel.algorithm = 7;
    patch.channel.filter_type = timbre.filter_type;
    patch.channel.filter_cutoff = timbre.filter_cutoff;
    patch.channel.filter_resonance = timbre.filter_resonance;
    patch.channel.filter_self_oscillation = false;
    patch
}

fn render(patch: Op505Patch) -> Vec<f32> {
    let mut engine = Op505Engine::new(SAMPLE_RATE);
    engine.set_patch(patch);
    engine.note_on(0, NOTE_FREQ, 100);
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = vec![0.0f32; hold_samples + release_samples];
    engine.render(&mut out[..hold_samples], 1);
    engine.note_off(0);
    engine.render(&mut out[hold_samples..], 1);
    out
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
