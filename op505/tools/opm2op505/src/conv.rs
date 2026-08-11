//! OPM（YM2151）レジスタ値 → OP505パッチへの直接変換ロジック。
//!
//! 実機レート写像・非EGフィールド構築（[crate::map]、opm2x6からの複製）と、
//! op505-core `eg_convert::convert_eg_shape`（EGレートスケール→TimeEgParams変換）を
//! 1ツール内で合成することで、中間の`.38x6`ファイルを経由せず直接TimeEgParamsを得る。
//! opm2x6にはキャリアサステイン等の味付けオプションが存在しないため、
//! opz2op505/psr2op505と異なりis_carrier/carrier_sustainは扱わない。
//! 詳細な設計判断はop505/tools/opz2op505/src/conv.rsのdocコメント参照（同じパターンを踏襲）。

use op505_core::eg_convert::convert_eg_shape;
use op505_core::{Op505ChannelParams, Op505Patch, Op505PresetEntry};
use sound_core::TimeEgParams;

use crate::map;
use crate::parse::{OpmOpReg, OpmVoice, OperatorOrder};

/// アタック立ち上がりの表現方法。詳細はopz2op505::conv::AttackMode参照
/// （同じ問題・同じ選択肢。既定は`None`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackMode {
    /// アタック立ち上がりの聴感補正バイアスを適用（ゴールデンテストの基準・比較用）。
    Bias,
    /// バイアスなし（既定）。
    None,
    /// バイアスなし + stage0のみcurve=1（レイズドコサイン、A/B用）。
    Curve,
}

/// OPMの1オペレーターEG(ar/d1r/d2r/rr/d1l/ks)をTimeEgParamsへ直接変換する。
/// 実機レート写像は[map]の関数を合成再利用し、段割り当てはop505-coreの
/// [convert_eg_shape]を合成再利用する（独自数式は書かない）。
pub fn direct_eg(
    op: &OpmOpReg,
    attack_mode: AttackMode,
    warnings: &mut Vec<String>,
    label: &str,
) -> TimeEgParams {
    let eg_ar = match attack_mode {
        AttackMode::Bias => map::ar_to_eg_rate(op.ar, op.ks),
        AttackMode::None | AttackMode::Curve => map::rate_to_eg_rate(op.ar, op.ks),
    };
    let eg_d1r = map::rate_to_eg_rate(op.d1r, op.ks);
    let eg_d2r = map::rate_to_eg_rate(op.d2r, op.ks);
    let eg_d1l = map::sl_to_eg_level(op.d1l);
    let eg_rr = map::rr_to_eg_rate(op.rr, op.ks);

    let mut eg = convert_eg_shape(eg_ar, eg_d1r, eg_d1l, eg_d2r, eg_rr, 0, 0, 0, warnings, label);
    if attack_mode == AttackMode::Curve {
        eg.stages[0].curve = 1;
    }
    eg
}

/// OpmVoice → Op505Patch（オペレーター順・アタックモード指定）。非EGフィールドは
/// [map::convert_ops]/[map::convert_channel]で直接構築し、EGだけ[direct_eg]で埋める。
/// OPMにはFG（Pitch/Cutoff/Gain）が存在しないため、FG3本はOP505ネイティブの既定値を使う。
pub fn voice_to_op505_patch(
    voice: &OpmVoice,
    op_order: OperatorOrder,
    attack_mode: AttackMode,
) -> (Op505Patch, Vec<String>) {
    let ordered = map::ordered_op_regs(voice, op_order);
    let mut non_eg = map::convert_ops(voice, op_order);

    let mut warnings = Vec::new();
    for i in 0..4 {
        let label = format!("op{}", i + 1);
        non_eg[i].eg = direct_eg(ordered[i], attack_mode, &mut warnings, &label);
    }

    let ch = map::convert_channel(voice);
    let channel = Op505ChannelParams {
        algorithm: ch.algorithm,
        feedback: ch.feedback,
        chip_lfo_freq: ch.chip_lfo_freq,
        chip_lfo_pmd: ch.chip_lfo_pmd,
        chip_lfo_amd: ch.chip_lfo_amd,
        pms: ch.pms,
        ams: ch.ams,
        ..Op505ChannelParams::default()
    };

    (Op505Patch { operators: non_eg, channel }, warnings)
}

/// OpmVoice → Op505PresetEntry（ファイル出力用）。
pub fn voice_to_entry(
    voice: &OpmVoice,
    op_order: OperatorOrder,
    attack_mode: AttackMode,
) -> (Op505PresetEntry, Vec<String>) {
    let name = if voice.name.is_empty() { format!("voice{}", voice.number) } else { voice.name.clone() };
    let (patch, warnings) = voice_to_op505_patch(voice, op_order, attack_mode);
    (Op505PresetEntry { program: (voice.number % 128) as u8, name, patch }, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, ks: u8) -> OpmOpReg {
        OpmOpReg { ar, d1r, d2r, rr, d1l, ks, mul: 1, dt1: 0, dt2: 0, tl: 0, ams_en: false }
    }

    #[test]
    fn attack_none_has_no_bias_and_curve_only_shifts_stage0_curve() {
        let op = make_op(20, 15, 10, 8, 10, 0);
        let mut w = Vec::new();
        let none_eg = direct_eg(&op, AttackMode::None, &mut w, "op");
        let bias_eg = direct_eg(&op, AttackMode::Bias, &mut w, "op");
        let curve_eg = direct_eg(&op, AttackMode::Curve, &mut w, "op");

        assert!(none_eg.stages[0].time > bias_eg.stages[0].time);
        assert_eq!(curve_eg.stages[0].time, none_eg.stages[0].time);
        assert_eq!(curve_eg.stages[0].curve, 1);
        for i in 1..curve_eg.stage_count as usize {
            assert_eq!(curve_eg.stages[i].curve, 0, "stage {i} should stay linear");
        }
    }

    #[test]
    fn ar_zero_produces_silent_single_stage_with_warning() {
        let op = make_op(0, 10, 10, 8, 8, 0);
        let mut warnings = Vec::new();
        let eg = direct_eg(&op, AttackMode::Bias, &mut warnings, "op");
        assert_eq!(eg.stage_count, 2);
        assert_eq!(eg.stages[0].level, 0);
        assert!(warnings.iter().any(|w| w.contains("ar=0")));
    }

    fn make_voice(con: u8, op_ar: [u8; 4]) -> OpmVoice {
        OpmVoice {
            number: 0,
            name: "Test".to_string(),
            lfrq: 0,
            pmd: 0,
            amd: 0,
            lfo_wf: 2,
            fl: 3,
            con,
            pms: 0,
            ams: 0,
            slot: 120,
            m1: OpmOpReg { ar: op_ar[0], d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 20, mul: 1, dt1: 0, dt2: 0, ks: 1, ams_en: false },
            c1: OpmOpReg { ar: op_ar[1], d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 21, mul: 1, dt1: 0, dt2: 0, ks: 1, ams_en: false },
            m2: OpmOpReg { ar: op_ar[2], d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 22, mul: 1, dt1: 0, dt2: 0, ks: 1, ams_en: false },
            c2: OpmOpReg { ar: op_ar[3], d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 23, mul: 1, dt1: 0, dt2: 0, ks: 1, ams_en: false },
        }
    }

    #[test]
    fn voice_to_op505_patch_uses_native_fg_defaults() {
        let voice = make_voice(0, [20, 20, 20, 20]);
        let (patch, _) = voice_to_op505_patch(&voice, OperatorOrder::Direct, AttackMode::Bias);
        let default_channel = Op505ChannelParams::default();
        assert_eq!(patch.channel.pitch_fg, default_channel.pitch_fg);
        assert_eq!(patch.channel.cutoff_fg, default_channel.cutoff_fg);
        assert_eq!(patch.channel.gain_fg, default_channel.gain_fg);
    }

    #[test]
    fn register_order_swaps_c1_m2_like_opm2x6() {
        let voice = make_voice(4, [20, 20, 20, 20]);
        let direct = voice_to_op505_patch(&voice, OperatorOrder::Direct, AttackMode::None).0;
        let reg = voice_to_op505_patch(&voice, OperatorOrder::Register, AttackMode::None).0;
        // Direct: [M1,C1,M2,C2] / Register: [M1,M2,C1,C2]。TL(20/21/22/23)で判別。
        assert_eq!(direct.operators[1].tl, reg.operators[2].tl);
        assert_eq!(direct.operators[2].tl, reg.operators[1].tl);
    }
}
