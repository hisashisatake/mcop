//! MUCOM88 バイナリ音色バンク（`voice.dat`）パーサー。
//!
//! フォーマット（`muplug_fmeditor/editor/voice.h` の `VOICEFORMAT` 構造体準拠）:
//! - 1ボイス = **32バイト**（256スロット × 32 = 8192バイト固定）
//! - byte 0: `hed`（スロット番号 or 0xFF=空スロット）
//! - bytes 1-4: DT/ML × 4op（格納順: op1, op3, op2, op4 = OPNレジスターアドレス順）
//! - bytes 5-8: TL × 4op（同順）
//! - bytes 9-12: KS/AR × 4op
//! - bytes 13-16: AMON/DR × 4op
//! - bytes 17-20: SR × 4op
//! - bytes 21-24: SL/RR × 4op（byte: rr[3:0] | sl[7:4]）
//! - byte 25: AL[2:0] | FB[5:3] | pad[7:6]
//! - bytes 26-31: name[6]（Shift-JIS、NULL終端またはスペース埋め）
//!
//! **オペレーター並び替え**: 格納順 (op1, op3, op2, op4) → アルゴリズム順 (op1, op2, op3, op4)
//! へ変換してから `OpnVoice` に格納する。
//! OPN ALG0 の直列チェーンは op1→op2→op3→op4 であり、これが 38x6 の op0→op1→op2→op3 に対応。

use crate::conv::{NamedVoice, OpnOperator, OpnVoice};

/// 1スロットのバイト数。
pub const SLOT_SIZE: usize = 32;
/// スロット総数。
pub const SLOT_COUNT: usize = 256;
/// ファイルサイズ（固定）。
pub const FILE_SIZE: usize = SLOT_SIZE * SLOT_COUNT;

/// `voice.dat` バイナリから有効なボイスを抽出する。
///
/// 除外するスロット:
/// - `hed == 0xFF`（空マーカー）
/// - 全オペレーターの `AR == 0`（ゼロ初期化スロット、発音しない）
pub fn parse_voice_dat(dat: &[u8]) -> Result<Vec<NamedVoice>, String> {
    if dat.len() < FILE_SIZE {
        return Err(format!(
            "voice.dat が小さすぎます: {} バイト（{}バイト必要）",
            dat.len(),
            FILE_SIZE
        ));
    }
    let mut voices = Vec::new();
    for slot in 0..SLOT_COUNT {
        let base = slot * SLOT_SIZE;
        let rec = &dat[base..base + SLOT_SIZE];
        // 格納順インデックス (op1,op3,op2,op4) = (0,1,2,3)
        // → アルゴリズム順 (op1,op2,op3,op4) = struct indices (0,2,1,3)
        let reg_ops = decode_operators(rec);
        let ops = [reg_ops[0], reg_ops[2], reg_ops[1], reg_ops[3]];
        // 全オペレーターAR=0はゼロ初期化スロット → スキップ
        if ops.iter().all(|op| op.ar == 0) {
            continue;
        }
        let al = rec[25] & 0x07;
        let fb = (rec[25] >> 3) & 0x07;
        let name = decode_name(&rec[26..32], slot);
        voices.push(NamedVoice {
            slot: slot as u16,
            name,
            voice: OpnVoice { operators: ops, algorithm: al, feedback: fb },
        });
    }
    Ok(voices)
}

/// 格納順 4op のパラメーターをデコード（op1, op3, op2, op4 の格納順のまま返す）。
fn decode_operators(rec: &[u8]) -> [OpnOperator; 4] {
    let mut ops = [OpnOperator::default(); 4];
    for i in 0..4 {
        let dt_ml = rec[1 + i];
        let tl    = rec[5 + i];
        let ks_ar = rec[9 + i];
        let am_dr = rec[13 + i];
        let sr    = rec[17 + i];
        let sl_rr = rec[21 + i];
        ops[i] = OpnOperator {
            mul:       dt_ml & 0x0F,
            dt1:       (dt_ml >> 4) & 0x07,
            tl:        tl & 0x7F,
            ar:        ks_ar & 0x1F,
            ks:        (ks_ar >> 6) & 0x03,
            d1r:       am_dr & 0x1F,
            am_enable: (am_dr >> 7) & 1 == 1,
            d2r:       sr & 0x1F,
            rr:        sl_rr & 0x0F,
            d1l:       (sl_rr >> 4) & 0x0F,
        };
    }
    ops
}

/// name[6] バイト列 → 文字列。
/// - ASCII printable（0x20〜0x7E）はそのまま
/// - Shift-JIS 半角カナ（0xA1〜0xDF）は対応する Unicode ハーフ幅カナ（U+FF61〜U+FF9F）に変換
/// - 全て非printableなら "Voice NNN" にフォールバック
fn decode_name(bytes: &[u8], slot: usize) -> String {
    let mut result = String::new();
    for &b in bytes.iter().take_while(|&&b| b != 0) {
        if b.is_ascii_graphic() || b == b' ' {
            result.push(b as char);
        } else if (0xA1..=0xDF).contains(&b) {
            // 半角カナ: SJIS 0xA1〜0xDF → U+FF61〜U+FF9F
            // 変換式: Unicode = 0xFEC0 + byte
            let ch = char::from_u32(0xFEC0u32 + b as u32).unwrap_or('?');
            result.push(ch);
        }
        // 上記以外（全角の1バイト目など）は除去
    }
    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        format!("Voice {:03}", slot)
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_slot(hed: u8, ar_all: u8, al: u8, fb: u8, name: &[u8]) -> [u8; SLOT_SIZE] {
        let mut slot = [0u8; SLOT_SIZE];
        slot[0] = hed;
        // AR は bytes 9-12。全 op に同じ AR をセット（bit0-4）
        for i in 0..4 {
            slot[9 + i] = ar_all & 0x1F;
        }
        slot[25] = al | (fb << 3);
        let n = name.len().min(6);
        slot[26..26 + n].copy_from_slice(&name[..n]);
        slot
    }

    #[test]
    fn rejects_too_small_dat() {
        let small = [0u8; 100];
        assert!(parse_voice_dat(&small).is_err());
    }

    #[test]
    fn hed_ff_is_valid_voice() {
        // hed=0xFF は空スロットではなく有効な音色データに使われる（back1 等）
        // → hed=0xFF のみで除外してはならない
        let mut dat = [0u8; FILE_SIZE];
        let s = make_slot(0xFF, 31, 4, 5, b"back1 ");
        dat[0..SLOT_SIZE].copy_from_slice(&s);
        let voices = parse_voice_dat(&dat).unwrap();
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].name, "back1");
    }

    #[test]
    fn skips_zero_ar_slots() {
        // ゼロ初期化 → 全スロット AR=0 → 全スキップ
        let dat = [0u8; FILE_SIZE];
        assert_eq!(parse_voice_dat(&dat).unwrap().len(), 0);
    }

    #[test]
    fn operator_reorder_slot_to_algo() {
        // reg 格納順: (op1,op3,op2,op4) の TL をそれぞれ 10,30,20,40 に設定
        let mut dat = [0xFFu8; FILE_SIZE];
        let mut s = make_slot(0, 31, 0, 0, b"test");
        // TL bytes 5-8 = TL(op1)=10, TL(op3)=30, TL(op2)=20, TL(op4)=40
        s[5] = 10; s[6] = 30; s[7] = 20; s[8] = 40;
        dat[0..SLOT_SIZE].copy_from_slice(&s);

        let voices = parse_voice_dat(&dat).unwrap();
        let ops = &voices[0].voice.operators;
        // アルゴリズム順に並び替え後: [op1=10, op2=20, op3=30, op4=40]
        assert_eq!(ops[0].tl, 10); // op1（reg位置0 → algo位置0）
        assert_eq!(ops[1].tl, 20); // op2（reg位置2 → algo位置1）
        assert_eq!(ops[2].tl, 30); // op3（reg位置1 → algo位置2）
        assert_eq!(ops[3].tl, 40); // op4（reg位置3 → algo位置3）
    }

    #[test]
    fn name_ascii_trimmed() {
        let mut dat = [0xFFu8; FILE_SIZE];
        let s = make_slot(0, 31, 0, 0, b"dgt1  ");
        dat[0..SLOT_SIZE].copy_from_slice(&s);
        let voices = parse_voice_dat(&dat).unwrap();
        assert_eq!(voices[0].name, "dgt1");
        assert_eq!(voices[0].slot, 0);
    }

    #[test]
    fn name_empty_falls_back_to_slot_number() {
        let mut dat = [0xFFu8; FILE_SIZE];
        let s = make_slot(0, 31, 0, 0, b"");
        dat[0..SLOT_SIZE].copy_from_slice(&s);
        let voices = parse_voice_dat(&dat).unwrap();
        assert_eq!(voices[0].name, "Voice 000");
        assert_eq!(voices[0].slot, 0);
    }

    #[test]
    fn name_halfwidth_katakana() {
        let mut dat = [0xFFu8; FILE_SIZE];
        // "ﾄﾞﾗﾑ" = 0xC4(ﾄ) 0xDE(ﾞ) 0xD7(ﾗ) 0xD1(ﾑ)
        let s = make_slot(0, 31, 0, 0, b"\xC4\xDE\xD7\xD1");
        dat[0..SLOT_SIZE].copy_from_slice(&s);
        let voices = parse_voice_dat(&dat).unwrap();
        assert_eq!(voices[0].name, "ﾄﾞﾗﾑ");
    }
}
