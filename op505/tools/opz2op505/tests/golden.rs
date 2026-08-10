//! opz2op505のゴールデンテスト（デフォーク前の現行実装で採取）。
//!
//! 目的: Phase 1（op505/tools/opz2op505をym38x6-core/opz2x6依存から複製実装へ書き換える作業）で
//! 出力が1ビットも変わらないことを保証する。既存の「旧2段変換とビット一致」テスト
//! （`src/conv.rs`の`direct_eg_bias_matches_two_stage_adapter_path`）とは別軸で、
//! 「現在の実装が生成する値そのもの」をファイルへ凍結する。
//!
//! 二層構成（詳細はspec-fm.md「デフォーク」節）:
//! - `eg_sweep.fnv` / `voice_sweep.fnv`: 大量ケースの掃引を1個のハッシュへ凍結（網羅性）
//! - `voices.op505`: 特殊分岐が出る代表ケースの可読JSON（デバッグ可能性）
//!
//! ゴールデン更新: `$env:UPDATE_GOLDEN=1; cargo test -p opz2op505; Remove-Item Env:\UPDATE_GOLDEN`

use std::path::{Path, PathBuf};

use op505_core::{Op505ChannelParams, Op505PresetEntry, Op505PresetFile};
use op505_tools::golden::{assert_golden, Fingerprint};
use opz2op505::conv::{self, AttackMode};
use opz2op505::map::ConvOptions;
use opz2op505::parse::{OpzOpData, OpzVoice};

const GOLDEN_VERSION: u32 = 1;

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, rs: u8, egt: u8) -> OpzOpData {
    OpzOpData { ar, d1r, d2r, rr, d1l, out: 99, rs, egt, det: 3, ..Default::default() }
}

/// [`conv::direct_eg`]（EG変換の中核）をレジスタ全域・`AttackMode`3種・
/// キャリア/サステイン条件で掃引し、1個のハッシュへ凍結する。
#[test]
fn eg_sweep_golden() {
    let ar_vals = [0u8, 1, 5, 10, 20, 31];
    let rr_vals = [0u8, 4, 8, 15];
    let d1l_vals = [0u8, 5, 10, 15];
    let rs_vals = [0u8, 1, 2, 3];
    let egt_vals = [0u8, 1];
    let attack_modes = [AttackMode::Bias, AttackMode::None, AttackMode::Curve];
    let carrier_combos: [(bool, f32); 4] = [(false, 0.0), (true, 0.0), (true, 0.5), (true, 1.0)];

    let mut fp = Fingerprint::new();
    for &ar in &ar_vals {
        for &d1r in &ar_vals {
            for &d2r in &ar_vals {
                for &rr in &rr_vals {
                    for &d1l in &d1l_vals {
                        for &rs in &rs_vals {
                            for &egt in &egt_vals {
                                for &attack in &attack_modes {
                                    for &(is_carrier, carrier_sustain) in &carrier_combos {
                                        let op = make_op(ar, d1r, d2r, rr, d1l, rs, egt);
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
    }
    assert_golden(&golden_path("eg_sweep.fnv"), &fp.finish(GOLDEN_VERSION));
}

fn baseline_op() -> OpzOpData {
    OpzOpData {
        ar: 20, d1r: 15, d2r: 10, rr: 8, d1l: 10, out: 70, rs: 1, egt: 0,
        ame: false, kvs: 3, freq: 1, det: 3, ls: 0, ow: 0, fine: 0, egsft: 0,
    }
}

fn baseline_voice(algorithm: u8) -> OpzVoice {
    let mut v = OpzVoice {
        algorithm,
        feedback: 3,
        lfo_spd: 30,
        lfo_dly: 10,
        pmd: 20,
        amd: 15,
        lfo_sync: false,
        lfo_wf: 0,
        pms: 2,
        ams: 1,
        name: "Base".to_string(),
        ..Default::default()
    };
    for i in 0..4 {
        v.ops[i] = baseline_op();
    }
    v
}

/// [`conv::voice_to_op505_patch`]の非EGフィールド写像（`opz2x6::conv::voice_to_patch_opts`経由）を
/// 全アルゴリズム×全ConvOptionsパターン×各フィールドの one-at-a-time 掃引でカバーする。
/// Phase 1で「x6経由コピー」から「直接構築」へ書き換える対象そのものであり、フィールドの
/// 反映漏れ（コピー忘れ）を検出するのが目的（相互作用ではなく単独反映の正しさを見る）。
#[test]
fn voice_sweep_golden() {
    let opts_variants = [
        ConvOptions::default(),
        ConvOptions { mod_tl_cap: Some(180), ..ConvOptions::default() },
        ConvOptions { fb_override: Some(5), ..ConvOptions::default() },
        ConvOptions { carrier_sustain: 0.5, ksr_override: Some(200), ..ConvOptions::default() },
    ];

    let mut fp = Fingerprint::new();
    for opts in opts_variants {
        for algorithm in 0u8..=7 {
            // per-op フィールドの one-at-a-time 掃引（4オペレーター全てへ同じ値を適用）。
            for out in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.out = out;
                }
                push_patch(&mut fp, &v, opts);
            }
            for det in 0u8..=6 {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.det = det;
                }
                push_patch(&mut fp, &v, opts);
            }
            for kvs in 0u8..=7 {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.kvs = kvs;
                }
                push_patch(&mut fp, &v, opts);
            }
            for ls in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.ls = ls;
                }
                push_patch(&mut fp, &v, opts);
            }
            for ow in 0u8..=7 {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.ow = ow;
                }
                push_patch(&mut fp, &v, opts);
            }
            for fine in [0u8, 5, 10, 15] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.fine = fine;
                }
                push_patch(&mut fp, &v, opts);
            }
            for egsft in 0u8..=3 {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.egsft = egsft;
                }
                push_patch(&mut fp, &v, opts);
            }
            for freq in [0u8, 1, 2, 4, 8, 16, 32, 63] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.freq = freq;
                }
                push_patch(&mut fp, &v, opts);
            }
            for ame in [false, true] {
                let mut v = baseline_voice(algorithm);
                for op in &mut v.ops {
                    op.ame = ame;
                }
                push_patch(&mut fp, &v, opts);
            }

            // voice単位フィールドの one-at-a-time 掃引。
            for feedback in 0u8..=7 {
                let mut v = baseline_voice(algorithm);
                v.feedback = feedback;
                push_patch(&mut fp, &v, opts);
            }
            for lfo_spd in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(algorithm);
                v.lfo_spd = lfo_spd;
                push_patch(&mut fp, &v, opts);
            }
            for lfo_dly in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(algorithm);
                v.lfo_dly = lfo_dly;
                push_patch(&mut fp, &v, opts);
            }
            for pmd in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(algorithm);
                v.pmd = pmd;
                push_patch(&mut fp, &v, opts);
            }
            for amd in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(algorithm);
                v.amd = amd;
                push_patch(&mut fp, &v, opts);
            }
            for pms in 0u8..=7 {
                let mut v = baseline_voice(algorithm);
                v.pms = pms;
                push_patch(&mut fp, &v, opts);
            }
            for ams in 0u8..=3 {
                let mut v = baseline_voice(algorithm);
                v.ams = ams;
                push_patch(&mut fp, &v, opts);
            }
            for lfo_wf in 0u8..=3 {
                let mut v = baseline_voice(algorithm);
                v.lfo_wf = lfo_wf;
                push_patch(&mut fp, &v, opts);
            }
            for lfo_sync in [false, true] {
                let mut v = baseline_voice(algorithm);
                v.lfo_sync = lfo_sync;
                push_patch(&mut fp, &v, opts);
            }
        }
    }
    assert_golden(&golden_path("voice_sweep.fnv"), &fp.finish(GOLDEN_VERSION));
}

fn push_patch(fp: &mut Fingerprint, voice: &OpzVoice, opts: ConvOptions) {
    let (patch, warnings) = conv::voice_to_op505_patch(voice, opts, AttackMode::Bias);
    fp.push(&patch);
    fp.push(&warnings);
}

/// 特殊分岐が全部出る代表ボイス（`ar=0`フリーズ、`d1r=0`、30秒クランプ、`rs`/`kvs`最大、
/// `d1l`両端、alg 0/4/7、モジュレーターTL天井の有無、`carrier_sustain=1.0`）を
/// 可読JSONとして凍結する。フィールド差分をdiffで追えるのが目的（`eg_sweep`/`voice_sweep`は
/// ハッシュ1本なので、壊れたときどこが変わったか分からない）。
#[test]
fn representative_voices_golden() {
    let mut voices: Vec<OpzVoice> = Vec::new();

    // ar=0（フリーズ）。
    let mut v = baseline_voice(0);
    v.name = "ArZeroFreeze".to_string();
    v.ops[0].ar = 0;
    voices.push(v);

    // d1r=0。
    let mut v = baseline_voice(0);
    v.name = "D1rZero".to_string();
    v.ops[0].d1r = 0;
    voices.push(v);

    // 30秒クランプ発生ケース（d2r=1、egt=0で減衰が非常に遅い）。
    let mut v = baseline_voice(0);
    v.name = "ThirtySecClamp".to_string();
    v.ops[0].d2r = 1;
    v.ops[0].rs = 0;
    voices.push(v);

    // rs/kvs最大。
    let mut v = baseline_voice(4);
    v.name = "RsKvsMax".to_string();
    for op in &mut v.ops {
        op.rs = 3;
        op.kvs = 7;
    }
    voices.push(v);

    // d1l=0 / d1l=最大(15)。
    let mut v = baseline_voice(0);
    v.name = "D1lZero".to_string();
    for op in &mut v.ops {
        op.d1l = 0;
    }
    voices.push(v);
    let mut v = baseline_voice(0);
    v.name = "D1lMax".to_string();
    for op in &mut v.ops {
        op.d1l = 15;
    }
    voices.push(v);

    // alg 0/4/7。
    for alg in [0u8, 4, 7] {
        let mut v = baseline_voice(alg);
        v.name = format!("Alg{alg}");
        voices.push(v);
    }

    // モジュレーターTL天井の有無・carrier_sustain=1.0はConvOptions側で表現するため、
    // 専用ボイスをそれぞれの opts で変換する。
    voices.push({
        let mut v = baseline_voice(4);
        v.name = "ModCapTarget".to_string();
        v
    });
    voices.push({
        let mut v = baseline_voice(4);
        v.name = "CarrierSustainMaxTarget".to_string();
        v
    });

    let mut entries: Vec<Op505PresetEntry> = Vec::new();
    for (i, v) in voices.iter().enumerate() {
        let opts = if v.name == "ModCapTarget" {
            ConvOptions { mod_tl_cap: Some(180), ..ConvOptions::default() }
        } else if v.name == "CarrierSustainMaxTarget" {
            ConvOptions { carrier_sustain: 1.0, ..ConvOptions::default() }
        } else {
            ConvOptions::default()
        };
        let (patch, _warnings) = conv::voice_to_op505_patch(v, opts, AttackMode::Bias);
        entries.push(Op505PresetEntry { program: i as u8, name: v.name.clone(), patch });
    }

    let file = Op505PresetFile::Presets { bank: 0, presets: entries };
    let json = file.to_json().expect("serialize .op505");
    assert_golden(&golden_path("voices.op505"), &json);
}

/// [`Op505ChannelParams::default`]のフィルター/質感LFO系フィールドが、Phase 1の書き換え後も
/// x6 default 経由と同じ値になることを明示的に固定する（`voice_sweep`にも掃引で含まれるが、
/// この事実は前提知識として単独テストでも残す）。
#[test]
fn channel_defaults_survive_x6_roundtrip() {
    let voice = baseline_voice(0);
    let (patch, _) = conv::voice_to_op505_patch(&voice, ConvOptions::default(), AttackMode::Bias);
    let default_channel = Op505ChannelParams::default();
    assert_eq!(patch.channel.filter_resonance, default_channel.filter_resonance);
    assert_eq!(patch.channel.filter_type, default_channel.filter_type);
    assert_eq!(patch.channel.filter_self_oscillation, default_channel.filter_self_oscillation);
    assert_eq!(patch.channel.texture_lfo, default_channel.texture_lfo);
}
