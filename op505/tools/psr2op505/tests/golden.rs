//! psr2op505のゴールデンテスト（デフォーク前の現行実装で採取）。
//! 設計意図はop505/tools/opz2op505/tests/golden.rsのdocコメント参照（同じパターンを踏襲）。
//!
//! ゴールデン更新: `$env:UPDATE_GOLDEN=1; cargo test -p psr2op505; Remove-Item Env:\UPDATE_GOLDEN`

use std::path::{Path, PathBuf};

use op505_core::{Op505ChannelParams, Op505PresetEntry, Op505PresetFile};
use op505_tools::golden::{assert_golden, Fingerprint};
use psr2op505::conv::{self, AttackMode};
use psr2x6::conv::{OpqOperator, OpqVoice, PsrConvOptions};

const GOLDEN_VERSION: u32 = 1;

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, ksr: u8) -> OpqOperator {
    OpqOperator { ar, d1r, d2r, rr, d1l, ksr, mul: 1, detune: 32, tl: 20, am_enable: false }
}

/// [`conv::direct_eg`]をレジスタ全域・`AttackMode`3種・キャリア/サステイン条件で掃引する。
#[test]
fn eg_sweep_golden() {
    let ar_vals = [0u8, 1, 5, 10, 20, 31];
    let rr_vals = [0u8, 4, 8, 15];
    let d1l_vals = [0u8, 5, 10, 15];
    let ksr_vals = [0u8, 1, 2, 3];
    let attack_modes = [AttackMode::Bias, AttackMode::None, AttackMode::Curve];
    let carrier_combos: [(bool, f32); 4] = [(false, 0.0), (true, 0.0), (true, 0.5), (true, 1.0)];

    let mut fp = Fingerprint::new();
    for &ar in &ar_vals {
        for &d1r in &ar_vals {
            for &d2r in &ar_vals {
                for &rr in &rr_vals {
                    for &d1l in &d1l_vals {
                        for &ksr in &ksr_vals {
                            for &attack in &attack_modes {
                                for &(is_carrier, carrier_sustain) in &carrier_combos {
                                    let op = make_op(ar, d1r, d2r, rr, d1l, ksr);
                                    let mut warnings = Vec::new();
                                    let eg = conv::direct_eg(
                                        &op,
                                        is_carrier,
                                        carrier_sustain,
                                        attack,
                                        &mut warnings,
                                        "op",
                                    );
                                    fp.push(&eg);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_golden(&golden_path("eg_sweep.fnv"), &fp.finish(GOLDEN_VERSION));
}

fn baseline_op() -> OpqOperator {
    OpqOperator { ar: 20, d1r: 15, d2r: 10, rr: 8, d1l: 10, mul: 1, detune: 32, ksr: 1, tl: 20, am_enable: false }
}

fn baseline_voice(algorithm: u8) -> OpqVoice {
    let mut v = OpqVoice { algorithm, feedback: 3, ..OpqVoice::default() };
    for i in 0..4 {
        v.operators[i] = baseline_op();
    }
    v
}

/// [`conv::voice_to_op505_patch`]の非EGフィールド写像を全アルゴリズム×全PsrConvOptions
/// パターン×各フィールドの one-at-a-time 掃引でカバーする。
#[test]
fn voice_sweep_golden() {
    let opts_variants = [
        PsrConvOptions::default(),
        PsrConvOptions { mod_tl_cap: Some(180), ..PsrConvOptions::default() },
        PsrConvOptions { carrier_sustain: 0.5, ..PsrConvOptions::default() },
        PsrConvOptions { filter_cutoff: Some(200), ..PsrConvOptions::default() },
    ];

    let mut fp = Fingerprint::new();
    for opts in opts_variants {
        for algorithm in 0u8..=7 {
            for tl in [0u8, 32, 64, 96, 127] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.operators {
                    op.tl = tl;
                }
                push_patch(&mut fp, &v, opts);
            }
            for mul in 0u8..=15 {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.operators {
                    op.mul = mul;
                }
                push_patch(&mut fp, &v, opts);
            }
            for detune in [0u8, 16, 32, 48, 63] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.operators {
                    op.detune = detune;
                }
                push_patch(&mut fp, &v, opts);
            }
            for ksr in 0u8..=3 {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.operators {
                    op.ksr = ksr;
                }
                push_patch(&mut fp, &v, opts);
            }
            for am_enable in [false, true] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.operators {
                    op.am_enable = am_enable;
                }
                push_patch(&mut fp, &v, opts);
            }
            for feedback in 0u8..=7 {
                let mut v = baseline_voice(algorithm);
                v.feedback = feedback;
                push_patch(&mut fp, &v, opts);
            }
        }
    }
    assert_golden(&golden_path("voice_sweep.fnv"), &fp.finish(GOLDEN_VERSION));
}

fn push_patch(fp: &mut Fingerprint, voice: &OpqVoice, opts: PsrConvOptions) {
    let (patch, warnings) = conv::voice_to_op505_patch(voice, opts, AttackMode::Bias);
    fp.push(&patch);
    fp.push(&warnings);
}

/// 特殊分岐が全部出る代表ボイスを可読JSONとして凍結する。
#[test]
fn representative_voices_golden() {
    let mut voices: Vec<(String, OpqVoice, PsrConvOptions)> = Vec::new();

    let mut v = baseline_voice(0);
    v.operators[0].ar = 0;
    voices.push(("ArZeroFreeze".to_string(), v, PsrConvOptions::default()));

    let mut v = baseline_voice(0);
    v.operators[0].d1r = 0;
    voices.push(("D1rZero".to_string(), v, PsrConvOptions::default()));

    let mut v = baseline_voice(0);
    v.operators[0].d2r = 1;
    v.operators[0].ksr = 0;
    voices.push(("ThirtySecClamp".to_string(), v, PsrConvOptions::default()));

    let mut v = baseline_voice(4);
    for op in &mut v.operators {
        op.ksr = 3;
    }
    voices.push(("KsrMax".to_string(), v, PsrConvOptions::default()));

    let mut v = baseline_voice(0);
    for op in &mut v.operators {
        op.d1l = 0;
    }
    voices.push(("D1lZero".to_string(), v, PsrConvOptions::default()));

    let mut v = baseline_voice(0);
    for op in &mut v.operators {
        op.d1l = 15;
    }
    voices.push(("D1lMax".to_string(), v, PsrConvOptions::default()));

    for alg in [0u8, 4, 7] {
        voices.push((format!("Alg{alg}"), baseline_voice(alg), PsrConvOptions::default()));
    }

    voices.push((
        "ModCapTarget".to_string(),
        baseline_voice(4),
        PsrConvOptions { mod_tl_cap: Some(180), ..PsrConvOptions::default() },
    ));
    voices.push((
        "CarrierSustainMaxTarget".to_string(),
        baseline_voice(4),
        PsrConvOptions { carrier_sustain: 1.0, ..PsrConvOptions::default() },
    ));

    let mut entries: Vec<Op505PresetEntry> = Vec::new();
    for (i, (name, voice, opts)) in voices.iter().enumerate() {
        let (patch, _warnings) = conv::voice_to_op505_patch(voice, *opts, AttackMode::Bias);
        entries.push(Op505PresetEntry { program: i as u8, name: name.clone(), patch });
    }

    let file = Op505PresetFile::Presets { bank: 0, presets: entries };
    let json = file.to_json().expect("serialize .op505");
    assert_golden(&golden_path("voices.op505"), &json);
}

/// [`Op505ChannelParams::default`]のフィルター/質感LFO系フィールドが、Phase 1の書き換え後も
/// x6 default 経由と同じ値になることを明示的に固定する。
#[test]
fn channel_defaults_survive_x6_roundtrip() {
    let voice = baseline_voice(0);
    let (patch, _) = conv::voice_to_op505_patch(&voice, PsrConvOptions::default(), AttackMode::Bias);
    let default_channel = Op505ChannelParams::default();
    assert_eq!(patch.channel.filter_resonance, default_channel.filter_resonance);
    assert_eq!(patch.channel.filter_type, default_channel.filter_type);
    assert_eq!(patch.channel.filter_self_oscillation, default_channel.filter_self_oscillation);
    assert_eq!(patch.channel.texture_lfo, default_channel.texture_lfo);
}
