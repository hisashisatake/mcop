//! 標準MIDIファイル（SMF）のパース。
//!
//! 外部クレート不使用。Format 0/1 のメトリカル・タイミング（division>0）に対応する。
//! ノートオン/オフ・プログラムチェンジ・ピッチベンド・コントロールチェンジ・テンポメタイベントを抽出する。
//! `ym38x6/tools/smf2wav/src/smf.rs`と同一実装（チップ非依存のためfork-on-writeでも無変更複製）。

/// 再生に必要なイベント種別。
pub enum EvKind {
    NoteOn(u8, u8, u8),        // ch, note, vel
    NoteOff(u8, u8),           // ch, note
    Program(u8, u8),           // ch, program
    Tempo(u32),                // µs/四分音符
    PitchBend(u8, i16),        // ch, raw value (-8192〜8191、中心=0)
    ControlChange(u8, u8, u8), // ch, cc番号, value
    ChannelPressure(u8, u8),   // ch, value（AT Destinationの加算源、NRPN(0,16)参照）
    PolyPressure(u8, u8, u8),  // ch, note, value（Poly AT Destination、NRPN(0,17)参照）
    /// 完全な1個のSysExメッセージ（先頭`0xF0`・末尾`0xF7`を含む）。GM2 Universal SysEx
    /// Master Volume等の解釈は呼び出し側（render.rs）が`sound_midi::parse_universal_sysex`
    /// で行う。
    SysEx(Vec<u8>),
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
                    let value = data.get(j).copied().unwrap_or(0);
                    j += 1;
                    events.push(Ev { tick, kind: EvKind::ChannelPressure(ch, value) });
                }
                0xA0 => {
                    let note = data.get(j).copied().unwrap_or(0);
                    let value = data.get(j + 1).copied().unwrap_or(0);
                    j += 2;
                    events.push(Ev { tick, kind: EvKind::PolyPressure(ch, note, value) });
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
                    } else if status == 0xF0 {
                        // SMF内のF0イベントは「0xF0 <VLQ長> <payload>」の形式で、0xF0自体は
                        // 既にstatusとして消費済み。payload末尾に0xF7を含むのが通例のため、
                        // 先頭へ0xF0を付け直して完全な1メッセージへ再構成する。
                        let (slen, nj) = read_vlq(data, j);
                        j = nj;
                        let payload_end = (j + slen as usize).min(data.len());
                        let mut full = Vec::with_capacity(1 + (payload_end - j));
                        full.push(0xF0);
                        full.extend_from_slice(&data[j..payload_end]);
                        events.push(Ev { tick, kind: EvKind::SysEx(full) });
                        j += slen as usize;
                    } else {
                        // 0xF7単体（分割SysExの継続）は未対応のため読み飛ばす。
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

#[cfg(test)]
mod tests {
    use super::*;

    fn build_minimal_smf(track_events: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&6u32.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes()); // format 0
        data.extend_from_slice(&1u16.to_be_bytes()); // ntrk=1
        data.extend_from_slice(&96u16.to_be_bytes()); // division
        data.extend_from_slice(b"MTrk");
        data.extend_from_slice(&(track_events.len() as u32).to_be_bytes());
        data.extend_from_slice(track_events);
        data
    }

    /// SMF内のF0イベント（0xF0 <VLQ長> <payload>、0xF0自体はstatusとして消費済み）から
    /// 完全な1メッセージ（先頭0xF0付き）が再構成されること。
    #[test]
    fn parses_sysex_event_and_reconstructs_full_message() {
        let mut track = vec![0x00, 0xF0, 0x07]; // delta=0, F0, len=7
        track.extend_from_slice(&[0x7F, 0x7F, 0x04, 0x01, 0x00, 0x7F, 0xF7]);
        let smf = build_minimal_smf(&track);

        let (_division, events) = parse_smf(&smf).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].kind {
            EvKind::SysEx(bytes) => {
                assert_eq!(bytes, &vec![0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x7F, 0xF7]);
            }
            _ => panic!("expected SysEx event"),
        }
    }

    /// SysExイベントの後に続くイベント（NoteOn）の解釈がずれないこと
    /// （VLQ長の読み進めが正しいことの間接証明）。
    #[test]
    fn sysex_event_does_not_disturb_following_events() {
        let mut track = vec![0x00, 0xF0, 0x03, 0x00, 0x00, 0xF7]; // 短いダミーSysEx
        track.extend_from_slice(&[0x00, 0x90, 60, 100]); // delta=0, NoteOn ch0 note60 vel100
        let smf = build_minimal_smf(&track);

        let (_division, events) = parse_smf(&smf).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].kind, EvKind::SysEx(_)));
        assert!(matches!(events[1].kind, EvKind::NoteOn(0, 60, 100)));
    }

    /// メタイベント（0xFF）は従来どおりSysExと区別されテンポとして解釈される
    /// （0xF0/0xFFの分岐が壊れていないことの回帰確認）。
    #[test]
    fn meta_tempo_event_is_not_treated_as_sysex() {
        let mut track = vec![0x00, 0xFF, 0x51, 0x03]; // delta=0, Meta, Tempo, len=3
        track.extend_from_slice(&[0x07, 0xA1, 0x20]); // 500000us = 120BPM
        let smf = build_minimal_smf(&track);

        let (_division, events) = parse_smf(&smf).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, EvKind::Tempo(500_000)));
    }
}
