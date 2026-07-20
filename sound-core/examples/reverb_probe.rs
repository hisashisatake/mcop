//! リバーブのオンセット過渡（「フッ」）の正体を切り分ける診断用の使い捨て例。
//!
//! 本物の `Reverb`（Hall1）に、
//!   (a) 急峻に立ち上がるサイン波
//!   (b) 40msかけて滑らかにフェードインするサイン波
//! を通し、それぞれのwet出力（`Reverb::process`の戻り値＝純wet）をWAVに書き出す。
//!
//! (b)で「フッ」が消えるなら犯人は音源のオンセット過渡（リバーブは無罪、
//! オールパス追加は無意味）。(b)でも残るならリバーブ固有の初期反射のまばらさが犯人。
//!
//! 実行: cargo run -p sound-core --example reverb_probe -- <出力ディレクトリ>

use std::f32::consts::PI;
use std::path::Path;

use sound_core::effects::{Reverb, ReverbType};

const SAMPLE_RATE: f32 = 44100.0;
const FREQ: f32 = 261.63; // C4
const NOTE_SECS: f32 = 0.5;
const TAIL_SECS: f32 = 3.5;

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let out_dir = Path::new(&out_dir);

    let fade_samples = (0.040 * SAMPLE_RATE) as usize;

    // (a) 急峻立ち上がり・急峻リリース: 常にフル振幅、0.5sで一段に切る。
    let abrupt = synth_sine(|_, _| 1.0);

    // (b) 40msフェードイン・急峻リリース: 立ち上がりだけ滑らか、切り方は瞬時。
    let faded_on = synth_sine(|i, note_len| {
        let attack = fade_in(i, fade_samples);
        let _ = note_len;
        attack
    });

    // (c) 40msフェードイン・40msフェードアウト: 両端を滑らかにする。
    let faded_both = synth_sine(|i, note_len| {
        let attack = fade_in(i, fade_samples);
        let release = fade_in(note_len - i, fade_samples); // 末尾からの距離でフェードアウト
        attack.min(release)
    });

    write_wav(&out_dir.join("probe_wet_abrupt.wav"), &reverb_wet(&abrupt));
    write_wav(&out_dir.join("probe_wet_faded.wav"), &reverb_wet(&faded_on));
    write_wav(&out_dir.join("probe_wet_faded_both.wav"), &reverb_wet(&faded_both));

    println!("wrote 3 probe WAVs to {}", out_dir.display());
}

/// raised-cosineの立ち上がり（0→1）。`i>=fade`では1.0。
fn fade_in(i: usize, fade: usize) -> f32 {
    if i >= fade {
        1.0
    } else {
        0.5 - 0.5 * (PI * i as f32 / fade as f32).cos()
    }
}

/// `env(i, note_len)` を振幅エンベロープとして 0.4×sin を生成（NOTE_SECS発音 + TAIL_SECS無音）。
fn synth_sine(env: impl Fn(usize, usize) -> f32) -> Vec<f32> {
    let note_len = (NOTE_SECS * SAMPLE_RATE) as usize;
    let total = ((NOTE_SECS + TAIL_SECS) * SAMPLE_RATE) as usize;
    (0..total)
        .map(|i| {
            if i < note_len {
                0.4 * env(i, note_len) * (2.0 * PI * FREQ * i as f32 / SAMPLE_RATE).sin()
            } else {
                0.0
            }
        })
        .collect()
}

/// dry信号を本物のReverb(Hall1, time=128)に通し、wet出力のみを返す。
fn reverb_wet(dry: &[f32]) -> Vec<f32> {
    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_type(ReverbType::Hall1);
    reverb.set_time(128);
    dry.iter()
        .map(|&s| {
            let (l, _r) = reverb.process(s, s);
            l
        })
        .collect()
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
