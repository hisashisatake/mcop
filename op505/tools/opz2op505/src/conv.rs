//! TX81Z（OPZ/YM2414）レジスタ値 → OP505パッチへの直接変換ロジック。
//!
//! 実機レート写像・非EGフィールド構築（[crate::map]、opz2x6からの複製）と、
//! op505-core `eg_convert::convert_eg_shape`（EGレートスケール→TimeEgParams変換）を
//! 1ツール内で合成することで、中間の`.38x6`ファイルを経由せず直接TimeEgParamsを得る。
//! `AttackMode::Bias`はゴールデンテスト（`tests/golden.rs`）でデフォーク前の実装と
//! ビット一致することを確認済み。実際の既定は聴感A/Bの結果`AttackMode::None`（詳細は[AttackMode]参照）。

use op505_core::eg_convert::convert_eg_shape;
use op505_core::{Op505ChannelParams, Op505Patch, Op505PresetEntry};
use sound_core::TimeEgParams;

use crate::map::{self, ConvOptions, CARRIERS};
use crate::parse::{OpzOpData, OpzVoice};

/// アタック立ち上がりの表現方法。OP505のEG振幅もdBリニア（`operator.rs::compute_env_amp`）
/// なので、38x6と同じ「線形levelランプ=一定dB/s上昇=立ち上がりが遅く聞こえる」問題が
/// そのまま発生する。`Bias`は[map::ATTACK_ONSET_BIAS]補正を適用する（ゴールデンテストの基準）。
/// 既定は`None`（`--attack`未指定時。2026-08-10、`bias`との聴感差が小さいとの
/// ユーザー判断で採用）。`Curve`は別の質感オプションとしてA/B用に残す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackMode {
    /// [map::ATTACK_ONSET_BIAS]補正を適用（ゴールデンテストの基準・比較用）。
    Bias,
    /// バイアスなし（既定）。
    None,
    /// バイアスなし + stage0のみcurve=1（レイズドコサイン、A/B用）。
    /// dBドメインでは開始点微分0のため`None`より立ち上がりが遅くなる（BIASの代替にはならない）。
    Curve,
}

/// TX81Zの1オペレーターEG(ar/d1r/d2r/rr/d1l/rs/egt)をTimeEgParamsへ直接変換する。
/// 実機レート写像は[map]の関数を合成再利用し、段割り当てはop505-coreの
/// [convert_eg_shape]を合成再利用する（独自数式は書かない）。
pub fn direct_eg(
    op: &OpzOpData,
    is_carrier: bool,
    carrier_sustain: f32,
    attack_mode: AttackMode,
    warnings: &mut Vec<String>,
    label: &str,
) -> TimeEgParams {
    let d2r = map::effective_d2r(op.d2r, op.egt);

    let eg_ar = match attack_mode {
        AttackMode::Bias => map::ar_to_eg_rate(op.ar, op.rs),
        AttackMode::None | AttackMode::Curve => map::rate_to_eg_rate(op.ar, op.rs),
    };
    let mut eg_d1r = map::rate_to_eg_rate(op.d1r, op.rs);
    let mut eg_d2r = map::rate_to_eg_rate(d2r, op.rs);
    let mut eg_d1l = map::sl_to_eg_level(op.d1l);
    let eg_rr = map::rr_to_eg_rate(op.rr, op.rs);

    if is_carrier && carrier_sustain > 0.0 {
        map::apply_carrier_sustain(&mut eg_d1l, &mut eg_d1r, &mut eg_d2r, carrier_sustain);
    }

    let mut eg = convert_eg_shape(eg_ar, eg_d1r, eg_d1l, eg_d2r, eg_rr, 0, 0, 0, warnings, label);
    if attack_mode == AttackMode::Curve {
        eg.stages[0].curve = 1;
    }
    eg
}

/// OpzVoice → Op505Patch（オプション指定）。非EGフィールドは[map::convert_op]/
/// [map::convert_channel]で直接構築し、EGだけ[direct_eg]で埋める。
/// TX81ZにはFG（Pitch/Cutoff/Gain）が存在しないため、FG3本はOP505ネイティブの既定値を使う。
pub fn voice_to_op505_patch(
    voice: &OpzVoice,
    opts: ConvOptions,
    attack_mode: AttackMode,
) -> (Op505Patch, Vec<String>) {
    let alg = voice.algorithm.min(7) as usize;
    let carriers = CARRIERS[alg];
    // opz2x6::conv::voice_to_patch_opts と同じ恒等写像（operators[i] ← ops[i]）。
    const OP_SRC: [usize; 4] = [0, 1, 2, 3];
    let atten = map::alg_atten(alg as u8);
    let pitch_fold = map::pitch_fold_for(voice, opts);

    let mut warnings = Vec::new();
    let operators = std::array::from_fn(|i| {
        let op = &voice.ops[OP_SRC[i]];
        let is_carrier = carriers.contains(&i);
        let label = format!("op{}", i + 1);
        let eg = direct_eg(op, is_carrier, opts.carrier_sustain, attack_mode, &mut warnings, &label);
        let mut params = map::convert_op(op, is_carrier, atten, opts, pitch_fold);
        params.eg = eg;
        params
    });

    let ch = map::convert_channel(voice, alg as u8, opts);
    let channel = Op505ChannelParams {
        algorithm: ch.algorithm,
        feedback: ch.feedback,
        chip_lfo_freq: ch.chip_lfo_freq,
        chip_lfo_pmd: ch.chip_lfo_pmd,
        chip_lfo_amd: ch.chip_lfo_amd,
        chip_lfo_delay: ch.chip_lfo_delay,
        pms: ch.pms,
        ams: ch.ams,
        filter_cutoff: ch.filter_cutoff,
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
        assert_eq!(eg.release_point, 0, "保持は段0のみ、リリースは段1");
    }

    #[test]
    fn d2r_zero_without_egt_sticks_at_d1l() {
        let op = make_op(20, 10, 0, 8, 8, 0, 0);
        let mut warnings = Vec::new();
        let eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut warnings, "op");
        assert_eq!(eg.stage_count, 3);
        assert_eq!(eg.loop_start, 1);
        assert_eq!(eg.release_point, 1);
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
            voice.ops[i] = OpzOpData {
                ar: op_ar[i], out: op_out[i], det: 3, d1r: 15, d2r: 10, rr: 8, d1l: 10,
                ..Default::default()
            };
        }
        voice
    }

    #[test]
    fn voice_to_op505_patch_uses_native_fg_defaults() {
        // TX81ZにはFGが存在しないため、FG3本はOP505ネイティブ既定値のまま。
        let voice = make_voice(0, [20, 20, 20, 20], [80, 80, 80, 80]);
        let (patch, _) = voice_to_op505_patch(&voice, ConvOptions::default(), AttackMode::Bias);
        let default_channel = Op505ChannelParams::default();
        assert_eq!(patch.channel.pitch_fg, default_channel.pitch_fg);
        assert_eq!(patch.channel.cutoff_fg, default_channel.cutoff_fg);
        assert_eq!(patch.channel.gain_fg, default_channel.gain_fg);
    }
}
