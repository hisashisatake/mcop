//! OPM（YM2151）レジスタ値 → OP505パッチへの直接変換ロジック。
//!
//! opm2x6（VOPM形式.opm→.38x6変換）のEGヘルパー（実機レート→38x6レート写像）と、
//! op505-core adapter.rs の`convert_eg_shape`（38x6レート→TimeEgParams変換）を
//! 1ツール内で合成することで、中間の`.38x6`ファイルを経由せず直接TimeEgParamsを得る。
//! opm2x6にはキャリアサステイン等の味付けオプションが存在しないため、
//! opz2op505/psr2op505と異なりis_carrier/carrier_sustainは扱わない。
//! 詳細な設計判断はop505/tools/opz2op505/src/conv.rsのdocコメント参照（同じパターンを踏襲）。

use op505_core::adapter::convert_eg_shape;
use op505_core::{Op505ChannelParams, Op505OperatorParams, Op505Patch, Op505PresetEntry};
use opm2x6::conv;
use opm2x6::parse::{OpmOpReg, OpmVoice, OperatorOrder};
use sound_core::TimeEgParams;

/// アタック立ち上がりの表現方法。詳細はopz2op505::conv::AttackMode参照
/// （同じ問題・同じ選択肢。既定は`None`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackMode {
    /// opm2x6と同じATTACK_ONSET_BIAS補正を適用（旧2段変換とビット一致、回帰テストの基準・比較用）。
    Bias,
    /// バイアスなし（既定）。
    None,
    /// バイアスなし + stage0のみcurve=1（レイズドコサイン、A/B用）。
    Curve,
}

/// OPMの1オペレーターEG(ar/d1r/d2r/rr/d1l/ks)をTimeEgParamsへ直接変換する。
/// 実機レート→38x6レート写像はopm2x6の関数（[conv::opm_ar_to_x6]等）を合成再利用し、
/// 段割り当てはop505-coreの[convert_eg_shape]を合成再利用する（独自数式は書かない）。
pub fn direct_eg(
    op: &OpmOpReg,
    attack_mode: AttackMode,
    warnings: &mut Vec<String>,
    label: &str,
) -> TimeEgParams {
    let x6_ar = match attack_mode {
        AttackMode::Bias => conv::opm_ar_to_x6(op.ar, op.ks),
        AttackMode::None | AttackMode::Curve => conv::opm_rate_to_x6(op.ar, op.ks),
    };
    let x6_d1r = conv::opm_rate_to_x6(op.d1r, op.ks);
    let x6_d2r = conv::opm_rate_to_x6(op.d2r, op.ks);
    let x6_d1l = conv::sl_to_x6(op.d1l);
    let x6_rr = conv::opm_rr_to_x6(op.rr, op.ks);

    let mut eg = convert_eg_shape(x6_ar, x6_d1r, x6_d1l, x6_d2r, x6_rr, 0, 0, 0, warnings, label);
    if attack_mode == AttackMode::Curve {
        eg.stages[0].curve = 1;
    }
    eg
}

/// OpmVoice → Op505Patch（オペレーター順・アタックモード指定）。非EGパラメータは
/// `opm2x6::conv::voice_to_patch`で一度.38x6化してフィールドコピーし
/// （EG以外は完全一致のフィールド構成のため）、EGだけ[direct_eg]で置き換える。
/// OPMにはFG（Pitch/Cutoff/Gain）が存在しないため、FG3本はOP505ネイティブの既定値を使う。
pub fn voice_to_op505_patch(
    voice: &OpmVoice,
    op_order: OperatorOrder,
    attack_mode: AttackMode,
) -> (Op505Patch, Vec<String>) {
    let x6 = conv::voice_to_patch(voice, op_order);
    let ordered = conv::ordered_op_regs(voice, op_order);

    let mut warnings = Vec::new();
    let operators = std::array::from_fn(|i| {
        let op = ordered[i];
        let label = format!("op{}", i + 1);
        let eg = direct_eg(op, attack_mode, &mut warnings, &label);
        let x6_op = &x6.operators[i];
        Op505OperatorParams {
            tl: x6_op.tl,
            eg,
            mul: x6_op.mul,
            dt1: x6_op.dt1,
            ksr: x6_op.ksr,
            am_enable: x6_op.am_enable,
            velocity_sensitivity: x6_op.velocity_sensitivity,
            waveform: x6_op.waveform,
            op_fine_tune: x6_op.op_fine_tune,
            eg_shift: x6_op.eg_shift,
            level_scale: x6_op.level_scale,
            velocity_gain: x6_op.velocity_gain,
        }
    });

    let channel = Op505ChannelParams {
        algorithm: x6.channel.algorithm,
        feedback: x6.channel.feedback,
        chip_lfo_freq: x6.channel.chip_lfo_freq,
        chip_lfo_pmd: x6.channel.chip_lfo_pmd,
        chip_lfo_amd: x6.channel.chip_lfo_amd,
        chip_lfo_delay: x6.channel.chip_lfo_delay,
        pms: x6.channel.pms,
        ams: x6.channel.ams,
        filter_cutoff: x6.channel.filter_cutoff,
        filter_resonance: x6.channel.filter_resonance,
        filter_type: x6.channel.filter_type,
        filter_self_oscillation: x6.channel.filter_self_oscillation,
        texture_lfo: x6.channel.texture_lfo,
        ..Op505ChannelParams::default()
    };

    (Op505Patch { operators, channel }, warnings)
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
    use ym38x6_core::OperatorParams;

    fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, ks: u8) -> OpmOpReg {
        OpmOpReg { ar, d1r, d2r, rr, d1l, ks, mul: 1, dt1: 0, dt2: 0, tl: 0, ams_en: false }
    }

    fn to_x6_operator_params(op: &OpmOpReg) -> OperatorParams {
        OperatorParams {
            ar: conv::opm_ar_to_x6(op.ar, op.ks),
            d1r: conv::opm_rate_to_x6(op.d1r, op.ks),
            d2r: conv::opm_rate_to_x6(op.d2r, op.ks),
            d1l: conv::sl_to_x6(op.d1l),
            rr: conv::opm_rr_to_x6(op.rr, op.ks),
            ..OperatorParams::default()
        }
    }

    /// 核となる回帰テスト: `AttackMode::Bias`（旧2段変換相当）の直接変換は、
    /// 「opm2x6のx6レート写像 → op505-core adapter::convert_operator_eg」という
    /// 旧2段変換とTimeEgParams完全一致する（独自数式を持たないことの検証）。
    #[test]
    fn direct_eg_bias_matches_two_stage_adapter_path() {
        let cases = [
            make_op(20, 15, 10, 8, 10, 0),
            make_op(31, 31, 31, 15, 15, 3),
            make_op(1, 5, 3, 3, 5, 1),
            make_op(0, 10, 10, 8, 8, 0), // ar=0 フリーズ
            make_op(20, 0, 10, 8, 8, 0), // d1r=0
        ];
        for op in cases {
            let mut direct_warnings = Vec::new();
            let direct = direct_eg(&op, AttackMode::Bias, &mut direct_warnings, "op");

            let x6_op = to_x6_operator_params(&op);
            let two_stage = op505_core::adapter::convert_operator_eg(&x6_op);

            assert_eq!(direct, two_stage, "mismatch for {op:?}");
        }
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
    fn voice_to_op505_patch_copies_non_eg_fields_from_x6() {
        let voice = make_voice(4, [20, 20, 20, 20]);
        let (patch, warnings) = voice_to_op505_patch(&voice, OperatorOrder::Direct, AttackMode::Bias);
        let x6 = conv::voice_to_patch(&voice, OperatorOrder::Direct);

        assert!(warnings.is_empty());
        for i in 0..4 {
            assert_eq!(patch.operators[i].tl, x6.operators[i].tl, "op{i} tl mismatch");
            assert_eq!(patch.operators[i].mul, x6.operators[i].mul, "op{i} mul mismatch");
        }
        assert_eq!(patch.channel.algorithm, x6.channel.algorithm);
        assert_eq!(patch.channel.feedback, x6.channel.feedback);
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
