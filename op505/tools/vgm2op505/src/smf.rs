//! SMF（Standard MIDI File）バイナリライター。外部クレート不使用。
//!
//! Format 1（マルチトラック）を出力する:
//!   Track 0: テンポ・メタ情報
//!   Track 1-8: OPM チャンネル 0-7
//!
//! 由来: ym38x6/tools/vgm2x6/src/smf.rs（コミット535604f時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! vgm2x6側の修正は自動では反映されない（`git diff 535604f -- ym38x6/tools/vgm2x6/src/smf.rs`
//! で追従漏れを確認できる）。このファイル自体はym38x6型に一切触れないため、複製は無変更。

use std::io::Write;
use std::path::Path;

const PPQ: u16 = 480;
// 120 BPM = 500000 µs/beat
const TEMPO_US: u32 = 500_000;
const SAMPLE_RATE: u32 = 44_100;

pub struct SmfBuilder {
    // tracks[0] = テンポトラック、tracks[1..9] = OPMチャンネル 0-7
    tracks: Vec<Vec<(u64, Vec<u8>)>>, // Vec of (absolute_tick, event_bytes)
}

impl SmfBuilder {
    pub fn new() -> Self {
        // トラック0（テンポ）+ トラック1-8（チャンネル）= 9トラック
        let mut tracks = Vec::with_capacity(9);
        for _ in 0..9 {
            tracks.push(Vec::new());
        }
        Self { tracks }
    }

    /// サンプル数を MIDI ティック数に変換する（120BPM、PPQ=480、44100Hz基準）。
    pub fn samples_to_ticks(samples: u64) -> u64 {
        // ticks = samples × PPQ × (1e6 / TEMPO_US) / SAMPLE_RATE
        //       = samples × 480 × 2 / 44100
        ((samples as f64 * PPQ as f64 * 1_000_000.0)
            / (TEMPO_US as f64 * SAMPLE_RATE as f64))
            .round() as u64
    }

    // --- イベント追加 ---

    /// 指定トラックが存在するようトラック配列を伸長する（OPN系の9ch等、9トラックを超える場合に対応）。
    fn ensure_track(&mut self, track: usize) {
        while self.tracks.len() <= track {
            self.tracks.push(Vec::new());
        }
    }

    pub fn add_note_on(&mut self, track: usize, tick: u64, ch: u8, note: u8, vel: u8) {
        self.ensure_track(track);
        self.tracks[track].push((tick, vec![0x90 | ch, note, vel]));
    }

    pub fn add_note_off(&mut self, track: usize, tick: u64, ch: u8, note: u8) {
        self.ensure_track(track);
        self.tracks[track].push((tick, vec![0x80 | ch, note, 0]));
    }

    /// MIDI ピッチベンドを送出する（value: 0-16383, center=8192）。
    pub fn add_pitch_bend(&mut self, track: usize, tick: u64, ch: u8, value: i16) {
        self.ensure_track(track);
        let v = value as u16;
        let lsb = (v & 0x7F) as u8;
        let msb = ((v >> 7) & 0x7F) as u8;
        self.tracks[track].push((tick, vec![0xE0 | ch, lsb, msb]));
    }

    pub fn add_program_change(&mut self, track: usize, tick: u64, ch: u8, program: u8) {
        self.ensure_track(track);
        self.tracks[track].push((tick, vec![0xC0 | ch, program & 0x7F]));
    }

    pub fn add_cc(&mut self, track: usize, tick: u64, ch: u8, cc: u8, val: u8) {
        self.ensure_track(track);
        self.tracks[track].push((tick, vec![0xB0 | ch, cc, val]));
    }

    /// RPN 0（ピッチベンド感度）を設定する。`semitones` は感度の半音数。
    /// RPN null（CC101/100=127）は送らない：同一tickに重複CCがあるとDominoが最後の値のみ
    /// 送信し CC6=semitones が届かないため。vgm2x6 SMF は CC6 を他に使わないので問題ない。
    pub fn set_pitch_bend_sensitivity(&mut self, track: usize, ch: u8, semitones: u8) {
        self.add_cc(track, 0, ch, 101, 0);       // RPN MSB = 0
        self.add_cc(track, 0, ch, 100, 0);       // RPN LSB = 0
        self.add_cc(track, 0, ch, 6, semitones); // Data Entry MSB = semitones
    }

    fn add_meta(&mut self, track: usize, tick: u64, meta_type: u8, data: &[u8]) {
        self.ensure_track(track);
        let mut ev = vec![0xFF, meta_type];
        write_vlq_to_vec(&mut ev, data.len() as u64);
        ev.extend_from_slice(data);
        self.tracks[track].push((tick, ev));
    }

    /// SMF Format 1 のバイト列を構築する（ファイルI/Oを伴わない。テスト・埋め込み用途）。
    pub fn to_bytes(&mut self) -> Vec<u8> {
        // テンポトラックを構築
        let total_ticks = self.tracks.iter()
            .flat_map(|t| t.iter())
            .map(|(tick, _)| *tick)
            .max()
            .unwrap_or(0);

        // テンポ: 0xFF 0x51 0x03 [3bytes]
        let t = TEMPO_US.to_be_bytes();
        self.add_meta(0, 0, 0x51, &t[1..4]); // 上位1バイトを除いた3バイト

        // 拍子記号 4/4
        self.add_meta(0, 0, 0x58, &[4, 2, 24, 8]);

        // 全トラックに End of Track を追加
        let num_tracks = self.tracks.len();
        for i in 0..num_tracks {
            self.add_meta(i, total_ticks, 0x2F, &[]);
        }

        let mut buf = Vec::new();

        // MThd ヘッダー
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());                  // chunk length
        buf.extend_from_slice(&1u16.to_be_bytes());                  // format 1
        buf.extend_from_slice(&(num_tracks as u16).to_be_bytes());   // トラック数
        buf.extend_from_slice(&PPQ.to_be_bytes());                   // PPQ

        // 各トラック
        for track_events in &mut self.tracks {
            track_events.sort_by_key(|(tick, _)| *tick);
            let track_data = serialize_track(track_events);
            buf.extend_from_slice(b"MTrk");
            buf.extend_from_slice(&(track_data.len() as u32).to_be_bytes());
            buf.extend_from_slice(&track_data);
        }

        buf
    }

    /// SMF Format 1 をファイルに書き出す。
    pub fn write(&mut self, path: &Path) -> std::io::Result<()> {
        let buf = self.to_bytes();
        let mut f = std::fs::File::create(path)?;
        f.write_all(&buf)
    }
}

/// トラックイベント列を SMF トラックデータ（デルタタイム + イベント）に変換する。
fn serialize_track(events: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut prev_tick: u64 = 0;
    for (tick, data) in events {
        let delta = tick.saturating_sub(prev_tick);
        write_vlq_to_vec(&mut buf, delta);
        buf.extend_from_slice(data);
        prev_tick = *tick;
    }
    buf
}

/// 可変長数値（VLQ）を Vec に書き出す。
fn write_vlq_to_vec(buf: &mut Vec<u8>, mut value: u64) {
    let mut bytes = [0u8; 10];
    let mut n = 0usize;
    loop {
        bytes[n] = (value & 0x7F) as u8;
        value >>= 7;
        n += 1;
        if value == 0 { break; }
    }
    for i in (0..n).rev() {
        if i > 0 {
            buf.push(bytes[i] | 0x80);
        } else {
            buf.push(bytes[i]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_to_ticks_at_120bpm() {
        // 1秒 = 44100サンプル → 960ティック (120BPM × PPQ480 / 60)
        let ticks = SmfBuilder::samples_to_ticks(44100);
        assert!((ticks as i64 - 960).abs() <= 1, "got {ticks}");
    }

    #[test]
    fn vlq_single_byte() {
        let mut buf = Vec::new();
        write_vlq_to_vec(&mut buf, 60);
        assert_eq!(buf, vec![60]);
    }

    #[test]
    fn vlq_multi_byte() {
        let mut buf = Vec::new();
        write_vlq_to_vec(&mut buf, 0x80); // → 0x81 0x00
        assert_eq!(buf, vec![0x81, 0x00]);
    }
}
