//! OPN（YM2608/OPNA）ボイスレジスタ → 実機非依存スケールへの写像・非EGフィールド構築。
//!
//! 由来: ym38x6/tools/mucom2x6/src/conv.rs（コミット8d9c6bb時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! mucom2x6側の修正は自動では反映されない（`git diff 8d9c6bb -- ym38x6/tools/mucom2x6/src/conv.rs`
//! で追従漏れを確認できる）。
//!
//! EGレート系の関数名（旧`_to_x6`）は`_to_eg_rate`/`_to_eg_level`へ改名した。実体は
//! `sound_core::eg::{ar_to_delta, decay_to_delta, rr_to_delta}`が解釈するレートスケール
//! （0〜255）であり、ym38x6固有ではない（旧名の"x6"は歴史的経緯にすぎない）。

use op505_core::Op505OperatorParams;
use sound_core::TimeEgParams;

// ---------------------------------------------------------------------------
// OPN中間表現（レジスタ値のまま保持、アルゴリズム順: op0=OP1, op1=OP2, op2=OP3, op3=OP4）
// ---------------------------------------------------------------------------

/// OPNオペレーター1個分のレジスタ値（アルゴリズム順のop番号で保持）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpnOperator {
    /// Total Level（7bit, 0〜127。0=最大音量、127=最小音量）。
    pub tl: u8,
    /// Attack Rate（5bit, 0〜31）。
    pub ar: u8,
    /// Decay Rate / D1R（5bit, 0〜31）。
    pub d1r: u8,
    /// Sustain Rate / D2R（5bit, 0〜31）。
    pub d2r: u8,
    /// Sustain Level / D1L（4bit, 0〜15）。
    pub d1l: u8,
    /// Release Rate（4bit, 0〜15）。
    pub rr: u8,
    /// Multiple（4bit, 0〜15）。
    pub mul: u8,
    /// Detune1（3bit, 0〜7。OPN符号付きエンコード: 0/4=±0, 1-3=正方向, 5-7=負方向）。
    pub dt1: u8,
    /// Key Scale Rate（2bit, 0〜3）。
    pub ks: u8,
    /// AMS-EN（このオペレーターをAM変調対象にするか）。
    pub am_enable: bool,
}

/// OPN 1ボイス（4オペレーター + チャンネル設定）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpnVoice {
    /// アルゴリズム順: [op0=OP1(FB), op1=OP2, op2=OP3, op3=OP4(carrier for ALG0)]
    pub operators: [OpnOperator; 4],
    /// Algorithm（3bit, 0〜7）。
    pub algorithm: u8,
    /// Feedback（3bit, 0〜7）。
    pub feedback: u8,
}

/// 名前付きボイス（元のスロット番号を保持）。
#[derive(Clone, Debug, PartialEq)]
pub struct NamedVoice {
    /// MUCOM88 のスロット番号（0〜255）。プログラム番号への対応付けに使う。
    pub slot: u16,
    pub name: String,
    pub voice: OpnVoice,
}

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// OPN KS(0-3、rate scaling)→ ksr(0-255、実行時の音域依存レート倍率)。
#[inline]
pub fn ks_to_ksr(v: u8) -> u8 {
    const TABLE: [u8; 4] = [0, 64, 128, 255];
    TABLE[v.min(3) as usize]
}

/// Total Level: OPN（0=最大音量, 127=最小音量）→ TL（0=最小, 254=最大）。
#[inline]
pub fn tl_reg_to_level(tl: u8) -> u8 {
    (127 - tl.min(127)) * 2
}

/// Sustain Level: OPN（減衰量 0=減衰なし/フルレベル, 15=ほぼ無音）→ EGレベル(0-255)。
#[inline]
pub fn sl_to_eg_level(sl: u8) -> u8 {
    let reg = sl.min(15);
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    (255.0 * (1.0 + db / 93.0)).round() as u8
}

/// OPN DT1（3bit, 0〜7）→ dt1（8bit, 中心128=±0, ±50セント）。
#[inline]
pub fn dt1_reg_to_detune(dt1: u8) -> u8 {
    const TABLE: [u8; 8] = [128, 131, 134, 136, 128, 125, 122, 120];
    TABLE[(dt1 & 7) as usize]
}

/// OPN 5bit レート（AR/D1R/D2R, 0〜31）+ KS（0〜3）→ EGレート(0〜255)。
pub fn rate_to_eg_rate(rate_5bit: u8, ks: u8) -> u8 {
    if rate_5bit == 0 {
        return 0;
    }
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (2 * rate_5bit as u16 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

/// AR専用のオンセット補正バイアス（EGレート加算値）。
const ATTACK_ONSET_BIAS: u16 = 30;

/// OPN 5bit AR（0〜31）+ KS（0〜3）→ EGレート(0〜255)。
pub fn ar_to_eg_rate(ar_5bit: u8, ks: u8) -> u8 {
    if ar_5bit == 0 {
        return 0;
    }
    (rate_to_eg_rate(ar_5bit, ks) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPN 4bit RR（0〜15）+ KS（0〜3）→ EGレート(0〜255)。
pub fn rr_to_eg_rate(rr_4bit: u8, ks: u8) -> u8 {
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (4 * rr_4bit as u16 + 2 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

/// 3bit（0〜7）→ 8bit（0〜255）: ×36。Feedback に使用。
#[inline]
pub fn scale_3bit(v: u8) -> u8 {
    v.min(7) * 36
}

// ---------------------------------------------------------------------------
// オペレーター・チャンネルの非EGフィールド構築
// ---------------------------------------------------------------------------

/// アルゴリズム別のキャリア（出力に直接合算される）オペレーターindex。
pub const CARRIERS: [&[usize]; 8] = [
    &[3],          // 0
    &[3],          // 1
    &[3],          // 2
    &[3],          // 3
    &[1, 3],       // 4
    &[1, 2, 3],    // 5
    &[1, 2, 3],    // 6
    &[0, 1, 2, 3], // 7
];

/// OPNオペレーター1個分の非EGフィールドを`Op505OperatorParams`へ直接構築する
/// （`eg`フィールドはこの関数では埋めない。呼び出し側が`direct_eg`の結果で上書きする）。
pub fn convert_op(op: &OpnOperator) -> Op505OperatorParams {
    Op505OperatorParams {
        tl: tl_reg_to_level(op.tl),
        eg: TimeEgParams::default(),
        mul: op.mul.min(15),
        dt1: dt1_reg_to_detune(op.dt1),
        ksr: ks_to_ksr(op.ks),
        am_enable: op.am_enable,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: 128,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    }
}

/// チャンネル単位の非EGフィールド一式。
pub struct ChannelFields {
    pub algorithm: u8,
    pub feedback: u8,
}

pub fn convert_channel(voice: &OpnVoice) -> ChannelFields {
    ChannelFields {
        algorithm: voice.algorithm.min(7),
        feedback: scale_3bit(voice.feedback.min(7)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dt1_zero_and_four_both_map_to_center() {
        // OPN DT1の冗長エンコード（0と4がどちらも±0セント）。
        // vgm2op505のdedupキー設計はこの衝突を前提に案B（ソース値そのままの比較）を採用した。
        assert_eq!(dt1_reg_to_detune(0), 128);
        assert_eq!(dt1_reg_to_detune(4), 128);
    }

    #[test]
    fn rate_zero_freezes_and_max_saturates() {
        assert_eq!(rate_to_eg_rate(0, 0), 0);
        assert_eq!(ar_to_eg_rate(0, 0), 0);
        assert_eq!(rate_to_eg_rate(31, 0), 255);
        assert_eq!(rr_to_eg_rate(15, 0), 255);
    }

    #[test]
    fn tl_polarity_inverts() {
        assert_eq!(tl_reg_to_level(0), 254);
        assert_eq!(tl_reg_to_level(127), 0);
    }
}
