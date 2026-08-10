//! TX81Z（OPZ/YM2414）レジスタ値 → OP505パッチへの直接変換ロジック。
//!
//! opz2x6（TX81Z→.38x6変換）のEGヘルパー（実機レート→38x6レート写像）と、
//! op505-core adapter.rs の`convert_eg_shape`（38x6レート→TimeEgParams変換）を
//! 1ツール内で合成することで、中間の`.38x6`ファイルを経由せず直接TimeEgParamsを得る。
//! `AttackMode::Bias`は旧2段変換（opz2x6→adapter::convert_operator_eg）とビット単位で一致する
//! （`tests::direct_eg_bias_matches_two_stage_adapter_path`参照、回帰テストの基準として利用）。
//! 実際の既定は聴感A/Bの結果`AttackMode::None`（詳細は[AttackMode]参照）。

use op505_core::adapter::convert_eg_shape;
use op505_core::{Op505ChannelParams, Op505OperatorParams, Op505Patch, Op505PresetEntry};
use opz2x6::conv::{self, ConvOptions, CARRIERS};
use opz2x6::parse::{OpzOpData, OpzVoice};
use sound_core::TimeEgParams;

/// アタック立ち上がりの表現方法。OP505のEG振幅もdBリニア（`operator.rs::compute_env_amp`）
/// なので、38x6と同じ「線形levelランプ=一定dB/s上昇=立ち上がりが遅く聞こえる」問題が
/// そのまま発生する。`Bias`はopz2x6と同じ`ATTACK_ONSET_BIAS`補正を適用し旧2段変換とビット一致させる
/// （回帰テストの基準）。既定は`None`（`--attack`未指定時。2026-08-10、`bias`との聴感差が小さいとの
/// ユーザー判断で採用）。`Curve`は別の質感オプションとしてA/B用に残す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackMode {
    /// opz2x6と同じATTACK_ONSET_BIAS補正を適用（旧2段変換とビット一致、回帰テストの基準・比較用）。
    Bias,
    /// バイアスなし（既定）。
    None,
    /// バイアスなし + stage0のみcurve=1（レイズドコサイン、A/B用）。
    /// dBドメインでは開始点微分0のため`None`より立ち上がりが遅くなる（BIASの代替にはならない）。
    Curve,
}

/// TX81Zの1オペレーターEG(ar/d1r/d2r/rr/d1l/rs/egt)をTimeEgParamsへ直接変換する。
/// 実機レート→38x6レート写像はopz2x6の関数（[conv::ar_to_x6]等）を合成再利用し、
/// 段割り当てはop505-coreの[convert_eg_shape]を合成再利用する（独自数式は書かない）。
pub fn direct_eg(
    op: &OpzOpData,
    is_carrier: bool,
    carrier_sustain: f32,
    attack_mode: AttackMode,
    warnings: &mut Vec<String>,
    label: &str,
) -> TimeEgParams {
    let d2r = conv::effective_d2r(op.d2r, op.egt);

    let x6_ar = match attack_mode {
        AttackMode::Bias => conv::ar_to_x6(op.ar, op.rs),
        AttackMode::None | AttackMode::Curve => conv::opm_rate_to_x6(op.ar, op.rs),
    };
    let mut x6_d1r = conv::opm_rate_to_x6(op.d1r, op.rs);
    let mut x6_d2r = conv::opm_rate_to_x6(d2r, op.rs);
    let mut x6_d1l = conv::sl_to_x6(op.d1l);
    let x6_rr = conv::rr_to_x6(op.rr, op.rs);

    if is_carrier && carrier_sustain > 0.0 {
        conv::apply_carrier_sustain(&mut x6_d1l, &mut x6_d1r, &mut x6_d2r, carrier_sustain);
    }

    let mut eg = convert_eg_shape(x6_ar, x6_d1r, x6_d1l, x6_d2r, x6_rr, 0, 0, 0, warnings, label);
    if attack_mode == AttackMode::Curve {
        eg.stages[0].curve = 1;
    }
    eg
}

/// OpzVoice → Op505Patch（オプション指定）。非EGパラメータは
/// `opz2x6::conv::voice_to_patch_opts`で一度.38x6化してフィールドコピーし
/// （EG以外は完全一致のフィールド構成のため）、EGだけ[direct_eg]で置き換える。
/// TX81ZにはFG（Pitch/Cutoff/Gain）が存在しないため、FG3本はOP505ネイティブの既定値を使う。
pub fn voice_to_op505_patch(
    voice: &OpzVoice,
    opts: ConvOptions,
    attack_mode: AttackMode,
) -> (Op505Patch, Vec<String>) {
    let x6 = conv::voice_to_patch_opts(voice, opts);
    let alg = voice.algorithm.min(7) as usize;
    let carriers = CARRIERS[alg];
    // opz2x6::conv::voice_to_patch_opts と同じ恒等写像（operators[i] ← ops[i]）。
    const OP_SRC: [usize; 4] = [0, 1, 2, 3];

    let mut warnings = Vec::new();
    let operators = std::array::from_fn(|i| {
        let op = &voice.ops[OP_SRC[i]];
        let is_carrier = carriers.contains(&i);
        let label = format!("op{}", i + 1);
        let eg = direct_eg(op, is_carrier, opts.carrier_sustain, attack_mode, &mut warnings, &label);
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

/// OpzVoice → Op505PresetEntry（オプション指定）。
pub fn voice_to_entry_opts(
    voice: &OpzVoice,
    opts: ConvOptions,
    attack_mode: AttackMode,
) -> (Op505PresetEntry, Vec<String>) {
    let (patch, warnings) = voice_to_op505_patch(voice, opts, attack_mode);
    (
        Op505PresetEntry { program: (voice.number % 128) as u8, name: voice.name.clone(), patch },
        warnings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, rs: u8, egt: u8) -> OpzOpData {
        OpzOpData { ar, d1r, d2r, rr, d1l, out: 99, rs, egt, det: 3, ..Default::default() }
    }

    fn to_x6_operator_params(op: &OpzOpData) -> ym38x6_core::OperatorParams {
        ym38x6_core::OperatorParams {
            ar: conv::ar_to_x6(op.ar, op.rs),
            d1r: conv::opm_rate_to_x6(op.d1r, op.rs),
            d2r: conv::opm_rate_to_x6(conv::effective_d2r(op.d2r, op.egt), op.rs),
            d1l: conv::sl_to_x6(op.d1l),
            rr: conv::rr_to_x6(op.rr, op.rs),
            ..ym38x6_core::OperatorParams::default()
        }
    }

    /// 核となる回帰テスト: `AttackMode::Bias`（既定）の直接変換は、
    /// 「opz2x6のx6レート写像 → op505-core adapter::convert_operator_eg」という
    /// 旧2段変換とTimeEgParams完全一致する（独自数式を持たないことの検証）。
    #[test]
    fn direct_eg_bias_matches_two_stage_adapter_path() {
        let cases = [
            make_op(20, 15, 10, 8, 10, 0, 0),
            make_op(31, 31, 31, 15, 15, 3, 0),
            make_op(1, 5, 0, 3, 5, 1, 0),
            make_op(20, 10, 0, 8, 12, 2, 1), // egt=1 & d2r=0 → effective_d2r=20
            make_op(0, 10, 10, 8, 8, 0, 0),  // ar=0 フリーズ
            make_op(20, 0, 10, 8, 8, 0, 0),  // d1r=0
            make_op(20, 10, 0, 8, 8, 0, 0),  // egt=0 & d2r=0
        ];
        for op in cases {
            let mut direct_warnings = Vec::new();
            let direct = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut direct_warnings, "op");

            let x6_op = to_x6_operator_params(&op);
            let two_stage = op505_core::adapter::convert_operator_eg(&x6_op);

            assert_eq!(direct, two_stage, "mismatch for {op:?}");
        }
    }

    #[test]
    fn attack_none_has_no_bias_and_curve_only_shifts_stage0_curve() {
        let op = make_op(20, 15, 10, 8, 10, 0, 0);
        let mut w = Vec::new();
        let none_eg = direct_eg(&op, false, 0.0, AttackMode::None, &mut w, "op");
        let bias_eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut w, "op");
        let curve_eg = direct_eg(&op, false, 0.0, AttackMode::Curve, &mut w, "op");

        // biasはARレートを底上げする分ARの実時間が短くなる = time値が小さくなる。
        assert!(none_eg.stages[0].time > bias_eg.stages[0].time);

        // curveはstage0のみcurve=1、他は0。timeはnoneと同一。
        assert_eq!(curve_eg.stages[0].time, none_eg.stages[0].time);
        assert_eq!(curve_eg.stages[0].curve, 1);
        for i in 1..curve_eg.stage_count as usize {
            assert_eq!(curve_eg.stages[i].curve, 0, "stage {i} should stay linear");
        }
    }

    #[test]
    fn ar_zero_produces_silent_single_stage_with_warning() {
        let op = make_op(0, 10, 10, 8, 8, 0, 0);
        let mut warnings = Vec::new();
        let eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut warnings, "op");
        assert_eq!(eg.stage_count, 2);
        assert_eq!(eg.stages[0].level, 0);
        assert!(warnings.iter().any(|w| w.contains("ar=0")));
    }

    #[test]
    fn d1r_zero_sticks_at_peak_before_release() {
        let op = make_op(20, 0, 10, 8, 8, 0, 0);
        let mut warnings = Vec::new();
        let eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut warnings, "op");
        assert_eq!(eg.stage_count, 2);
        assert_eq!(eg.stages[0].level, 255);
        assert_eq!(eg.release_start, 1);
    }

    #[test]
    fn d2r_zero_without_egt_sticks_at_d1l() {
        let op = make_op(20, 10, 0, 8, 8, 0, 0);
        let mut warnings = Vec::new();
        let eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut warnings, "op");
        assert_eq!(eg.stage_count, 3);
        assert_eq!(eg.loop_start, 1);
        assert_eq!(eg.loop_end, 1);
    }

    #[test]
    fn egt_one_forces_d2r_nonzero_yielding_four_stages() {
        let op = make_op(20, 10, 0, 8, 12, 2, 1);
        let mut warnings = Vec::new();
        let eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut warnings, "op");
        assert_eq!(eg.stage_count, 4);
    }

    #[test]
    fn higher_rate_scaling_shortens_stage_time_monotonically() {
        let mut warnings = Vec::new();
        let times: Vec<u8> = (0..=3)
            .map(|rs| {
                let op = make_op(20, 15, 10, 8, 10, rs, 0);
                direct_eg(&op, false, 0.0, AttackMode::Bias, &mut warnings, "op").stages[1].time
            })
            .collect();
        for i in 1..times.len() {
            assert!(times[i] <= times[i - 1], "d1r time should not increase as rs grows: {times:?}");
        }
    }

    fn make_voice(algorithm: u8, op_ar: [u8; 4], op_out: [u8; 4]) -> OpzVoice {
        let mut voice = OpzVoice { algorithm, name: "Test".to_string(), ..Default::default() };
        for i in 0..4 {
            // d1r/d2r/rr/d1lはデフォルト(0)だと30秒クランプ警告が出る値になるため、
            // 非EGフィールドのコピーを検証する本題と無関係な警告を避ける適当な値を入れる。
            voice.ops[i] = OpzOpData {
                ar: op_ar[i], out: op_out[i], det: 3, d1r: 15, d2r: 10, rr: 8, d1l: 10,
                ..Default::default()
            };
        }
        voice
    }

    #[test]
    fn voice_to_op505_patch_copies_non_eg_fields_from_x6() {
        let voice = make_voice(4, [20, 20, 20, 20], [70, 90, 60, 99]);
        let (patch, warnings) = voice_to_op505_patch(&voice, ConvOptions::default(), AttackMode::Bias);
        let x6 = conv::voice_to_patch_opts(&voice, ConvOptions::default());

        assert!(warnings.is_empty());
        for i in 0..4 {
            assert_eq!(patch.operators[i].tl, x6.operators[i].tl, "op{i} tl mismatch");
            assert_eq!(patch.operators[i].mul, x6.operators[i].mul, "op{i} mul mismatch");
            assert_eq!(patch.operators[i].waveform, x6.operators[i].waveform, "op{i} waveform mismatch");
        }
        assert_eq!(patch.channel.algorithm, x6.channel.algorithm);
        assert_eq!(patch.channel.feedback, x6.channel.feedback);
    }

    #[test]
    fn voice_to_op505_patch_uses_native_fg_defaults() {
        // TX81ZにはFGが存在しないため、FG3本はOP505ネイティブ既定値のまま
        // （adapter::convert_patch経由の無意味な警告を出さないことの確認）。
        let voice = make_voice(0, [20, 20, 20, 20], [80, 80, 80, 80]);
        let (patch, _) = voice_to_op505_patch(&voice, ConvOptions::default(), AttackMode::Bias);
        let default_channel = Op505ChannelParams::default();
        assert_eq!(patch.channel.pitch_fg, default_channel.pitch_fg);
        assert_eq!(patch.channel.cutoff_fg, default_channel.cutoff_fg);
        assert_eq!(patch.channel.gain_fg, default_channel.gain_fg);
    }
}
