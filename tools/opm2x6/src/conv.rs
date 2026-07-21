//! OPM（YM2151）レジスタ値 → ym38x6 パッチへの変換ロジック。
//!
//! Python版 convert.py からの主な改善点:
//! - DT2（2-bit粗デチューン）を op_fine_tune にマッピング（以前は捨てていた）
//! - slot_mask で無効なOPのTLを0に強制（以前は無視していた）
//! - DT1 をルックアップテーブルで正確に変換（以前は±127まで広げすぎていた）
//! - AR/D1R/D2R/RR をKSR補正付きレートに変換＋ARにオンセット補正バイアスを適用

use ym38x6_core::{ChannelParams, OperatorParams, PresetEntry, Ym38x6Patch};

use crate::parse::{OpmOpReg, OpmVoice, OperatorOrder};

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// TL（OPM 7-bit、0=最大音量、127=最小音量）→ 38x6 TL（0=最小、254=最大）。
#[inline]
fn tl_to_x6(tl: u8) -> u8 {
    (127 - tl.min(127)) * 2
}

/// D1L/SL（OPM 4-bit、0-15）→ 38x6 D1L（0=ほぼ無音、255=フルレベル）。
/// reg=0..14: -3dB/step、reg=15: -93dBへジャンプ（実機準拠）。
fn sl_to_x6(reg: u8) -> u8 {
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    (255.0 * (1.0 + db / 93.0)).round() as u8
}

/// DT1（OPM/OPN共通 3-bit、0-7）→ 38x6 dt1（中心128、±50セント）。
/// 実機最大デチューンは約±3.24セント。ルックアップテーブルで近似（mucom2x6と同一テーブル）。
fn dt1_to_x6(dt1: u8) -> u8 {
    const TABLE: [u8; 8] = [128, 131, 134, 136, 128, 125, 122, 120];
    TABLE[(dt1 & 7) as usize]
}

/// DT2（OPM固有 2-bit、0-3）→ 38x6 op_fine_tune（中心128、±1200セント）。
/// OPMのDT2はモジュレーターを大きくシフトしてインハーモニックな金属・ベル系音を作る。
/// 各値のセントオフセット: [0¢, +600¢(tritone/√2), +781¢(π/2), +950¢]
/// op_fine_tune = 128 + round(cents × 128 / 1200)
fn dt2_to_op_fine_tune(dt2: u8) -> u8 {
    const TABLE: [u8; 4] = [128, 192, 211, 229];
    TABLE[(dt2 & 3) as usize]
}

/// OPM 5-bit レート（AR/D1R/D2R、0-31）+ KS（0-3）→ 38x6 rate（0-255）。
/// OPN/OPMで共通のEGレート式: eg_rate_eff = 2×rate + KSR補正（A4基準）。
fn opm_rate_to_x6(rate: u8, ks: u8) -> u8 {
    if rate == 0 { return 0; }
    // A4のkeycode=19の導出はopz2x6::conv::KEY_CODE_A4のコメント参照（(block<<2)|(note>>2)、block=4,note=12）。
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (2 * rate as u16 + ksr_add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// dBリニアアタックの可聴オンセット遅れをOPM実機に寄せる補正バイアス。
/// mucom2x6/conv.rs と同値・同根拠（詳細は mucom2x6/conv.rs の ATTACK_ONSET_BIAS コメント参照）。
const ATTACK_ONSET_BIAS: u16 = 30;

/// OPM AR（5-bit）+ KS → 38x6 ar（オンセット補正付き）。
fn opm_ar_to_x6(ar: u8, ks: u8) -> u8 {
    if ar == 0 { return 0; }
    (opm_rate_to_x6(ar, ks) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPM RR（4-bit、0-15）+ KS → 38x6 rr。
fn opm_rr_to_x6(rr: u8, ks: u8) -> u8 {
    // A4のkeycode=19の導出はopz2x6::conv::KEY_CODE_A4のコメント参照（(block<<2)|(note>>2)、block=4,note=12）。
    const KEY_CODE_A4: u16 = 19;
    let ksr_shift = 3u16.saturating_sub(ks.min(3) as u16);
    let ksr_add = KEY_CODE_A4 >> ksr_shift;
    let eg_rate = (4 * rr as u16 + 2 + ksr_add).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// OPM KS(0-3、rate scaling)→ 38x6 ksr(0-255、実行時の音域依存レート倍率)。
///
/// エンジンの`ksr_rate_multiplier`は`exponent=ksr/255`の線形カーブ
/// （ksr=255で1オクターブごとに2倍、ksr=0で音域に依らず常に1.0）。
/// 実機の絶対keycode法則(eg_rate=2×R+keycode>>(3-KS))はKS=0〜3で2倍刻み
/// (1x,2x,4x,8x)のため、本来はKS=0→exponent 0.125だが、実機比較
/// （MUCOM88実機C2/C6、mucom2x6経由）で「KS=0は音域依存がほぼ無い方が近い」と
/// 判定されたため、KS=0のみ理論値でなく聴感を優先してksr=0(フラット)とする。
/// KS=2=128(exponent≈0.5)は他コンバーター(opz2x6)のGrandPianoキャリア実機比較で
/// 検証済み、KS=3=255(exponent=1.0)は理論値。opz2x6/mucom2x6/psr2x6の同名関数と同一テーブル。
fn ks_to_ksr(ks: u8) -> u8 {
    const TABLE: [u8; 4] = [0, 64, 128, 255];
    TABLE[ks.min(3) as usize]
}

/// OPM AMS sensitivity（CH 2-bit、0-3）→ 38x6 ams（0=off、1/128/255）。
fn ams_to_x6(reg: u8) -> u8 {
    if reg == 0 { return 0; }
    (1u16 + 127 * (reg.min(3) as u16 - 1)) as u8
}

/// OPM PMS sensitivity（CH 3-bit、0-7）→ 38x6 pms（0=off、等間隔1-255）。
fn pms_to_x6(reg: u8) -> u8 {
    if reg == 0 { return 0; }
    (1.0_f32 + 254.0 * (reg.min(7) - 1) as f32 / 6.0).round() as u8
}

/// AMD/PMD（7-bit、0-127）→ 38x6 depth（0-255）。直線スケール。
fn lfo_depth_to_x6(reg: u8) -> u8 {
    (reg as f32 * 255.0 / 127.0).round() as u8
}

// ---------------------------------------------------------------------------
// 構造体変換
// ---------------------------------------------------------------------------

/// OPM SLOT（CH行の slot フィールド）から各OPのミュートフラグを返す。
/// 戻り値: [M1_muted, C1_muted, M2_muted, C2_muted]
///
/// YM2151 SLOTレジスタ: bit3=M1, bit4=C1, bit5=M2, bit6=C2。
/// 対応ビットが0のOPはキーオン対象外→TLを0に強制してミュートする。
fn slot_muted(slot: u8) -> [bool; 4] {
    [
        (slot & (1 << 3)) == 0, // M1
        (slot & (1 << 4)) == 0, // C1
        (slot & (1 << 5)) == 0, // M2
        (slot & (1 << 6)) == 0, // C2
    ]
}

/// OPMオペレーターレジスタ → OperatorParams。
/// `muted` のときはTLを0に固定（slot_maskで無効化されたOP）。
fn convert_op(op: &OpmOpReg, muted: bool) -> OperatorParams {
    OperatorParams {
        tl: if muted { 0 } else { tl_to_x6(op.tl) },
        ar: opm_ar_to_x6(op.ar, op.ks),
        d1r: opm_rate_to_x6(op.d1r, op.ks),
        d2r: opm_rate_to_x6(op.d2r, op.ks),
        d1l: sl_to_x6(op.d1l),
        rr: opm_rr_to_x6(op.rr, op.ks),
        mul: op.mul.min(15),
        dt1: dt1_to_x6(op.dt1),
        ksr: ks_to_ksr(op.ks),
        am_enable: op.ams_en,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: dt2_to_op_fine_tune(op.dt2),
        floor: 0,
        loop_enabled: 0,
        curve: 0,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    }
}

/// OPMボイス → Ym38x6Patch。
/// `op_order` でオペレーター並び順を指定する:
/// - Direct（デフォルト）: M1,C1,M2,C2 → op[0..3]
/// - Register:             M1,M2,C1,C2 → op[0..3]
pub fn voice_to_patch(voice: &OpmVoice, op_order: OperatorOrder) -> Ym38x6Patch {
    let muted = slot_muted(voice.slot);

    // 物理OP配列: インデックス [0]=M1, [1]=C1, [2]=M2, [3]=C2
    let phys: [(&OpmOpReg, bool); 4] = [
        (&voice.m1, muted[0]),
        (&voice.c1, muted[1]),
        (&voice.m2, muted[2]),
        (&voice.c2, muted[3]),
    ];

    // operators[0..3]へ割り当てる物理OPインデックス
    // Direct:   [M1=0, C1=1, M2=2, C2=3]
    // Register: [M1=0, M2=2, C1=1, C2=3]
    let idx: [usize; 4] = match op_order {
        OperatorOrder::Direct   => [0, 1, 2, 3],
        OperatorOrder::Register => [0, 2, 1, 3],
    };

    let operators = std::array::from_fn(|i| {
        let (op, m) = phys[idx[i]];
        convert_op(op, m)
    });

    Ym38x6Patch {
        operators,
        channel: ChannelParams {
            algorithm: voice.con.min(7),
            // FL(3-bit, 0-7) → feedback: ×36（mucom2x6と同一スケール、実機FBノイズ再現調整済み）
            feedback: voice.fl.min(7) * 36,
            // LFRQは直接コピー（OPMのLFRQレートテーブルとym38x6は完全一致しないが低優先）
            chip_lfo_freq: voice.lfrq,
            chip_lfo_pmd: lfo_depth_to_x6(voice.pmd),
            chip_lfo_amd: lfo_depth_to_x6(voice.amd),
            chip_lfo_delay: 0,
            pms: pms_to_x6(voice.pms),
            ams: ams_to_x6(voice.ams),
            // フィルターはOPMに相当パラメーターなし → ChannelParams::default()の全開設定を使う
            ..ChannelParams::default()
        },
    }
}

/// OPMボイス → PresetEntry（ファイル出力用）。
pub fn voice_to_entry(voice: &OpmVoice, op_order: OperatorOrder) -> PresetEntry {
    let name = if voice.name.is_empty() {
        format!("voice{}", voice.number)
    } else {
        voice.name.clone()
    };
    PresetEntry {
        program: (voice.number % 128) as u8,
        name,
        patch: voice_to_patch(voice, op_order),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{OpmOpReg, OpmVoice};

    #[test]
    fn tl_polarity_inverts() {
        assert_eq!(tl_to_x6(0), 254);
        assert_eq!(tl_to_x6(127), 0);
    }

    #[test]
    fn sl_to_x6_bounds_and_jump() {
        assert_eq!(sl_to_x6(0), 255);  // 0dB = フルレベル
        assert_eq!(sl_to_x6(15), 0);  // -93dB = ほぼ無音
        // reg=14は-42dB: 255*(1-42/93) ≈ 140
        let v14 = sl_to_x6(14);
        assert!(v14 > 0 && v14 < 255);
    }

    #[test]
    fn dt1_center_and_direction() {
        assert_eq!(dt1_to_x6(0), 128); // +0
        assert_eq!(dt1_to_x6(4), 128); // +0 (OPM/OPN共通: DT1=4はDT1=0と同じ)
        assert!(dt1_to_x6(1) > 128);   // 正方向
        assert!(dt1_to_x6(5) < 128);   // 負方向
        // 最大デチューンは実機±3.24セント相当（±8 unitsで収まるはず）
        assert!(dt1_to_x6(3) <= 136);
        assert!(dt1_to_x6(7) >= 120);
    }

    #[test]
    fn dt2_no_detune_gives_center() {
        assert_eq!(dt2_to_op_fine_tune(0), 128);
    }

    #[test]
    fn dt2_positive_offsets() {
        // DT2=1: +600¢ → 128+64=192
        assert_eq!(dt2_to_op_fine_tune(1), 192);
        // DT2=2 > DT2=1 > DT2=0
        assert!(dt2_to_op_fine_tune(2) > dt2_to_op_fine_tune(1));
        assert!(dt2_to_op_fine_tune(3) > dt2_to_op_fine_tune(2));
        // 全て128以上（DT2はプラス方向のみ）
        for i in 0u8..4 {
            assert!(dt2_to_op_fine_tune(i) >= 128);
        }
    }

    #[test]
    fn slot_all_active() {
        // slot=120 (0b01111000) → 全OP有効
        let m = slot_muted(120);
        assert!(!m[0] && !m[1] && !m[2] && !m[3]);
    }

    #[test]
    fn slot_m1_c1_inactive() {
        // slot=96 (0b01100000) → M2(bit5)・C2(bit6)のみ有効、M1(bit3)・C1(bit4)はミュート
        let m = slot_muted(96);
        assert!(m[0],  "M1 should be muted");
        assert!(m[1],  "C1 should be muted");
        assert!(!m[2], "M2 should be active");
        assert!(!m[3], "C2 should be active");
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
        assert!(p.tl > 0, "active op should be audible");
    }

    #[test]
    fn opm_ar_applies_onset_bias() {
        assert_eq!(opm_ar_to_x6(0, 0), 0); // AR=0はフリーズ（補正なし）
        let base = opm_rate_to_x6(18, 0);
        assert_eq!(opm_ar_to_x6(18, 0), (base as u16 + ATTACK_ONSET_BIAS).min(255) as u8);
        assert_eq!(opm_ar_to_x6(31, 0), 255); // 高速ARは255にクランプ
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

        let direct = voice_to_patch(&voice, OperatorOrder::Direct);
        let reg    = voice_to_patch(&voice, OperatorOrder::Register);

        // Direct: [M1, C1, M2, C2]
        assert_eq!(direct.operators[0].mul, voice.m1.mul);
        assert_eq!(direct.operators[1].mul, voice.c1.mul);
        assert_eq!(direct.operators[2].mul, voice.m2.mul);
        assert_eq!(direct.operators[3].mul, voice.c2.mul);

        // Register: [M1, M2, C1, C2]
        assert_eq!(reg.operators[0].mul, voice.m1.mul);
        assert_eq!(reg.operators[1].mul, voice.m2.mul);
        assert_eq!(reg.operators[2].mul, voice.c1.mul);
        assert_eq!(reg.operators[3].mul, voice.c2.mul);
    }

    #[test]
    fn e_piano_voice_conversion() {
        // sample.opmのE.PIANOをDirect変換し、主要フィールドを検証
        let text = include_str!("../sample.opm");
        let voices = crate::parse::parse_opm(text).unwrap();
        let patch = voice_to_patch(&voices[0], OperatorOrder::Direct);

        assert_eq!(patch.channel.algorithm, 4); // CON=4
        assert_eq!(patch.channel.feedback, 7 * 36); // FL=7 → 252
        // M1: TL=25(非ゼロなので可聴), AR=31(最速), DT2=0(op_fine_tune=128)
        assert!(patch.operators[0].tl > 0);
        assert_eq!(patch.operators[0].op_fine_tune, 128);
    }
}
