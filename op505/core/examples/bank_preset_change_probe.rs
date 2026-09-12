//! `.op505`バンクファイルから実際のプリセットを1つ読み込んでnote_on〜note_offし、
//! `Operator::compute_env_amp`のキャッシュ変更前後の出力差をFB値ごとに検証する診断用の
//! 使い捨て例（[[project_env_amp_cache_epsilon_perf_fix]]の続き、Bank B版）。
//!
//! 実行: cargo run --release -p op505-core --example bank_preset_change_probe -- <bank.op505> <program 0-127> <出力wavパス>

use std::path::Path;

use op505_core::{Op505Engine, Op505PresetBank};
use sound_core::Vco;

const SAMPLE_RATE: f32 = 44100.0;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bank_path = args.get(1).expect("引数1: bank.op505");
    let program: u8 = args.get(2).and_then(|s| s.parse().ok()).expect("引数2: program 0-127");
    let out_path = args.get(3).cloned().unwrap_or_else(|| "out.wav".to_string());

    let bank = Op505PresetBank::load_from_file(Path::new(bank_path)).expect("バンク読み込み失敗");
    let preset = bank.get(0, program).unwrap_or_else(|| panic!("program={program} が見つからない"));
    println!("音色: {} (program={program}, feedback={})", preset.name, preset.patch.channel.feedback);

    let mut engine = Op505Engine::new(SAMPLE_RATE);
    engine.set_patch(preset.patch);

    // 実曲に近い複雑さを再現するため和音+複数回のノート切り替えを行う。
    let chords: [[f32; 3]; 3] =
        [[220.0, 277.18, 329.63], [246.94, 311.13, 369.99], [196.00, 246.94, 293.66]];
    let mut buf = vec![0.0f32; 4096];
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

    let tail = (SAMPLE_RATE * 2.5) as usize;
    let mut out = vec![0.0f32; tail];
    engine.render(&mut out, 1);

    write_wav(Path::new(&out_path), &out);
    println!("wrote {out_path}");
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
