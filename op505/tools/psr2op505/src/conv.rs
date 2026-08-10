//! PSR-70（OPQ/YM3806）ボイスレジスタ → OP505パッチへの直接変換ロジック。
//!
//! psr2x6（PSR-70→.38x6変換）のEGヘルパー（実機レート→38x6レート写像）と、
//! op505-core adapter.rs の`convert_eg_shape`（38x6レート→TimeEgParams変換）を
//! 1ツール内で合成することで、中間の`.38x6`ファイルを経由せず直接TimeEgParamsを得る。
//! 詳細な設計判断はop505/tools/opz2op505/src/conv.rsのdocコメント参照（同じパターンを踏襲）。

use op505_core::adapter::convert_eg_shape;
use op505_core::{Op505ChannelParams, Op505OperatorParams, Op505Patch, Op505PresetEntry, Op505PresetFile};
use psr2x6::conv::{self, NamedVoice, OpqOperator, OpqVoice, PsrConvOptions};
use sound_core::TimeEgParams;
use ym38x6_core::OperatorParams;

/// アタック立ち上がりの表現方法。詳細はopz2op505::conv::AttackMode参照
/// （同じ問題・同じ選択肢。既定は`None`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackMode {
    /// psr2x6と同じATTACK_ONSET_BIAS補正を適用（旧2段変換とビット一致、回帰テストの基準・比較用）。
    Bias,
    /// バイアスなし（既定）。
    None,
    /// バイアスなし + stage0のみcurve=1（レイズドコサイン、A/B用）。
    Curve,
}

/// PSR-70の1オペレーターEG(ar/d1r/d2r/rr/d1l/ksr)をTimeEgParamsへ直接変換する。
/// 実機レート→38x6レート写像はpsr2x6の関数（[conv::opq_ar_to_x6]等）を合成再利用し、
/// 段割り当てはop505-coreの[convert_eg_shape]を合成再利用する（独自数式は書かない）。
pub fn direct_eg(
    op: &OpqOperator,
    is_carrier: bool,
    carrier_sustain: f32,
    attack_mode: AttackMode,
    warnings: &mut Vec<String>,
    label: &str,
) -> TimeEgParams {
    let x6_ar = match attack_mode {
        AttackMode::Bias => conv::opq_ar_to_x6(op.ar, op.ksr),
        AttackMode::None | AttackMode::Curve => conv::opq_rate_to_x6(op.ar, op.ksr),
    };
    let mut x6_d1r = conv::opq_rate_to_x6(op.d1r, op.ksr);
    let mut x6_d2r = conv::opq_rate_to_x6(op.d2r, op.ksr);
    let mut x6_d1l = conv::sl_opq_to_x6(op.d1l);
    let x6_rr = conv::opq_rr_to_x6(op.rr, op.ksr);

    if is_carrier && carrier_sustain > 0.0 {
        // psr2x6::conv::apply_carrier_sustainはOperatorParams全体を受け取るため、
        // EG関連3フィールドだけを積んだ一時値で合成再利用する（独自数式を書かない）。
        let mut tmp = OperatorParams { d1l: x6_d1l, d1r: x6_d1r, d2r: x6_d2r, ..OperatorParams::default() };
        conv::apply_carrier_sustain(&mut tmp, carrier_sustain);
        x6_d1l = tmp.d1l;
        x6_d1r = tmp.d1r;
        x6_d2r = tmp.d2r;
    }

    let mut eg = convert_eg_shape(x6_ar, x6_d1r, x6_d1l, x6_d2r, x6_rr, 0, 0, 0, warnings, label);
    if attack_mode == AttackMode::Curve {
        eg.stages[0].curve = 1;
    }
    eg
}

/// OpqVoice → Op505Patch（オプション指定）。非EGパラメータは
/// `psr2x6::conv::to_ym38x6_patch_opts`で一度.38x6化してフィールドコピーし
/// （EG以外は完全一致のフィールド構成のため）、EGだけ[direct_eg]で置き換える。
/// PSR-70にはFG（Pitch/Cutoff/Gain）が存在しないため、FG3本はOP505ネイティブの既定値を使う。
pub fn voice_to_op505_patch(
    voice: &OpqVoice,
    opts: PsrConvOptions,
    attack_mode: AttackMode,
) -> (Op505Patch, Vec<String>) {
    let x6 = (*voice).to_ym38x6_patch_opts(opts);
    let alg = voice.algorithm.min(7) as usize;
    let carriers = conv::CARRIERS[alg];

    let mut warnings = Vec::new();
    let operators = std::array::from_fn(|i| {
        let op = &voice.operators[i];
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

/// 名前付きボイス列 → `.op505` プリセットファイル群（オプション指定）。
/// psr2x6::conv::voices_to_preset_files_optsと同じバンク分割規則
/// （Programは0〜127のため128件ごとに連番バンクへ分割）。
/// 戻り値の第2要素は音色名ごとの変換警告（空でない音色のみ）。
pub fn voices_to_op505_preset_files_opts(
    start_bank: u16,
    voices: &[NamedVoice],
    opts: PsrConvOptions,
    attack_mode: AttackMode,
) -> (Vec<Op505PresetFile>, Vec<(String, Vec<String>)>) {
    let mut all_warnings = Vec::new();
    let files = voices
        .chunks(128)
        .enumerate()
        .map(|(bank_index, chunk)| {
            let bank = start_bank + bank_index as u16;
            let presets = chunk
                .iter()
                .map(|nv| {
                    let (patch, warnings) = voice_to_op505_patch(&nv.voice, opts, attack_mode);
                    if !warnings.is_empty() {
                        all_warnings.push((nv.name.clone(), warnings));
                    }
                    patch
                })
                .enumerate()
                .map(|(program, patch)| Op505PresetEntry {
                    program: program as u8,
                    name: chunk[program].name.clone(),
                    patch,
                })
                .collect();
            Op505PresetFile::Presets { bank, presets }
        })
        .collect();
    (files, all_warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, ksr: u8) -> OpqOperator {
        OpqOperator { ar, d1r, d2r, rr, d1l, ksr, mul: 1, detune: 32, tl: 0, am_enable: false }
    }

    fn to_x6_operator_params(op: &OpqOperator) -> OperatorParams {
        OperatorParams {
            ar: conv::opq_ar_to_x6(op.ar, op.ksr),
            d1r: conv::opq_rate_to_x6(op.d1r, op.ksr),
            d2r: conv::opq_rate_to_x6(op.d2r, op.ksr),
            d1l: conv::sl_opq_to_x6(op.d1l),
            rr: conv::opq_rr_to_x6(op.rr, op.ksr),
            ..OperatorParams::default()
        }
    }

    /// 核となる回帰テスト: `AttackMode::Bias`（旧2段変換相当）の直接変換は、
    /// 「psr2x6のx6レート写像 → op505-core adapter::convert_operator_eg」という
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
            let direct = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut direct_warnings, "op");

            let x6_op = to_x6_operator_params(&op);
            let two_stage = op505_core::adapter::convert_operator_eg(&x6_op);

            assert_eq!(direct, two_stage, "mismatch for {op:?}");
        }
    }

    #[test]
    fn attack_none_has_no_bias_and_curve_only_shifts_stage0_curve() {
        let op = make_op(20, 15, 10, 8, 10, 0);
        let mut w = Vec::new();
        let none_eg = direct_eg(&op, false, 0.0, AttackMode::None, &mut w, "op");
        let bias_eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut w, "op");
        let curve_eg = direct_eg(&op, false, 0.0, AttackMode::Curve, &mut w, "op");

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
        let eg = direct_eg(&op, false, 0.0, AttackMode::Bias, &mut warnings, "op");
        assert_eq!(eg.stage_count, 2);
        assert_eq!(eg.stages[0].level, 0);
        assert!(warnings.iter().any(|w| w.contains("ar=0")));
    }

    fn make_voice(algorithm: u8, op_ar: [u8; 4]) -> OpqVoice {
        let mut voice = OpqVoice { algorithm, feedback: 3, ..OpqVoice::default() };
        for i in 0..4 {
            voice.operators[i] = OpqOperator {
                ar: op_ar[i], d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 20 + i as u8, mul: 1, detune: 32, ksr: 1,
                am_enable: false,
            };
        }
        voice
    }

    #[test]
    fn voice_to_op505_patch_copies_non_eg_fields_from_x6() {
        let voice = make_voice(4, [20, 20, 20, 20]);
        let (patch, warnings) = voice_to_op505_patch(&voice, PsrConvOptions::default(), AttackMode::Bias);
        let x6 = voice.to_ym38x6_patch_opts(PsrConvOptions::default());

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
        let (patch, _) = voice_to_op505_patch(&voice, PsrConvOptions::default(), AttackMode::Bias);
        let default_channel = Op505ChannelParams::default();
        assert_eq!(patch.channel.pitch_fg, default_channel.pitch_fg);
        assert_eq!(patch.channel.cutoff_fg, default_channel.cutoff_fg);
        assert_eq!(patch.channel.gain_fg, default_channel.gain_fg);
    }

    #[test]
    fn preset_files_chunk_into_banks_of_128() {
        let voices: Vec<NamedVoice> = (0..130)
            .map(|i| NamedVoice { name: format!("V{i}"), voice: make_voice(0, [20, 20, 20, 20]) })
            .collect();
        let (files, warnings) =
            voices_to_op505_preset_files_opts(1, &voices, PsrConvOptions::default(), AttackMode::None);
        assert!(warnings.is_empty());
        assert_eq!(files.len(), 2);
        match &files[0] {
            Op505PresetFile::Presets { bank, presets } => {
                assert_eq!(*bank, 1);
                assert_eq!(presets.len(), 128);
                assert_eq!(presets[0].program, 0);
                assert_eq!(presets[127].program, 127);
            }
            _ => panic!("expected Presets"),
        }
        match &files[1] {
            Op505PresetFile::Presets { bank, presets } => {
                assert_eq!(*bank, 2);
                assert_eq!(presets.len(), 2);
            }
            _ => panic!("expected Presets"),
        }
    }
}
