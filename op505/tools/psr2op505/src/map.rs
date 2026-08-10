//! OPQ（YM3806）ボイスレジスタ → 実機非依存スケールへの写像・非EGフィールド構築。
//!
//! 由来: ym38x6/tools/psr2x6/src/conv.rs（コミット2fcdd7a時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! psr2x6側の修正は自動では反映されない（`git diff 2fcdd7a -- ym38x6/tools/psr2x6/src/conv.rs`
//! で追従漏れを確認できる）。
//!
//! EGレート系の関数名（旧`_to_x6`）は`_to_eg_rate`/`_to_eg_level`へ改名した。実体は
//! `sound_core::eg::{ar_to_delta, decay_to_delta, rr_to_delta}`が解釈するレートスケール
//! （0〜255）であり、ym38x6固有ではない（旧名の"x6"は歴史的経緯にすぎない）。

use op505_core::Op505OperatorParams;
use sound_core::TimeEgParams;

// ---------------------------------------------------------------------------
// OPQ中間表現（各レジスタを実機のビット幅のまま保持する）
// ---------------------------------------------------------------------------

/// OPQオペレーター1個分のレジスタ値（実機のビット幅のまま）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpqOperator {
    /// Total Level（7bit, 0〜127。0=最大音量/0dB、127=最小音量/-95.25dB の減衰量）。
    pub tl: u8,
    /// Attack Rate（5bit, 0〜31）。
    pub ar: u8,
    /// Decay1 Rate（5bit, 0〜31）。
    pub d1r: u8,
    /// Decay2 Rate（5bit, 0〜31）。
    pub d2r: u8,
    /// Decay1 Level / Sustain Level（4bit, 0〜15）。
    pub d1l: u8,
    /// Release Rate（4bit, 0〜15）。
    pub rr: u8,
    /// Multiple（4bit, 0〜15）。
    pub mul: u8,
    /// Detune（6bit, 0〜63。中心32=デチューンなし）。
    pub detune: u8,
    /// Key Scale Rate（2bit, 0〜3）。
    pub ksr: u8,
    /// AMS-EN（このオペレーターをAM変調対象にするか）。
    pub am_enable: bool,
}

/// OPQ 1ボイス（4オペレーター + チャンネル設定）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpqVoice {
    pub operators: [OpqOperator; 4],
    /// Algorithm / Connection（3bit, 0〜7）。
    pub algorithm: u8,
    /// Feedback（3bit, 0〜7）。
    pub feedback: u8,
}

/// 名前付きボイス（def_seqs.h由来の音色名と本体）。
#[derive(Clone, Debug, PartialEq)]
pub struct NamedVoice {
    pub name: String,
    pub voice: OpqVoice,
}

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// OPQ 5bit レート（AR/D1R/D2R, 0〜31）+ KSR（0〜3）→ EGレート(0〜255)。
#[inline]
pub fn rate_to_eg_rate(rate_5bit: u8, ksr: u8) -> u8 {
    if rate_5bit == 0 {
        return 0;
    }
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ksr.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (2 * rate_5bit as u16 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

/// AR専用のオンセット補正バイアス（EGレート加算値）。
const ATTACK_ONSET_BIAS: u16 = 30;

/// OPQ 5bit AR（0〜31）+ KSR（0〜3）→ EGレート(0〜255)。
#[inline]
pub fn ar_to_eg_rate(ar_5bit: u8, ksr: u8) -> u8 {
    if ar_5bit == 0 {
        return 0;
    }
    (rate_to_eg_rate(ar_5bit, ksr) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPQ 4bit RR（0〜15）+ KSR（0〜3）→ EGレート(0〜255)。
#[inline]
pub fn rr_to_eg_rate(rr_4bit: u8, ksr: u8) -> u8 {
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ksr.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (4 * rr_4bit as u16 + 2 + ksr_add).min(62);
    (1 + (eg_rate.saturating_sub(2)) * 254 / 60).min(255) as u8
}

/// 3bit（0〜7）→ 8bit（0〜255）: ×36。Feedback に使用。
#[inline]
pub fn scale_3bit(v: u8) -> u8 {
    (v.min(7)) * 36
}

/// OPQ KSR(0-3、rate scaling)→ ksr(0-255、実行時の音域依存レート倍率)。
#[inline]
pub fn ks_to_ksr(v: u8) -> u8 {
    const TABLE: [u8; 4] = [0, 64, 128, 255];
    TABLE[v.min(3) as usize]
}

/// Detune 6bit（0〜63, 中心32）→ dt1 8bit（中心128）: ×4。
#[inline]
pub fn dt1_reg_to_detune(v: u8) -> u8 {
    (v.min(63)) * 4
}

/// Total Level: OPQ（減衰量 0=最大音量, 127=最小音量）→ TL（0=最小, 254=最大）。
#[inline]
pub fn tl_reg_to_level(tl: u8) -> u8 {
    (127 - tl.min(127)) * 2
}

/// Decay1 Level / Sustain Level: OPQ（減衰量 0=減衰なし, 15=ほぼ無音）→ EGレベル(0-255)。
#[inline]
pub fn sl_to_eg_level(sl: u8) -> u8 {
    let reg = sl.min(15);
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    (255.0 * (1.0 + db / 93.0)).round() as u8
}

// ---------------------------------------------------------------------------
// 味付けオプション
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

/// モジュレーター TL 天井のオプトイン値。
pub const DEFAULT_MOD_TL_CAP: u8 = 180;

/// 変換の味付けオプション。
#[derive(Clone, Copy, Debug)]
pub struct PsrConvOptions {
    /// モジュレーター TL 天井。既定は`None`（天井なし＝実機TLをそのまま反映）。
    pub mod_tl_cap: Option<u8>,
    /// キャリアのサステイン延長（0.0=実機準拠 .. 1.0=最大延長）。
    pub carrier_sustain: f32,
    /// ローパスフィルターのカットオフ上書き（`None`=全開255=20kHz）。
    pub filter_cutoff: Option<u8>,
}

impl Default for PsrConvOptions {
    fn default() -> Self {
        Self { mod_tl_cap: None, carrier_sustain: 0.0, filter_cutoff: None }
    }
}

/// キャリアのサステイン延長をEGフィールド（d1l/d1r/d2r）へ適用する。
pub fn apply_carrier_sustain(d1l: &mut u8, d1r: &mut u8, d2r: &mut u8, sustain: f32) {
    let k = sustain.clamp(0.0, 1.0);
    if k <= 0.0 {
        return;
    }
    let d1l_f = *d1l as f32;
    *d1l = (d1l_f + (255.0 - d1l_f) * 0.7 * k).round().clamp(0.0, 255.0) as u8;
    *d1r = (*d1r as f32 * (1.0 - 0.60 * k)).round().clamp(0.0, 255.0) as u8;
    *d2r = (*d2r as f32 * (1.0 - 0.85 * k)).round().clamp(0.0, 255.0) as u8;
}

// ---------------------------------------------------------------------------
// オペレーター・チャンネルの非EGフィールド構築
// ---------------------------------------------------------------------------

/// OPQオペレーター1個分の非EGフィールドを`Op505OperatorParams`へ直接構築する
/// （`eg`フィールドはこの関数では埋めない。呼び出し側が`direct_eg`の結果で上書きする）。
/// OPQに無いパラメーターは規約値で埋める（`velocity_sensitivity=0`：OPQにベロシティ感度
/// レジスタ無し、`waveform=0`：OPQはサイン固定）。
pub fn convert_op(op: &OpqOperator, is_carrier: bool, mod_tl_cap: Option<u8>) -> Op505OperatorParams {
    let mut tl = tl_reg_to_level(op.tl);
    if !is_carrier {
        if let Some(cap) = mod_tl_cap {
            tl = (tl as f32 * cap as f32 / 254.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Op505OperatorParams {
        tl,
        eg: TimeEgParams::default(),
        mul: op.mul.min(15),
        dt1: dt1_reg_to_detune(op.detune),
        ksr: ks_to_ksr(op.ksr),
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
    pub filter_cutoff: u8,
}

pub fn convert_channel(voice: &OpqVoice, alg: u8, opts: PsrConvOptions) -> ChannelFields {
    ChannelFields {
        algorithm: alg,
        feedback: scale_3bit(voice.feedback),
        filter_cutoff: opts.filter_cutoff.unwrap_or(255),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_reaches_upper_bound() {
        assert_eq!(scale_3bit(7), 252);
        assert_eq!(ks_to_ksr(3), 255);
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

    #[test]
    fn sl_mid_values_follow_db_linear_curve() {
        assert_eq!(sl_to_eg_level(4), 222);
        assert_eq!(sl_to_eg_level(8), 189);
        assert_eq!(sl_to_eg_level(14), 140);
    }

    #[test]
    fn mod_tl_cap_compresses_modulator_only() {
        let op = OpqOperator { tl: 0, ar: 31, d1r: 10, d2r: 4, d1l: 2, rr: 7, mul: 1, detune: 32, ksr: 1, am_enable: false };
        let uncapped = convert_op(&op, false, None);
        let capped = convert_op(&op, false, Some(DEFAULT_MOD_TL_CAP));
        assert!(capped.tl <= uncapped.tl);
        let carrier_capped = convert_op(&op, true, Some(DEFAULT_MOD_TL_CAP));
        assert_eq!(carrier_capped.tl, uncapped.tl, "キャリアは天井の影響を受けない");
    }
}
