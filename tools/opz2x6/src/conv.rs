//! TX81Z（OPZ/YM2414）レジスタ値 → ym38x6 パッチへの変換ロジック。

use ym38x6_core::{ChannelParams, OperatorParams, PresetEntry, Ym38x6Patch};

use crate::parse::OpzVoice;

// ---------------------------------------------------------------------------
// キャリア判定テーブル（ym38x6-core/src/algorithm.rs から複製）
// index = algorithm (0-7), value = 38x6 operators[] のキャリアインデックス一覧
// ---------------------------------------------------------------------------

const CARRIERS: [&[usize]; 8] = [
    &[3],          // 0: O1→O2→O3→O4
    &[3],          // 1: (O1+O2)→O3→O4
    &[3],          // 2: (O1+(O2→O3))→O4
    &[3],          // 3: ((O1→O2)+O3)→O4
    &[1, 3],       // 4: (O1→O2)+(O3→O4)
    &[1, 2, 3],    // 5: (O1→O2)+(O1→O3)+(O1→O4)
    &[1, 2, 3],    // 6: (O1→O2)+O3+O4
    &[0, 1, 2, 3], // 7: O1+O2+O3+O4（全並列）
];

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// OUT (TX81Z Output Level 0-99, 99=最大) → 38x6 TL（キャリア用、0=無音, 254=最大）。
fn out_to_tl(out: u8) -> u8 {
    (out.min(99) as f32 / 99.0 * 254.0).round() as u8
}

/// モジュレーター TL 天井の既定値。
///
/// エンジンの `modulation` は位相サイクル単位（1.0=1周=2πrad）で、変調深度は
/// β[rad] = tl_to_gain(tl) × FM_MODULATION_INDEX_SCALE(4.0) × 2π。
/// TL=254 → β≈24rad（ノイズ）、TL=200 → β≈2.4rad（音楽的）。
/// キャリアは音量に直結するため天井を設けず out_to_tl を使う。
pub const DEFAULT_MOD_TL_CAP: u8 = 200;

/// OUT → 38x6 TL（モジュレーター用、上限 `cap`）。
fn out_to_tl_mod(out: u8, cap: u8) -> u8 {
    (out.min(99) as f32 / 99.0 * cap as f32).round() as u8
}

/// D1L/SL（OPM型 4-bit 0-15）→ 38x6 D1L（opm2x6 と同実装）。
fn sl_to_x6(reg: u8) -> u8 {
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    (255.0 * (1.0 + db / 93.0)).round() as u8
}

/// TX81Z DET (0-6, 3=中心) → 38x6 dt1（中心128）。
fn det_to_x6(det: u8) -> u8 {
    // DET 3=無デチューン → OPM DT1=0（+0¢）
    // DET 4,5,6 = 正方向（増大）→ DT1=1,2,3
    // DET 0,1,2 = 負方向（増大）→ DT1=7,6,5（DET 0が最強）
    const DT1_FROM_DET: [u8; 7] = [7, 6, 5, 0, 1, 2, 3];
    let dt1 = DT1_FROM_DET[det.min(6) as usize];
    const DT1_TO_X6: [u8; 8] = [128, 131, 134, 136, 128, 125, 122, 120];
    DT1_TO_X6[dt1 as usize]
}

/// TX81Z FREQ (0-63) → 38x6 (MUL 0-15, op_fine_tune 0-255)。
///
/// TX81Z の周波数コースは整数 MUL 比（0.5, 1, 2, ..., 15）を基数として
/// {1.0, √2, 2^(2/3), 2^(3/4)} の4段階で刻む（= 4種 × 16グループ = 64値）。
/// 最近傍の整数 MUL を選び、差分セントを op_fine_tune に写像する。
pub fn freq_to_mul_fine(freq: u8) -> (u8, u8) {
    const SCALE: [f32; 4] = [1.0, 1.414_213_6, 1.587_401, 1.681_793];
    const BASE: [f32; 16] = [
        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0,
        8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    let f = freq.min(63) as usize;
    let ratio = BASE[f / 4] * SCALE[f % 4];

    // 最近傍の整数MUL（対数空間距離）
    let mut best_mul = 0u8;
    let mut best_dist = f32::MAX;
    for m in 0u8..=15 {
        let mul_ratio = if m == 0 { 0.5_f32 } else { m as f32 };
        let dist = (ratio / mul_ratio).log2().abs();
        if dist < best_dist {
            best_dist = dist;
            best_mul = m;
        }
    }

    let mul_ratio = if best_mul == 0 { 0.5_f32 } else { best_mul as f32 };
    let cents = (ratio / mul_ratio).log2() * 1200.0;
    // op_fine_tune: 中心128, 1単位 = 1200/127 ≈ 9.45¢
    let oft = (128.0 + (cents * 127.0 / 1200.0)).round().clamp(0.0, 255.0) as u8;
    (best_mul, oft)
}

/// OPM型5-bitレート（AR/D1R/D2R, 0-31）+ RS → 38x6 rate（opm2x6 と同実装）。
fn opm_rate_to_x6(rate: u8, rs: u8) -> u8 {
    if rate == 0 { return 0; }
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(rs.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (2 * rate as u16 + ksr_add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

const ATTACK_ONSET_BIAS: u16 = 30;

fn ar_to_x6(ar: u8, rs: u8) -> u8 {
    if ar == 0 { return 0; }
    (opm_rate_to_x6(ar, rs) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPM型4-bitリリースレート（RR, 0-15）+ RS → 38x6 rr（opm2x6 と同実装）。
fn rr_to_x6(rr: u8, rs: u8) -> u8 {
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(rs.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (4 * rr as u16 + 2 + ksr_add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// TX81Z AMS (0-3) → 38x6 ams（opm2x6 と同実装）。
fn ams_to_x6(reg: u8) -> u8 {
    if reg == 0 { return 0; }
    (1u16 + 127 * (reg.min(3) as u16 - 1)) as u8
}

/// TX81Z PMS (0-7) → 38x6 pms（opm2x6 と同実装）。
fn pms_to_x6(reg: u8) -> u8 {
    if reg == 0 { return 0; }
    (1.0_f32 + 254.0 * (reg.min(7) - 1) as f32 / 6.0).round() as u8
}

/// AMD/PMD (0-99) → 38x6 depth（0-255 線形スケール）。
fn lfo_depth_to_x6(reg: u8) -> u8 {
    (reg as f32 * 255.0 / 99.0).round() as u8
}

/// TX81Z FB (0-7) → 38x6 feedback（opm2x6 と同実装、FB×36）。
fn fb_to_x6(fb: u8) -> u8 {
    fb.min(7) * 36
}

// ---------------------------------------------------------------------------
// オペレーター変換
// ---------------------------------------------------------------------------

fn convert_op(op: &crate::parse::OpzOpData, is_carrier: bool, opts: ConvOptions) -> OperatorParams {
    let mod_tl_cap = opts.mod_tl_cap;
    let (mul, op_fine_tune) = freq_to_mul_fine(op.freq);

    // EGT=1 のとき D2R を強制的に高値にしてリリース挙動を作る
    // (TX81Z は EGT=1 で D1L で止まらず一定レートで減衰 = sustain-less decay)
    let d2r = if op.egt != 0 && op.d2r == 0 { 20 } else { op.d2r };

    OperatorParams {
        tl: if is_carrier { out_to_tl(op.out) } else { out_to_tl_mod(op.out, mod_tl_cap) },
        ar: ar_to_x6(op.ar, op.rs),
        d1r: opm_rate_to_x6(op.d1r, op.rs),
        d2r: opm_rate_to_x6(d2r, op.rs),
        d1l: sl_to_x6(op.d1l),
        rr: rr_to_x6(op.rr, op.rs),
        mul,
        dt1: det_to_x6(op.det),
        ksr: opts.ksr_override.unwrap_or(op.rs.min(3) * 85),
        am_enable: op.ame,
        // キャリアは velocity_sensitivity=0（38x6の「velocity=音量」設計を維持）
        // モジュレーターは KVS を写像: KVS(0-7) → 0..70
        // (* 255/7 は実効TLを最大にクランプさせすぎるため * 10 に抑制)
        velocity_sensitivity: if is_carrier { 0 } else { op.kvs.min(7) * 10 },
        waveform: op.ow.min(7),
        op_fine_tune,
    }
}

// ---------------------------------------------------------------------------
// ボイス変換
// ---------------------------------------------------------------------------

/// 変換オプション（音質追い込み用の上書き群）。
#[derive(Clone, Copy, Debug)]
pub struct ConvOptions {
    /// モジュレーター TL 天井。
    pub mod_tl_cap: u8,
    /// チャンネルフィードバックの上書き（`Some(n)` で 38x6 feedback を直接指定、`None` で .syx 由来）。
    /// 切り分け診断用：`Some(0)` でフィードバックを無効化できる。
    pub fb_override: Option<u8>,
    /// 全オペレーターの KSR（鍵盤レート追従）上書き（`None` で .syx 由来）。
    /// 切り分け診断用：`Some(0)` で高音のエンベロープ加速を弱められる。
    pub ksr_override: Option<u8>,
}

impl Default for ConvOptions {
    fn default() -> Self {
        Self { mod_tl_cap: DEFAULT_MOD_TL_CAP, fb_override: None, ksr_override: None }
    }
}

/// OpzVoice → Ym38x6Patch（オプション指定）。
pub fn voice_to_patch_opts(voice: &OpzVoice, opts: ConvOptions) -> Ym38x6Patch {
    let alg = voice.algorithm.min(7) as usize;
    let carriers = CARRIERS[alg];

    // VCED格納順 [OP4=ops[0], OP3=ops[1], OP2=ops[2], OP1=ops[3]] を
    // 38x6 operators[0..3] = [OP1, OP2, OP3, OP4] に逆順変換する
    let operators = std::array::from_fn(|i| {
        let op = &voice.ops[3 - i]; // operators[i] ← VCED ops[3-i]
        let is_carrier = carriers.contains(&i);
        convert_op(op, is_carrier, opts)
    });

    Ym38x6Patch {
        operators,
        channel: ChannelParams {
            algorithm: alg as u8,
            feedback: opts.fb_override.unwrap_or_else(|| fb_to_x6(voice.feedback)),
            tone_lfo_freq: (voice.lfo_spd as f32 * 255.0 / 99.0).round() as u8,
            tone_lfo_pmd: lfo_depth_to_x6(voice.pmd),
            tone_lfo_amd: lfo_depth_to_x6(voice.amd),
            tone_lfo_delay: (voice.lfo_dly as f32 * 255.0 / 99.0).round() as u8,
            pms: pms_to_x6(voice.pms),
            ams: ams_to_x6(voice.ams),
            ..ChannelParams::default()
        },
    }
}

/// OpzVoice → Ym38x6Patch（既定オプション）。
pub fn voice_to_patch(voice: &OpzVoice) -> Ym38x6Patch {
    voice_to_patch_opts(voice, ConvOptions::default())
}

/// OpzVoice → PresetEntry（オプション指定）。
pub fn voice_to_entry_opts(voice: &OpzVoice, opts: ConvOptions) -> PresetEntry {
    PresetEntry {
        program: (voice.number % 128) as u8,
        name: voice.name.clone(),
        patch: voice_to_patch_opts(voice, opts),
    }
}

/// OpzVoice → PresetEntry（既定オプション）。
pub fn voice_to_entry(voice: &OpzVoice) -> PresetEntry {
    voice_to_entry_opts(voice, ConvOptions::default())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::OpzOpData;

    #[test]
    fn out_to_tl_polarity() {
        assert_eq!(out_to_tl(0), 0);    // 無音
        assert_eq!(out_to_tl(99), 254); // 最大（キャリア用）
        assert!(out_to_tl(50) > 0 && out_to_tl(50) < 254);
    }

    #[test]
    fn out_to_tl_mod_caps_at_given_cap() {
        assert_eq!(out_to_tl_mod(0, 200), 0);
        assert_eq!(out_to_tl_mod(99, 200), 200);
        assert!(out_to_tl_mod(50, 200) > 0 && out_to_tl_mod(50, 200) < 200);
        // 天井を変えると最大値も追従する
        assert_eq!(out_to_tl_mod(99, 254), 254);
        assert_eq!(out_to_tl_mod(99, 180), 180);
    }

    #[test]
    fn det_center_no_detune() {
        assert_eq!(det_to_x6(3), 128); // 中心 = 無デチューン
    }

    #[test]
    fn det_positive_and_negative() {
        assert!(det_to_x6(4) > 128); // 正方向
        assert!(det_to_x6(5) > det_to_x6(4)); // 大きくなる
        assert!(det_to_x6(2) < 128); // 負方向
        assert!(det_to_x6(1) < det_to_x6(2)); // 絶対値増大
    }

    #[test]
    fn freq_integer_mul_gives_center_fine() {
        // FREQ 4 = 1.0x (MUL=1), FREQ 8 = 2.0x (MUL=2) etc. → op_fine_tune=128
        let (m, oft) = freq_to_mul_fine(4);
        assert_eq!(m, 1);
        assert_eq!(oft, 128);

        let (m, oft) = freq_to_mul_fine(8);
        assert_eq!(m, 2);
        assert_eq!(oft, 128);

        let (m, oft) = freq_to_mul_fine(0); // 0.5x
        assert_eq!(m, 0);
        assert_eq!(oft, 128);
    }

    #[test]
    fn freq_sqrt2_interval_near_tritone() {
        // FREQ 5 = 1.414x: MUL=1 か MUL=2 の近傍（600¢）
        let (m, oft) = freq_to_mul_fine(5);
        // ±600¢ なので either MUL=1(oft>128) or MUL=2(oft<128)
        assert!(m == 1 || m == 2);
        if m == 1 { assert!(oft > 128); } else { assert!(oft < 128); }
    }

    #[test]
    fn kvs_carrier_is_zero() {
        let op = OpzOpData { kvs: 7, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 0, "carrier velocity_sensitivity must be 0");
    }

    #[test]
    fn kvs_modulator_maps_70_at_max() {
        let op = OpzOpData { kvs: 7, freq: 4, det: 3, out: 50, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 70);
    }

    #[test]
    fn kvs_modulator_zero_stays_zero() {
        let op = OpzOpData { kvs: 0, freq: 4, det: 3, out: 50, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, ConvOptions::default());
        assert_eq!(p.velocity_sensitivity, 0);
    }

    #[test]
    fn egt1_forces_d2r_nonzero() {
        let op = OpzOpData { d2r: 0, egt: 1, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, ConvOptions::default());
        assert!(p.d2r > 0, "EGT=1 with D2R=0 should force d2r > 0");
    }

    #[test]
    fn egt0_d2r_zero_stays_zero() {
        let op = OpzOpData { d2r: 0, egt: 0, freq: 4, det: 3, out: 99, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, true, ConvOptions::default());
        assert_eq!(p.d2r, 0, "EGT=0 D2R=0 → sustain型（d2r=0のまま）");
    }

    #[test]
    fn waveform_direct_copy() {
        let op = OpzOpData { ow: 5, freq: 4, det: 3, out: 80, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, ConvOptions::default());
        assert_eq!(p.waveform, 5);
    }

    #[test]
    fn op_order_reversal() {
        // VCED: ops[0]=OP4(out=10), ops[3]=OP1(out=80)
        // 38x6: operators[0]=OP1(tl from 80), operators[3]=OP4(tl from 10)
        let mut voice = OpzVoice::default();
        voice.ops[0] = OpzOpData { out: 10, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() }; // OP4
        voice.ops[3] = OpzOpData { out: 80, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() }; // OP1
        let patch = voice_to_patch(&voice);
        // operators[0] ← OP1 (out=80) → tl should be large
        assert!(patch.operators[0].tl > patch.operators[3].tl,
            "operators[0](OP1,out=80) should have higher tl than operators[3](OP4,out=10)");
    }
}
