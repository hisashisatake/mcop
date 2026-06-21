//! SMF を `ym38x6-core` で再生し、mono f32 バッファへレンダリングする。
//!
//! ボイスIDは `midi_channel*128 + note` とし、エンジンの HashMap チャンネル管理で
//! ポリフォニーをそのまま扱う（VST と同じID符号化方式）。
//! プログラムチェンジ → `PatchBank::patch(program)` を音色として使う。

use ym38x6_core::{SoundEngine, Ym38x6Engine};

use crate::bank::PatchBank;
use crate::smf::{parse_smf, EvKind};

fn render_chunk(engine: &mut Ym38x6Engine, out: &mut Vec<f32>, rendered: &mut usize, target: usize) {
    if target > *rendered {
        let n = target - *rendered;
        let mut buf = vec![0.0f32; n];
        engine.render(&mut buf, 1);
        out.extend_from_slice(&buf);
        *rendered = target;
    }
}

/// SMF を `bank` を音色として再生し、mono f32 サンプル列を返す。
/// `tail_secs` はノートオフ後の残響を伸ばす秒数。
/// `max_secs` が `Some(s)` のとき、出力を `s` 秒（テール込み）で打ち切る（試聴の時短用）。
pub fn render_smf(
    data: &[u8],
    bank: &PatchBank,
    sample_rate: f32,
    tail_secs: f32,
    max_secs: Option<f32>,
) -> Result<Vec<f32>, String> {
    let (division, events) = parse_smf(data)?;

    let mut engine = Ym38x6Engine::new(sample_rate);
    let mut out: Vec<f32> = Vec::new();
    let mut programs = [0u8; 16];

    let mut tempo_us: f64 = 500_000.0; // 既定 120BPM
    let mut spt = tempo_us / 1_000_000.0 * sample_rate as f64 / division as f64; // samples/tick
    let mut cur_tick: u64 = 0;
    let mut sample_pos: f64 = 0.0;
    let mut rendered: usize = 0;

    let max_samples = max_secs.map(|s| (s * sample_rate).max(0.0) as usize);

    for e in &events {
        let dt = e.tick - cur_tick;
        sample_pos += dt as f64 * spt;
        cur_tick = e.tick;
        let target = sample_pos.floor() as usize;
        // 時短打ち切り: 上限に達したらそこまでレンダリングして以降のイベントは無視する。
        if let Some(maxs) = max_samples {
            if target >= maxs {
                render_chunk(&mut engine, &mut out, &mut rendered, maxs);
                return Ok(out);
            }
        }
        render_chunk(&mut engine, &mut out, &mut rendered, target);

        match e.kind {
            EvKind::Tempo(us) => {
                tempo_us = us as f64;
                spt = tempo_us / 1_000_000.0 * sample_rate as f64 / division as f64;
            }
            EvKind::Program(ch, p) => {
                programs[ch as usize] = p;
            }
            EvKind::NoteOn(ch, note, vel) => {
                let patch = bank.patch(programs[ch as usize]).clone();
                let id = ch as usize * 128 + note as usize;
                let freq = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
                engine.note_on_with_velocity(id, freq, vel, patch);
            }
            EvKind::NoteOff(ch, note) => {
                let id = ch as usize * 128 + note as usize;
                engine.note_off(id);
            }
        }
    }

    // 残響テール（max_secs 指定時は上限でクランプ）
    let mut tail_target = rendered + (sample_rate * tail_secs) as usize;
    if let Some(maxs) = max_samples {
        tail_target = tail_target.min(maxs);
    }
    render_chunk(&mut engine, &mut out, &mut rendered, tail_target);
    Ok(out)
}
