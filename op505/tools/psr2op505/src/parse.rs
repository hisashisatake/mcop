//! PSR-70 サウンドROM（`ROM2.bin`、32KB）からのOPQ音色テーブル抽出。
//!
//! 由来: ym38x6/tools/psr2x6/src/rom2.rs（コミット8d9c6bb時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! psr2x6側の修正は自動では反映されない（`git diff 8d9c6bb -- ym38x6/tools/psr2x6/src/rom2.rs`
//! で追従漏れを確認できる）。
//!
//! 工程0の解析（`private/`のROM2逆アセンブリ・自己相関・統計プロファイリング）で確定した構造：
//! - 音色テーブルは `0x0660` から **68音色 × 64バイト**（末尾 `0x1760` はコード境界）。
//! - 1音色は **パラメーター主体レイアウト（16行 × 4オペレーター）**。`byte(row, op) = rec[row*4 + op]`。
//!
//! 64バイト各行 → OPQパラメーター対応（ymfmのOPQレジスタ配置と統計で確定／一部は試聴で要確定）：
//! - row0 (+00): Detune（6bit, 0x20=中心）
//! - row1 (+04): MUL（下位4bit。bit7は常時セットのため無視）
//! - row2 (+08): Total Level（7bit）
//! - row4 (+10): D2R/Sustain Rate（5bit）※暫定（試聴で確定）
//! - row5 (+14): KSR(bit6-7) + Attack Rate(bit0-4)
//! - row6 (+18): AM-EN(bit7) + 波形(bit6) + D1R(bit0-4)
//! - row8 (+20): D1L(上位4bit) + RR(下位4bit)※暫定（試聴で確定）
//! - +0x3B: Feedback(bit3-5) + Algorithm(bit0-2)（全68音色でalg 0〜6に分散。+0x32は出力L/R用で下位3bit常時0）
//!
//! ROM2.bin自体は著作物のため本クレートに同梱せず、パス引数で受け取る。

use crate::map::{NamedVoice, OpqOperator, OpqVoice};

/// 音色テーブルの先頭オフセット（ROM2.bin内）。
pub const VOICE_TABLE_OFFSET: usize = 0x0660;
/// 音色数。
pub const VOICE_COUNT: usize = 68;
/// 1音色のバイト数。
pub const VOICE_SIZE: usize = 64;

/// ROM2バイナリ（32KB）から68音色を抽出する。
pub fn parse_rom2_voices(rom: &[u8]) -> Result<Vec<NamedVoice>, String> {
    let end = VOICE_TABLE_OFFSET + VOICE_COUNT * VOICE_SIZE;
    if rom.len() < end {
        return Err(format!(
            "ROM2が小さすぎます: {}バイト（音色テーブル末尾 {:#06X} に届かない）",
            rom.len(),
            end
        ));
    }
    let mut voices = Vec::with_capacity(VOICE_COUNT);
    for i in 0..VOICE_COUNT {
        let base = VOICE_TABLE_OFFSET + i * VOICE_SIZE;
        voices.push(NamedVoice {
            name: format!("PSR70 {i:02}"),
            voice: decode_voice(&rom[base..base + VOICE_SIZE]),
        });
    }
    Ok(voices)
}

/// 64バイトの1音色レコード → [`OpqVoice`]。
fn decode_voice(rec: &[u8]) -> OpqVoice {
    // byte(row, op)。パラメーター主体（各行=4オペレーター）。
    let b = |row: usize, op: usize| rec[row * 4 + op];

    let mut operators = [OpqOperator::default(); 4];
    for (op, slot) in operators.iter_mut().enumerate() {
        let r5 = b(5, op); // KSR + AR
        let r6 = b(6, op); // AM-EN + 波形 + D1R
        let r8 = b(8, op); // D1L + RR
        *slot = OpqOperator {
            detune: b(0, op) & 0x3F,
            mul: b(1, op) & 0x0F,
            tl: b(2, op) & 0x7F,
            ar: r5 & 0x1F,
            ksr: (r5 >> 6) & 0x03,
            d1r: r6 & 0x1F,
            am_enable: (r6 >> 7) & 1 == 1,
            d2r: b(4, op) & 0x1F,    // 暫定
            d1l: (r8 >> 4) & 0x0F,   // 暫定
            rr: r8 & 0x0F,           // 暫定
        };
    }

    // チャンネル: +0x3B = Feedback(3-5) + Algorithm(0-2)
    // （出力L/R(+0x32)とは別バイト。reg 0x10の出力bitを除いた FB+ALG 相当）
    let ch = rec[0x3B];
    OpqVoice {
        operators,
        algorithm: ch & 0x07,
        feedback: (ch >> 3) & 0x07,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 既知のサンプル音色（解析時に 0x0A60 で観測した値）を直接デコードし、
    /// 各フィールドが想定どおり取り出せることを確認する（ROM2.bin非依存の固定ベクタ）。
    #[test]
    fn decodes_known_record_fields() {
        // 0x0A60 の音色の先頭部分（row0..row8の一部）を再現したレコード。
        let mut rec = [0u8; VOICE_SIZE];
        // row0 detune
        rec[0..4].copy_from_slice(&[0x20, 0x20, 0x20, 0x20]);
        // row1 MUL (bit7セット)
        rec[4..8].copy_from_slice(&[0x81, 0x83, 0x81, 0x80]);
        // row2 TL
        rec[8..12].copy_from_slice(&[0x1C, 0x14, 0x16, 0x15]);
        // row4 D2R(暫定)
        rec[16..20].copy_from_slice(&[0x04, 0x07, 0x04, 0x00]);
        // row5 KSR+AR
        rec[20..24].copy_from_slice(&[0x1F, 0x9F, 0x9F, 0x9F]);
        // row6 AM+wave+D1R
        rec[24..28].copy_from_slice(&[0x1F, 0x9F, 0x9F, 0x9F]);
        // row8 D1L+RR(暫定)
        rec[32..36].copy_from_slice(&[0x59, 0x09, 0x09, 0x09]);
        // +0x3B = FB+ALG。0x3C = FB=7, ALG=4
        rec[0x3B] = 0x3C;

        let v = decode_voice(&rec);
        // op0
        assert_eq!(v.operators[0].detune, 0x20);
        assert_eq!(v.operators[0].mul, 1); // 0x81 & 0x0F
        assert_eq!(v.operators[0].tl, 0x1C);
        assert_eq!(v.operators[0].ar, 0x1F); // 0x1F & 0x1F
        assert_eq!(v.operators[0].ksr, 0); // 0x1F >> 6
        assert_eq!(v.operators[0].d1r, 0x1F); // 0x1F & 0x1F
        assert!(!v.operators[0].am_enable); // 0x1F bit7=0
        assert_eq!(v.operators[0].d1l, 5); // 0x59 >> 4
        assert_eq!(v.operators[0].rr, 9); // 0x59 & 0x0F
                                          // op1 (AR行=0x9F: KSR=2, AR=31; D1R行=0x9F: AM-EN=1, D1R=31)
        assert_eq!(v.operators[1].ksr, 2); // 0x9F >> 6
        assert_eq!(v.operators[1].ar, 0x1F);
        assert!(v.operators[1].am_enable); // 0x9F bit7=1
        assert_eq!(v.operators[1].mul, 3); // 0x83 & 0x0F
        // channel: 0x3C = FB=7, ALG=4
        assert_eq!(v.algorithm, 4);
        assert_eq!(v.feedback, 7);
    }

    #[test]
    fn rejects_too_small_rom() {
        let small = [0u8; 0x100];
        assert!(parse_rom2_voices(&small).is_err());
    }

    #[test]
    fn full_table_yields_68_voices() {
        let rom = [0u8; 0x8000]; // 32KB全ゼロでも件数は68
        let voices = parse_rom2_voices(&rom).expect("parse");
        assert_eq!(voices.len(), VOICE_COUNT);
        assert_eq!(voices[0].name, "PSR70 00");
    }
}
