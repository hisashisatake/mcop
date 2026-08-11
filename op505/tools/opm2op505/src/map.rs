//! OPM（YM2151）レジスタ値 → 実機非依存スケールへの写像・非EGフィールド構築。
//!
//! 由来: ym38x6/tools/opm2x6/src/conv.rs（コミット2fcdd7a時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! opm2x6側の修正は自動では反映されない（`git diff 2fcdd7a -- ym38x6/tools/opm2x6/src/conv.rs`
//! で追従漏れを確認できる）。
//!
//! EGレート系の関数名（旧`_to_x6`）は`_to_eg_rate`/`_to_eg_level`へ改名した。実体は
//! `sound_core::eg::{ar_to_delta, decay_to_delta, rr_to_delta}`が解釈するレートスケール
//! （0〜255）であり、ym38x6固有ではない（旧名の"x6"は歴史的経緯にすぎない）。

use op505_core::Op505OperatorParams;
use sound_core::TimeEgParams;

use crate::parse::{OpmOpReg, OpmVoice, OperatorOrder};

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// TL（OPM 7-bit、0=最大音量、127=最小音量）→ TL（0=最小、254=最大）。
#[inline]
fn tl_reg_to_level(tl: u8) -> u8 {
    (127 - tl.min(127)) * 2
}

/// D1L/SL（OPM 4-bit、0-15）→ EGレベル（0=ほぼ無音、255=フルレベル）。
pub fn sl_to_eg_level(reg: u8) -> u8 {
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    (255.0 * (1.0 + db / 93.0)).round() as u8
}

/// DT1（OPM/OPN共通 3-bit、0-7）→ dt1（中心128、±50セント）。
fn dt1_reg_to_detune(dt1: u8) -> u8 {
    const TABLE: [u8; 8] = [128, 131, 134, 136, 128, 125, 122, 120];
    TABLE[(dt1 & 7) as usize]
}

/// DT2（OPM固有 2-bit、0-3）→ op_fine_tune（中心128、±1200セント）。
fn dt2_to_op_fine_tune(dt2: u8) -> u8 {
    const TABLE: [u8; 4] = [128, 192, 211, 229];
    TABLE[(dt2 & 3) as usize]
}

/// OPM 5-bit レート（AR/D1R/D2R、0-31）+ KS（0-3）→ EGレート(0-255)。
pub fn rate_to_eg_rate(rate: u8, ks: u8) -> u8 {
    if rate == 0 {
        return 0;
    }
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (2 * rate as u16 + ksr_add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// dBリニアアタックの可聴オンセット遅れをOPM実機に寄せる補正バイアス。
const ATTACK_ONSET_BIAS: u16 = 30;

/// OPM AR（5-bit）+ KS → EGレート（オンセット補正付き）。
pub fn ar_to_eg_rate(ar: u8, ks: u8) -> u8 {
    if ar == 0 {
        return 0;
    }
    (rate_to_eg_rate(ar, ks) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPM RR（4-bit、0-15）+ KS → EGレート(0-255)。
pub fn rr_to_eg_rate(rr: u8, ks: u8) -> u8 {
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (4 * rr as u16 + 2 + ksr_add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// OPM KS(0-3、rate scaling)→ ksr(0-255、実行時の音域依存レート倍率)。
fn ks_to_ksr(ks: u8) -> u8 {
    const TABLE: [u8; 4] = [0, 64, 128, 255];
    TABLE[ks.min(3) as usize]
}

/// OPM AMS sensitivity（CH 2-bit、0-3）→ ams（0=off、1/128/255）。
fn ams_reg_to_depth(reg: u8) -> u8 {
    if reg == 0 {
        return 0;
    }
    (1u16 + 127 * (reg.min(3) as u16 - 1)) as u8
}

/// OPM PMS sensitivity（CH 3-bit、0-7）→ pms（0=off、等間隔1-255）。
fn pms_reg_to_depth(reg: u8) -> u8 {
    if reg == 0 {
        return 0;
    }
    (1.0_f32 + 254.0 * (reg.min(7) - 1) as f32 / 6.0).round() as u8
}

/// AMD/PMD（7-bit、0-127）→ depth（0-255）。直線スケール。
fn lfo_depth_reg_to_depth(reg: u8) -> u8 {
    (reg as f32 * 255.0 / 127.0).round() as u8
}

// ---------------------------------------------------------------------------
// 構造体変換
// ---------------------------------------------------------------------------

/// OPM SLOT（CH行の slot フィールド）から各OPのミュートフラグを返す。
/// 戻り値: [M1_muted, C1_muted, M2_muted, C2_muted]
fn slot_muted(slot: u8) -> [bool; 4] {
    [
        (slot & (1 << 3)) == 0, // M1
        (slot & (1 << 4)) == 0, // C1
        (slot & (1 << 5)) == 0, // M2
        (slot & (1 << 6)) == 0, // C2
    ]
}

/// SLOT配列 [M1,C1,M2,C2] を`op_order`に従って `operators[0..3]` の物理OP順へ並べ替える。
pub fn ordered_op_regs(voice: &OpmVoice, op_order: OperatorOrder) -> [&OpmOpReg; 4] {
    let phys: [&OpmOpReg; 4] = [&voice.m1, &voice.c1, &voice.m2, &voice.c2];
    let idx: [usize; 4] = match op_order {
        OperatorOrder::Direct => [0, 1, 2, 3],
        OperatorOrder::Register => [0, 2, 1, 3],
    };
    std::array::from_fn(|i| phys[idx[i]])
}

/// `ordered_op_regs`と同じ並びで、対応するミュートフラグ配列を返す。
fn ordered_muted(voice: &OpmVoice, op_order: OperatorOrder) -> [bool; 4] {
    let muted = slot_muted(voice.slot);
    let idx: [usize; 4] = match op_order {
        OperatorOrder::Direct => [0, 1, 2, 3],
        OperatorOrder::Register => [0, 2, 1, 3],
    };
    std::array::from_fn(|i| muted[idx[i]])
}

/// OPMオペレーター1個分の非EGフィールドを`Op505OperatorParams`へ直接構築する
/// （`eg`フィールドはこの関数では埋めない。呼び出し側が`direct_eg`の結果で上書きする）。
/// `muted`のときはTLを0に固定（slot_maskで無効化されたOP）。
pub fn convert_op(op: &OpmOpReg, muted: bool) -> Op505OperatorParams {
    Op505OperatorParams {
        tl: if muted { 0 } else { tl_reg_to_level(op.tl) },
        eg: TimeEgParams::default(),
        mul: op.mul.min(15),
        dt1: dt1_reg_to_detune(op.dt1),
        ksr: ks_to_ksr(op.ks),
        am_enable: op.ams_en,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: dt2_to_op_fine_tune(op.dt2),
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    }
}

/// `op_order`に従って並べたオペレーター4個の非EGフィールド一式（`eg`未設定）と、
/// 対応するミュートフラグを返す。呼び出し側はこの順序で[crate::conv::direct_eg]を
/// 呼び`eg`を埋める。
pub fn convert_ops(voice: &OpmVoice, op_order: OperatorOrder) -> [Op505OperatorParams; 4] {
    let ops = ordered_op_regs(voice, op_order);
    let muted = ordered_muted(voice, op_order);
    std::array::from_fn(|i| convert_op(ops[i], muted[i]))
}

/// チャンネル単位の非EGフィールド一式。
pub struct ChannelFields {
    pub algorithm: u8,
    pub feedback: u8,
    pub chip_lfo_freq: u8,
    pub chip_lfo_pmd: u8,
    pub chip_lfo_amd: u8,
    pub pms: u8,
    pub ams: u8,
}

pub fn convert_channel(voice: &OpmVoice) -> ChannelFields {
    ChannelFields {
        algorithm: voice.con.min(7),
        feedback: voice.fl.min(7) * 36,
        // LFRQは直接コピー（OPMのLFRQレートテーブルとop505は完全一致しないが低優先）。
        chip_lfo_freq: voice.lfrq,
        chip_lfo_pmd: lfo_depth_reg_to_depth(voice.pmd),
        chip_lfo_amd: lfo_depth_reg_to_depth(voice.amd),
        pms: pms_reg_to_depth(voice.pms),
        ams: ams_reg_to_depth(voice.ams),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tl_polarity_inverts() {
        assert_eq!(tl_reg_to_level(0), 254);
        assert_eq!(tl_reg_to_level(127), 0);
    }

    #[test]
    fn dt1_center_and_direction() {
        assert_eq!(dt1_reg_to_detune(0), 128);
        assert_eq!(dt1_reg_to_detune(4), 128);
        assert!(dt1_reg_to_detune(1) > 128);
        assert!(dt1_reg_to_detune(5) < 128);
    }

    #[test]
    fn muted_op_has_tl_zero() {
        let op = OpmOpReg { tl: 60, ar: 31, rr: 7, mul: 1, ..OpmOpReg::default() };
        let muted = convert_op(&op, true);
        assert_eq!(muted.tl, 0, "muted op must have TL=0");
    }

    #[test]
    fn active_op_with_dt2_sets_op_fine_tune() {
        let op = OpmOpReg { tl: 0, ar: 31, rr: 7, mul: 1, dt2: 1, ..OpmOpReg::default() };
        let p = convert_op(&op, false);
        assert_eq!(p.op_fine_tune, 192, "DT2=1 → op_fine_tune=192 (+600¢)");
    }

    #[test]
    fn register_order_swaps_c1_m2() {
        let voice = OpmVoice {
            m1: OpmOpReg { tl: 10, ar: 31, rr: 7, mul: 1, ..OpmOpReg::default() },
            c1: OpmOpReg { tl: 20, ar: 31, rr: 7, mul: 2, ..OpmOpReg::default() },
            m2: OpmOpReg { tl: 30, ar: 31, rr: 7, mul: 3, ..OpmOpReg::default() },
            c2: OpmOpReg { tl: 40, ar: 31, rr: 7, mul: 4, ..OpmOpReg::default() },
            slot: 120,
            ..OpmVoice::default()
        };

        let direct = convert_ops(&voice, OperatorOrder::Direct);
        let reg = convert_ops(&voice, OperatorOrder::Register);

        assert_eq!(direct[0].mul, 1);
        assert_eq!(direct[1].mul, 2);
        assert_eq!(direct[2].mul, 3);
        assert_eq!(direct[3].mul, 4);

        assert_eq!(reg[0].mul, 1);
        assert_eq!(reg[1].mul, 3);
        assert_eq!(reg[2].mul, 2);
        assert_eq!(reg[3].mul, 4);
    }
}
