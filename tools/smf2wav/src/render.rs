//! SMF を `ym38x6-core` で再生し、mono f32 バッファへレンダリングする。
//!
//! ボイスIDは `midi_channel*128 + note` とし、エンジンの HashMap チャンネル管理で
//! ポリフォニーをそのまま扱う（VST と同じID符号化方式）。
//! プログラムチェンジ → `PatchBank::patch(program)` を音色として使う。
//!
//! 対応SMFイベント:
//!   - Note On/Off、Program Change、Tempo
//!   - Pitch Bend（チャンネル単位、RPN0,0でセンシティビティ設定可）
//!   - CC11 Expression: SSG音量制御（vgm2x6 SSGパッチのノイズOPを含む全アクティブOP更新）
//!   - CC64 Sustain Pedal（ホールドフラグ方式）
//!   - CC100/101/6: RPN選択とData Entry（ピッチベンド感度のみ）
//!   - CC120/CC123: All Sound Off / All Notes Off

use std::collections::HashMap;

use ym38x6_core::{SoundEngine, Ym38x6Engine, Ym38x6Patch};

use crate::bank::PatchBank;
use crate::smf::{parse_smf, EvKind};

/// CC11（Expression）の値 (0-127) → SSG OP の TL (0-255)。
/// vgm2x6 が `volume_to_cc11(vol) = vol*127/15` で書いた値の逆変換に相当する。
#[inline]
fn cc11_to_tl(cc11: u8) -> u8 {
    (cc11 as u16 * 255 / 127) as u8
}

/// パッチの SSG アクティブ OP マスクを返す（waveform≥16 かつ 元TL>0 のOP）。
/// vgm2x6 の WavSink と同じ判定基準。FM パッチ（waveform=0）は影響しない。
fn ssg_active_ops(patch: &Ym38x6Patch) -> [bool; 4] {
    std::array::from_fn(|i| patch.operators[i].waveform >= 16 && patch.operators[i].tl > 0)
}

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

    // ピッチベンド感度（半音）: RPN(0,0)で変更可。MIDI規格の既定値は±2半音。
    let mut pitch_bend_range = [2.0f32; 16];
    // RPN選択状態（CC101=MSB, CC100=LSB）。0xFF=未選択。
    let mut rpn_msb = [0xFFu8; 16];
    let mut rpn_lsb = [0xFFu8; 16];
    // サステインペダル（CC64）: ホールドフラグ + ペダル中のノートオフを保留するビットマスク。
    let mut pedal_down = [false; 16];
    let mut pending_release = [0u128; 16]; // bit N = note N が保留中

    // CC11（Expression）対応: SSG OP の TL をノートごとに音量スケールする。
    // vgm2x6 の SmfSink は CC11 でSSG音量を送るが、エンジンは TL 単位で音量を持つため
    // ここでノートごとのベースパッチを記憶し、CC11受信時に全発音ボイスのSSG OP TLを更新する。
    // キー: voice_id (ch*128+note)、値: note_on 時点のベースパッチ（TLはオリジナル値=255等）
    let mut note_base_patches: HashMap<usize, Ym38x6Patch> = HashMap::new();
    // チャンネルごとの直近 CC11 値（0-127）。初期値 127 = フル音量（TL=255）。
    let mut ch_cc11 = [127u8; 16];

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
                let base_patch = bank.patch(programs[ch as usize]).clone();
                let id = ch as usize * 128 + note as usize;
                let freq = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
                // CC11 が既に届いている場合はベースパッチの SSG OP TL をスケールしてから発音する。
                let mut patch = base_patch;
                let tl = cc11_to_tl(ch_cc11[ch as usize]);
                for i in 0..4 {
                    if patch.operators[i].waveform >= 16 && patch.operators[i].tl > 0 {
                        patch.operators[i].tl = tl;
                    }
                }
                note_base_patches.insert(id, base_patch);
                engine.note_on_with_velocity(id, freq, vel, patch);
            }
            EvKind::NoteOff(ch, note) => {
                let chi = ch as usize;
                let id = chi * 128 + note as usize;
                note_base_patches.remove(&id);
                if pedal_down[chi] {
                    // ペダル保持中: Note Offを保留する
                    pending_release[chi] |= 1u128 << note;
                } else {
                    engine.note_off(id);
                }
            }
            EvKind::PitchBend(ch, raw) => {
                let chi = ch as usize;
                // raw: -8192〜8191、8192=±1半音×pitch_bend_range半音
                let cents = raw as f32 / 8192.0 * pitch_bend_range[chi] * 100.0;
                engine.set_pitch_bend_group(chi, cents);
            }
            EvKind::ControlChange(ch, cc, val) => {
                let chi = ch as usize;
                match cc {
                    // RPN選択（CC101=MSB, CC100=LSB）
                    101 => rpn_msb[chi] = val,
                    100 => rpn_lsb[chi] = val,
                    // Data Entry MSB: RPN(0,0)のみ処理（ピッチベンド感度=半音数）
                    6 => {
                        if rpn_msb[chi] == 0 && rpn_lsb[chi] == 0 {
                            pitch_bend_range[chi] = val as f32;
                        }
                    }
                    // CC64 Sustain Pedal
                    64 => {
                        if val >= 64 {
                            pedal_down[chi] = true;
                        } else {
                            pedal_down[chi] = false;
                            // 保留中のNote Offをまとめて送出
                            let mut mask = pending_release[chi];
                            while mask != 0 {
                                let note = mask.trailing_zeros() as usize;
                                let id = chi * 128 + note;
                                note_base_patches.remove(&id);
                                engine.note_off(id);
                                mask &= mask - 1;
                            }
                            pending_release[chi] = 0;
                        }
                    }
                    // CC11 Expression: SSGパッチの全アクティブOP（トーン・ノイズ両方）の
                    // TL をスケールする。vgm2x6 が SSG ソフトエンベロープを CC11 で送る際に
                    // ノイズOP（mix_patchのoperators[2]等）も正しく音量制御されるようにする。
                    // ベースパッチを記憶した note_base_patches から SSGアクティブOPマスクを
                    // 引き直し、発音中の全ボイスに set_operator_params を適用する。
                    11 => {
                        ch_cc11[chi] = val;
                        let tl = cc11_to_tl(val);
                        for note in 0u8..128 {
                            let id = chi * 128 + note as usize;
                            if let Some(base) = note_base_patches.get(&id) {
                                let mask = ssg_active_ops(base);
                                for i in 0..4 {
                                    if mask[i] {
                                        let mut op = base.operators[i];
                                        op.tl = tl;
                                        engine.set_operator_params(id, i, op);
                                    }
                                }
                            }
                        }
                    }
                    // CC120 All Sound Off / CC123 All Notes Off
                    120 | 123 => {
                        for note in 0usize..128 {
                            let id = chi * 128 + note;
                            note_base_patches.remove(&id);
                            engine.note_off(id);
                        }
                        pending_release[chi] = 0;
                    }
                    _ => {}
                }
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
