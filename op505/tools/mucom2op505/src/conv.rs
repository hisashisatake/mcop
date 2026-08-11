//! MUCOM88（OPN/YM2608）ボイスレジスタ → OP505パッチへの直接変換ロジック。
//!
//! 実機レート写像・非EGフィールド構築（[crate::map]、mucom2x6からの複製）と、
//! op505-core `adapter::convert_eg_shape`（EGレートスケール→TimeEgParams変換）を
//! 1ツール内で合成することで、中間の`.38x6`ファイルを経由せず直接TimeEgParamsを得る。
//! mucom2x6にはキャリアサステイン等の味付けオプションが存在しないため、
//! opz2op505/psr2op505と異なりis_carrier/carrier_sustainは扱わない。
//! 詳細な設計判断はop505/tools/opz2op505/src/conv.rsのdocコメント参照（同じパターンを踏襲）。

use op505_core::adapter::convert_eg_shape;
use op505_core::{Op505ChannelParams, Op505Patch, Op505PresetEntry, Op505PresetFile};
use sound_core::TimeEgParams;

use crate::map::{self, NamedVoice, OpnOperator, OpnVoice};

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

/// MUCOM88の1オペレーターEG(ar/d1r/d2r/rr/d1l/ks)をTimeEgParamsへ直接変換する。
/// 実機レート写像は[map]の関数を合成再利用し、段割り当てはop505-coreの
/// [convert_eg_shape]を合成再利用する（独自数式は書かない）。
pub fn direct_eg(
    op: &OpnOperator,
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

/// OpnVoice → Op505Patch。非EGフィールドは[map::convert_op]/[map::convert_channel]で
/// 直接構築し、EGだけ[direct_eg]で埋める。MUCOM88にはFG（Pitch/Cutoff/Gain）が
/// 存在しないため、FG3本はOP505ネイティブの既定値を使う。
pub fn voice_to_op505_patch(voice: &OpnVoice, attack_mode: AttackMode) -> (Op505Patch, Vec<String>) {
    let mut warnings = Vec::new();
    let operators = std::array::from_fn(|i| {
        let op = &voice.operators[i];
        let label = format!("op{}", i + 1);
        let eg = direct_eg(op, attack_mode, &mut warnings, &label);
        let mut params = map::convert_op(op);
        params.eg = eg;
        params
    });

    let ch = map::convert_channel(voice);
    let channel = Op505ChannelParams {
        algorithm: ch.algorithm,
        feedback: ch.feedback,
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

    fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, ks: u8) -> OpnOperator {
        OpnOperator { ar, d1r, d2r, rr, d1l, ks, mul: 1, dt1: 0, tl: 0, am_enable: false }
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
