//! `Operator::compute_env_amp`のキャッシュ判定を厳密一致→許容誤差比較へ変更した際の
//! 出力差を、フィードバック(FB)の有無で切り分ける診断用の使い捨て例。
//! algorithm=0（O1→O2→O3→O4直列、feedback_op=0）の同一パッチをFB値だけ変えて鳴らし、
//! 新旧のoperator.rsをgit stashで切り替えながら同じ引数で実行し、出力WAVを比較する。
//!
//! 実行: cargo run --release -p op505-core --example env_amp_change_probe -- <fb 0-255> <出力wavパス>

use std::path::Path;

use op505_core::{Op505Engine, Op505Patch};
use sound_core::{TimeStage, Vco};

const SAMPLE_RATE: f32 = 44100.0;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let fb: u8 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let out_path = args.get(2).cloned().unwrap_or_else(|| "out.wav".to_string());

    let mut engine = Op505Engine::new(SAMPLE_RATE);
    let mut patch = Op505Patch::default();
    patch.channel.algorithm = 0;
    patch.channel.feedback = fb;
    patch.channel.filter_cutoff = 255;
    for (i, op) in patch.operators.iter_mut().enumerate() {
        op.tl = 40 + i as u8 * 15;
        op.mul = 1 + i as u8;
        op.eg.stage_count = 4;
        op.eg.stages[0] = TimeStage { time: 20, level: 255, curve: 0 };
        op.eg.stages[1] = TimeStage { time: 100, level: 180, curve: 0 };
        op.eg.stages[2] = TimeStage { time: 0, level: 180, curve: 0 };
        op.eg.stages[3] = TimeStage { time: 150, level: 0, curve: 0 };
        op.eg.release_point = 2;
    }
    engine.set_patch(patch);

    // 実曲に近い複雑さを再現するため和音+複数回のノート切り替えを行う。
    let chords: [[f32; 3]; 3] =
        [[220.0, 277.18, 329.63], [246.94, 311.13, 369.99], [196.00, 246.94, 293.66]];
    let mut buf = vec![0.0f32; 2048];
    for (chord_idx, chord) in chords.iter().enumerate() {
        for (i, &freq) in chord.iter().enumerate() {
            engine.note_on(chord_idx * 3 + i, freq, 90 + (i as u8) * 10);
        }
        engine.render(&mut buf, 1);
        for i in 0..chord.len() {
            engine.note_off(chord_idx * 3 + i);
        }
        engine.render(&mut buf, 1);
    }

    let tail = (SAMPLE_RATE * 2.0) as usize;
    let mut out = vec![0.0f32; tail];
    engine.render(&mut out, 1);

    write_wav(Path::new(&out_path), &out);
    println!("wrote {out_path} (fb={fb})");
}

fn write_wav(path: &Path, samples: &[f32]) {
    let mut bytes = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
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
