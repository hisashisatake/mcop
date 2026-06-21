//! 標準MIDIファイル（SMF）のパース。
//!
//! 外部クレート不使用。Format 0/1 のメトリカル・タイミング（division>0）に対応する。
//! ノートオン/オフ・プログラムチェンジ・ピッチベンド・コントロールチェンジ・テンポメタイベントを抽出する。

/// 再生に必要なイベント種別。
pub enum EvKind {
    NoteOn(u8, u8, u8),        // ch, note, vel
    NoteOff(u8, u8),           // ch, note
    Program(u8, u8),           // ch, program
    Tempo(u32),                // µs/四分音符
    PitchBend(u8, i16),        // ch, raw value (-8192〜8191、中心=0)
    ControlChange(u8, u8, u8), // ch, cc番号, value
}

/// 絶対 tick 付きイベント。
pub struct Ev {
    pub tick: u64,
    pub kind: EvKind,
}

fn read_vlq(data: &[u8], mut i: usize) -> (u64, usize) {
    let mut val: u64 = 0;
    loop {
        let b = data.get(i).copied().unwrap_or(0);
        i += 1;
        val = (val << 7) | (b & 0x7F) as u64;
        if b & 0x80 == 0 {
            break;
        }
    }
    (val, i)
}

/// SMF バイト列をパースし、(division, 全トラック統合イベント列) を返す。
pub fn parse_smf(data: &[u8]) -> Result<(u16, Vec<Ev>), String> {
    if data.len() < 14 || &data[0..4] != b"MThd" {
        return Err("MThd ヘッダがありません".to_string());
    }
    let ntrk = u16::from_be_bytes([data[10], data[11]]);
    let division = u16::from_be_bytes([data[12], data[13]]);
    if division & 0x8000 != 0 {
        return Err("SMPTE タイミング（division<0）は未対応です".to_string());
    }

    let mut events: Vec<Ev> = Vec::new();
    let mut i = 14usize;
    for _ in 0..ntrk {
        if i + 8 > data.len() || &data[i..i + 4] != b"MTrk" {
            break;
        }
        let tlen = u32::from_be_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        i += 8;
        let end = (i + tlen).min(data.len());
        let mut tick: u64 = 0;
        let mut status: u8 = 0;
        let mut j = i;
        while j < end {
            let (delta, nj) = read_vlq(data, j);
            j = nj;
            tick += delta;
            // ステータスバイト or ランニングステータス
            let mut b = data.get(j).copied().unwrap_or(0);
            if b & 0x80 != 0 {
                status = b;
                j += 1;
                b = data.get(j).copied().unwrap_or(0);
            }
            let _ = b;
            let ev = status & 0xF0;
            let ch = status & 0x0F;
            match ev {
                0x90 => {
                    let note = data.get(j).copied().unwrap_or(0);
                    let vel = data.get(j + 1).copied().unwrap_or(0);
                    j += 2;
                    events.push(Ev {
                        tick,
                        kind: if vel > 0 {
                            EvKind::NoteOn(ch, note, vel)
                        } else {
                            EvKind::NoteOff(ch, note)
                        },
                    });
                }
                0x80 => {
                    let note = data.get(j).copied().unwrap_or(0);
                    j += 2;
                    events.push(Ev { tick, kind: EvKind::NoteOff(ch, note) });
                }
                0xC0 => {
                    let p = data.get(j).copied().unwrap_or(0);
                    j += 1;
                    events.push(Ev { tick, kind: EvKind::Program(ch, p) });
                }
                0xD0 => {
                    j += 1;
                }
                0xA0 => {
                    j += 2; // Poly Key Pressure: 無視
                }
                0xB0 => {
                    let cc = data.get(j).copied().unwrap_or(0);
                    let val = data.get(j + 1).copied().unwrap_or(0);
                    j += 2;
                    events.push(Ev { tick, kind: EvKind::ControlChange(ch, cc, val) });
                }
                0xE0 => {
                    let lsb = data.get(j).copied().unwrap_or(0);
                    let msb = data.get(j + 1).copied().unwrap_or(0);
                    let raw14 = ((msb as u16) << 7) | (lsb as u16);
                    let raw = raw14 as i16 - 8192;
                    j += 2;
                    events.push(Ev { tick, kind: EvKind::PitchBend(ch, raw) });
                }
                _ => {
                    // 0xF0: メタ or sysex
                    if status == 0xFF {
                        let meta = data.get(j).copied().unwrap_or(0);
                        j += 1;
                        let (mlen, nj) = read_vlq(data, j);
                        j = nj;
                        if meta == 0x51 && mlen == 3 {
                            let us = ((data.get(j).copied().unwrap_or(0) as u32) << 16)
                                | ((data.get(j + 1).copied().unwrap_or(0) as u32) << 8)
                                | (data.get(j + 2).copied().unwrap_or(0) as u32);
                            events.push(Ev { tick, kind: EvKind::Tempo(us) });
                        }
                        j += mlen as usize;
                    } else {
                        // F0/F7 sysex
                        let (slen, nj) = read_vlq(data, j);
                        j = nj;
                        j += slen as usize;
                    }
                }
            }
        }
        i = end;
    }

    // tick 昇順に安定ソート（同 tick 内はトラック→出現順を保持。Program が NoteOn より先に来る）
    events.sort_by_key(|e| e.tick);
    Ok((division, events))
}
