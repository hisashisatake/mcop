//! mono / 16bit PCM の最小 WAV ライターと正規化ヘルパー。
//! `ym38x6/tools/smf2wav/src/wav.rs`・`ym38x6/tools/vgm2x6/src/play.rs`と完全同一実装
//! （デフォークによる複製。バイト出力を変えないことがゴールデンテストの前提）。

use std::path::Path;

/// ピーク正規化（既定 -6dBFS=0.5）。ほぼ無音の場合は何もしない。
pub fn normalize_peak(samples: &mut [f32], target_peak: f32) {
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak > 1e-4 {
        let norm = target_peak / peak;
        for s in samples {
            *s *= norm;
        }
    }
}

/// mono / 16bit PCM の最小 WAV ライター。
pub fn write_wav_mono16(path: &Path, samples: &[f32], sr: u32) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out)
}
