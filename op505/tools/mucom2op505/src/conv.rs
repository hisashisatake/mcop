//! MUCOM88（OPN/YM2608）ボイスレジスタ → OP505パッチへの直接変換ロジック。
//!
//! mucom2x6（MUCOM88→.38x6変換）のEGヘルパー（実機レート→38x6レート写像）と、
//! op505-core adapter.rs の`convert_eg_shape`（38x6レート→TimeEgParams変換）を
//! 1ツール内で合成することで、中間の`.38x6`ファイルを経由せず直接TimeEgParamsを得る。
//! mucom2x6にはキャリアサステイン等の味付けオプションが存在しないため、
//! opz2op505/psr2op505と異なりis_carrier/carrier_sustainは扱わない。
//! 詳細な設計判断はop505/tools/opz2op505/src/conv.rsのdocコメント参照（同じパターンを踏襲）。

use op505_core::adapter::convert_eg_shape;
use op505_core::{Op505ChannelParams, Op505OperatorParams, Op505Patch, Op505PresetEntry, Op505PresetFile};
use mucom2x6::conv::{self, NamedVoice, OpnOperator, OpnVoice};
use sound_core::TimeEgParams;

/// アタック立ち上がりの表現方法。詳細はopz2op505::conv::AttackMode参照
/// （同じ問題・同じ選択肢。既定は`None`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackMode {
    /// mucom2x6と同じATTACK_ONSET_BIAS補正を適用（旧2段変換とビット一致、回帰テストの基準・比較用）。
    Bias,
    /// バイアスなし（既定）。
    None,
    /// バイアスなし + stage0のみcurve=1（レイズドコサイン、A/B用）。
    Curve,
}

/// MUCOM88の1オペレーターEG(ar/d1r/d2r/rr/d1l/ks)をTimeEgParamsへ直接変換する。
/// 実機レート→38x6レート写像はmucom2x6の関数（[conv::opn_ar_to_x6]等）を合成再利用し、
/// 段割り当てはop505-coreの[convert_eg_shape]を合成再利用する（独自数式は書かない）。
pub fn direct_eg(
    op: &OpnOperator,
    attack_mode: AttackMode,
    warnings: &mut Vec<String>,
    label: &str,
) -> TimeEgParams {
    let x6_ar = match attack_mode {
        AttackMode::Bias => conv::opn_ar_to_x6(op.ar, op.ks),
        AttackMode::None | AttackMode::Curve => conv::opn_rate_to_x6(op.ar, op.ks),
    };
    let x6_d1r = conv::opn_rate_to_x6(op.d1r, op.ks);
    let x6_d2r = conv::opn_rate_to_x6(op.d2r, op.ks);
    let x6_d1l = conv::sl_opn_to_x6(op.d1l);
    let x6_rr = conv::opn_rr_to_x6(op.rr, op.ks);

    let mut eg = convert_eg_shape(x6_ar, x6_d1r, x6_d1l, x6_d2r, x6_rr, 0, 0, 0, warnings, label);
    if attack_mode == AttackMode::Curve {
        eg.stages[0].curve = 1;
    }
    eg
}

/// OpnVoice → Op505Patch。非EGパラメータは`mucom2x6::conv::to_ym38x6_patch`で一度.38x6化して
/// フィールドコピーし（EG以外は完全一致のフィールド構成のため）、EGだけ[direct_eg]で置き換える。
/// MUCOM88にはFG（Pitch/Cutoff/Gain）が存在しないため、FG3本はOP505ネイティブの既定値を使う。
pub fn voice_to_op505_patch(voice: &OpnVoice, attack_mode: AttackMode) -> (Op505Patch, Vec<String>) {
    let x6 = (*voice).to_ym38x6_patch();

    let mut warnings = Vec::new();
    let operators = std::array::from_fn(|i| {
        let op = &voice.operators[i];
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

/// 名前付きボイス列 → `.op505` プリセットファイル群。
/// mucom2x6::conv::voices_to_preset_filesと同じスロット割り当て規則
/// （slot 0-127はbank、128-255はbank+1）。
/// 戻り値の第2要素は音色名ごとの変換警告（空でない音色のみ）。
pub fn voices_to_op505_preset_files(
    start_bank: u16,
    voices: &[NamedVoice],
    attack_mode: AttackMode,
) -> (Vec<Op505PresetFile>, Vec<(String, Vec<String>)>) {
    let mut all_warnings = Vec::new();
    let mut bank0: Vec<Op505PresetEntry> = Vec::new();
    let mut bank1: Vec<Op505PresetEntry> = Vec::new();
    for nv in voices {
        let (patch, warnings) = voice_to_op505_patch(&nv.voice, attack_mode);
        if !warnings.is_empty() {
            all_warnings.push((nv.name.clone(), warnings));
        }
        let entry = Op505PresetEntry { program: (nv.slot % 128) as u8, name: nv.name.clone(), patch };
        if nv.slot < 128 {
            bank0.push(entry);
        } else {
            bank1.push(entry);
        }
    }
    let mut files = Vec::new();
    if !bank0.is_empty() {
        files.push(Op505PresetFile::Presets { bank: start_bank, presets: bank0 });
    }
    if !bank1.is_empty() {
        files.push(Op505PresetFile::Presets { bank: start_bank + 1, presets: bank1 });
    }
    (files, all_warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ym38x6_core::OperatorParams;

    fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, ks: u8) -> OpnOperator {
        OpnOperator { ar, d1r, d2r, rr, d1l, ks, mul: 1, dt1: 0, tl: 0, am_enable: false }
    }

    fn to_x6_operator_params(op: &OpnOperator) -> OperatorParams {
        OperatorParams {
            ar: conv::opn_ar_to_x6(op.ar, op.ks),
            d1r: conv::opn_rate_to_x6(op.d1r, op.ks),
            d2r: conv::opn_rate_to_x6(op.d2r, op.ks),
            d1l: conv::sl_opn_to_x6(op.d1l),
            rr: conv::opn_rr_to_x6(op.rr, op.ks),
            ..OperatorParams::default()
        }
    }

    /// 核となる回帰テスト: `AttackMode::Bias`（旧2段変換相当）の直接変換は、
    /// 「mucom2x6のx6レート写像 → op505-core adapter::convert_operator_eg」という
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

    fn make_voice(algorithm: u8, op_ar: [u8; 4]) -> OpnVoice {
        let mut voice = OpnVoice { algorithm, feedback: 3, ..OpnVoice::default() };
        for i in 0..4 {
            voice.operators[i] = OpnOperator {
                ar: op_ar[i], d1r: 15, d2r: 10, rr: 8, d1l: 10, tl: 20 + i as u8, mul: 1, dt1: 0, ks: 1,
                am_enable: false,
            };
        }
        voice
    }

    #[test]
    fn voice_to_op505_patch_copies_non_eg_fields_from_x6() {
        let voice = make_voice(4, [20, 20, 20, 20]);
        let (patch, warnings) = voice_to_op505_patch(&voice, AttackMode::Bias);
        let x6 = voice.to_ym38x6_patch();

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
        let (patch, _) = voice_to_op505_patch(&voice, AttackMode::Bias);
        let default_channel = Op505ChannelParams::default();
        assert_eq!(patch.channel.pitch_fg, default_channel.pitch_fg);
        assert_eq!(patch.channel.cutoff_fg, default_channel.cutoff_fg);
        assert_eq!(patch.channel.gain_fg, default_channel.gain_fg);
    }

    #[test]
    fn preset_files_split_by_slot_128() {
        let voices: Vec<NamedVoice> = (0..130u16)
            .map(|slot| NamedVoice { slot, name: format!("V{slot}"), voice: make_voice(0, [20, 20, 20, 20]) })
            .collect();
        let (files, warnings) = voices_to_op505_preset_files(1, &voices, AttackMode::None);
        assert!(warnings.is_empty());
        assert_eq!(files.len(), 2);
        match &files[0] {
            Op505PresetFile::Presets { bank, presets } => {
                assert_eq!(*bank, 1);
                assert_eq!(presets.len(), 128);
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
