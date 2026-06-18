//! VGM/VGZ ファイルのパーサー。
//!
//! VGM = "Video Game Music" バイナリフォーマット。
//! チップへのレジスタ書き込みとタイミング（44100Hzサンプル単位）を記録。
//! VGZ = gzip圧縮済みVGM。

use std::io::Read;

pub struct VgmHeader {
    pub ym2151_clock: u32,
    pub data_start: usize,
    pub total_samples: u32,
}

/// VGZ/VGMをデコードしてバイト列を返す。
pub fn load(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;

    // VGZ（gzip圧縮）か否かをマジックで判定
    if raw.starts_with(&[0x1F, 0x8B]) {
        let mut dec = flate2::read::GzDecoder::new(raw.as_slice());
        let mut data = Vec::new();
        dec.read_to_end(&mut data)
            .map_err(|e| format!("gzip解凍に失敗: {e}"))?;
        Ok(data)
    } else {
        Ok(raw)
    }
}

/// VGMヘッダーを解析する。
pub fn parse_header(data: &[u8]) -> Result<VgmHeader, String> {
    if data.len() < 0x40 {
        return Err("VGMヘッダーが短すぎます".into());
    }
    if &data[0..4] != b"Vgm " {
        return Err("VGMマジックが一致しません".into());
    }

    let ym2151_clock = u32_le(data, 0x30);
    let total_samples = u32_le(data, 0x18);

    // データ開始オフセット: v1.50以降は 0x34 の相対値、それ以前は固定 0x40
    let version = u32_le(data, 0x08);
    let data_start = if version >= 0x150 {
        let rel = u32_le(data, 0x34);
        if rel == 0 { 0x40 } else { 0x34 + rel as usize }
    } else {
        0x40
    };

    if data_start >= data.len() {
        return Err("データ開始オフセットがファイルサイズを超えています".into());
    }

    Ok(VgmHeader { ym2151_clock, data_start, total_samples })
}

#[inline]
fn u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// コマンドイテレーター
// ---------------------------------------------------------------------------

pub enum VgmCmd {
    Ym2151Write { reg: u8, val: u8 },
    Wait(u32),
    End,
}

pub struct VgmIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> VgmIter<'a> {
    pub fn new(data: &'a [u8], start: usize) -> Self {
        Self { data, pos: start }
    }

    fn byte(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }

    fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.data.len());
    }

    fn u32_le(&mut self) -> u32 {
        let b0 = self.byte().unwrap_or(0) as u32;
        let b1 = self.byte().unwrap_or(0) as u32;
        let b2 = self.byte().unwrap_or(0) as u32;
        let b3 = self.byte().unwrap_or(0) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }
}

impl<'a> Iterator for VgmIter<'a> {
    type Item = VgmCmd;

    fn next(&mut self) -> Option<VgmCmd> {
        loop {
            let cmd = self.byte()?;
            match cmd {
                // YM2151 ポート0 書き込み
                0x54 => {
                    let reg = self.byte()?;
                    let val = self.byte()?;
                    return Some(VgmCmd::Ym2151Write { reg, val });
                }

                // N サンプル待つ (16-bit LE)
                0x61 => {
                    let lo = self.byte()? as u32;
                    let hi = self.byte()? as u32;
                    return Some(VgmCmd::Wait(lo | (hi << 8)));
                }

                // 735 サンプル待つ (1/60 秒)
                0x62 => return Some(VgmCmd::Wait(735)),

                // 882 サンプル待つ (1/50 秒)
                0x63 => return Some(VgmCmd::Wait(882)),

                // データ終端
                0x66 => return Some(VgmCmd::End),

                // データブロック: 0x67 0x66 tt [4-byte len] [data]
                0x67 => {
                    self.byte(); // 0x66 固定
                    self.byte(); // type
                    let len = self.u32_le() & 0x7FFFFFFF;
                    self.skip(len as usize);
                }

                // PCM RAM書き込み (VGM 1.70): 11バイト固定
                0x68 => { self.skip(11); }

                // 短縮ウェイト: 1-16サンプル
                0x70..=0x7F => return Some(VgmCmd::Wait((cmd & 0x0F) as u32 + 1)),

                // YM2612 DAC + short wait (YM2612専用、スキップ)
                0x80..=0x8F => {}

                // その他の2バイト引数コマンド
                0x30..=0x3F | 0x4F | 0x50 => { self.byte(); }

                // その他の3バイト引数コマンド (register + data)
                0x40..=0x4E | 0x51..=0x5F | 0xA0..=0xBF => {
                    self.byte();
                    self.byte();
                }

                // 4バイト引数コマンド
                0xC0..=0xDF => {
                    self.byte();
                    self.byte();
                    self.byte();
                }

                // 5バイト引数コマンド
                0xE0..=0xFF => {
                    self.byte();
                    self.byte();
                    self.byte();
                    self.byte();
                }

                // seek: 4バイト
                0x90..=0x95 => { self.skip(4); }

                _ => {} // 不明コマンドはスキップ
            }
        }
    }
}
