//! opm2op505のゴールデンテスト（デフォーク前の現行実装で採取）。
//! 設計意図はop505/tools/opz2op505/tests/golden.rsのdocコメント参照（同じパターンを踏襲）。
//!
//! ゴールデン更新: `$env:UPDATE_GOLDEN=1; cargo test -p opm2op505; Remove-Item Env:\UPDATE_GOLDEN`

use std::path::{Path, PathBuf};

use op505_core::{Op505ChannelParams, Op505PresetEntry, Op505PresetFile};
use op505_tools::golden::{assert_golden, Fingerprint};
use opm2op505::conv::{self, AttackMode};
use opm2x6::parse::{OpmOpReg, OpmVoice, OperatorOrder};

const GOLDEN_VERSION: u32 = 1;

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

fn make_op(ar: u8, d1r: u8, d2r: u8, rr: u8, d1l: u8, ks: u8) -> OpmOpReg {
    OpmOpReg { ar, d1r, d2r, rr, d1l, ks, tl: 20, mul: 1, dt1: 0, dt2: 0, ams_en: false }
}

/// [`conv::direct_eg`]をレジスタ全域・`AttackMode`3種で掃引する
/// （opm2x6にはキャリアサステインが存在しないためキャリア条件の掃引はない）。
#[test]
fn eg_sweep_golden() {
    let ar_vals = [0u8, 1, 5, 10, 20, 31];
    let rr_vals = [0u8, 4, 8, 15];
    let d1l_vals = [0u8, 5, 10, 15];
    let ks_vals = [0u8, 1, 2, 3];
    let attack_modes = [AttackMode::Bias, AttackMode::None, AttackMode::Curve];

    let mut fp = Fingerprint::new();
    for &ar in &ar_vals {
        for &d1r in &ar_vals {
            for &d2r in &ar_vals {
                for &rr in &rr_vals {
                    for &d1l in &d1l_vals {
                        for &ks in &ks_vals {
                            for &attack in &attack_modes {
                                let op = make_op(ar, d1r, d2r, rr, d1l, ks);
                                let mut warnings = Vec::new();
                                let eg = conv::direct_eg(&op, attack, &mut warnings, "op");
                                fp.push(&eg);
                            }
                        }
                    }
                }
            }
        }
    }
    assert_golden(&golden_path("eg_sweep.fnv"), &fp.finish(GOLDEN_VERSION));
}

fn baseline_op(tl: u8) -> OpmOpReg {
    OpmOpReg { ar: 20, d1r: 15, d2r: 10, rr: 8, d1l: 10, tl, ks: 1, mul: 1, dt1: 0, dt2: 0, ams_en: false }
}

fn baseline_voice(con: u8) -> OpmVoice {
    OpmVoice {
        number: 0,
        name: "Base".to_string(),
        lfrq: 30,
        amd: 15,
        pmd: 20,
        lfo_wf: 2,
        fl: 3,
        con,
        ams: 1,
        pms: 2,
        slot: 120,
        m1: baseline_op(20),
        c1: baseline_op(21),
        m2: baseline_op(22),
        c2: baseline_op(23),
    }
}

/// [`conv::voice_to_op505_patch`]の非EGフィールド写像を全アルゴリズム×`OperatorOrder`2種×
/// 各フィールドの one-at-a-time 掃引でカバーする。`OperatorOrder::Register`は
/// `ordered_op_regs`の並び替えが非EGコピー経路と二重適用にならないことの回帰も兼ねる。
#[test]
fn voice_sweep_golden() {
    let orders = [OperatorOrder::Direct, OperatorOrder::Register];

    let mut fp = Fingerprint::new();
    for order in orders {
        for con in 0u8..=7 {
            for tl in [0u8, 32, 64, 96, 127] {
                let mut v = baseline_voice(con);
                for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
                    op.tl = tl;
                }
                push_patch(&mut fp, &v, order);
            }
            for mul in 0u8..=15 {
                let mut v = baseline_voice(con);
                for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
                    op.mul = mul;
                }
                push_patch(&mut fp, &v, order);
            }
            for dt1 in 0u8..=7 {
                let mut v = baseline_voice(con);
                for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
                    op.dt1 = dt1;
                }
                push_patch(&mut fp, &v, order);
            }
            for dt2 in 0u8..=3 {
                let mut v = baseline_voice(con);
                for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
                    op.dt2 = dt2;
                }
                push_patch(&mut fp, &v, order);
            }
            for ks in 0u8..=3 {
                let mut v = baseline_voice(con);
                for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
                    op.ks = ks;
                }
                push_patch(&mut fp, &v, order);
            }
            for ams_en in [false, true] {
                let mut v = baseline_voice(con);
                for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
                    op.ams_en = ams_en;
                }
                push_patch(&mut fp, &v, order);
            }
            for fl in 0u8..=7 {
                let mut v = baseline_voice(con);
                v.fl = fl;
                push_patch(&mut fp, &v, order);
            }
            for lfrq in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(con);
                v.lfrq = lfrq;
                push_patch(&mut fp, &v, order);
            }
            for amd in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(con);
                v.amd = amd;
                push_patch(&mut fp, &v, order);
            }
            for pmd in [0u8, 25, 50, 75, 99] {
                let mut v = baseline_voice(con);
                v.pmd = pmd;
                push_patch(&mut fp, &v, order);
            }
            for lfo_wf in 0u8..=3 {
                let mut v = baseline_voice(con);
                v.lfo_wf = lfo_wf;
                push_patch(&mut fp, &v, order);
            }
            for ams in 0u8..=3 {
                let mut v = baseline_voice(con);
                v.ams = ams;
                push_patch(&mut fp, &v, order);
            }
            for pms in 0u8..=7 {
                let mut v = baseline_voice(con);
                v.pms = pms;
                push_patch(&mut fp, &v, order);
            }
        }
    }
    assert_golden(&golden_path("voice_sweep.fnv"), &fp.finish(GOLDEN_VERSION));
}

fn push_patch(fp: &mut Fingerprint, voice: &OpmVoice, order: OperatorOrder) {
    let (patch, warnings) = conv::voice_to_op505_patch(voice, order, AttackMode::Bias);
    fp.push(&patch);
    fp.push(&warnings);
}

/// 特殊分岐が全部出る代表ボイスを可読JSONとして凍結する。
#[test]
fn representative_voices_golden() {
    let mut voices: Vec<(String, OpmVoice)> = Vec::new();

    let mut v = baseline_voice(0);
    v.m1.ar = 0;
    voices.push(("ArZeroFreeze".to_string(), v));

    let mut v = baseline_voice(0);
    v.m1.d1r = 0;
    voices.push(("D1rZero".to_string(), v));

    let mut v = baseline_voice(0);
    v.m1.d2r = 1;
    v.m1.ks = 0;
    voices.push(("ThirtySecClamp".to_string(), v));

    let mut v = baseline_voice(4);
    for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
        op.ks = 3;
    }
    voices.push(("KsMax".to_string(), v));

    let mut v = baseline_voice(0);
    for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
        op.d1l = 0;
    }
    voices.push(("D1lZero".to_string(), v));

    let mut v = baseline_voice(0);
    for op in [&mut v.m1, &mut v.c1, &mut v.m2, &mut v.c2] {
        op.d1l = 15;
    }
    voices.push(("D1lMax".to_string(), v));

    for con in [0u8, 4, 7] {
        voices.push((format!("Con{con}"), baseline_voice(con)));
    }

    let mut entries: Vec<Op505PresetEntry> = Vec::new();
    for (i, (name, voice)) in voices.iter().enumerate() {
        let (patch, _warnings) = conv::voice_to_op505_patch(voice, OperatorOrder::Direct, AttackMode::Bias);
        entries.push(Op505PresetEntry { program: i as u8, name: name.clone(), patch });
    }
    // Register順の代表ケースも1件追加（Direct/Registerの結線差分が可読JSONで見えるように）。
    let (reg_patch, _) =
        conv::voice_to_op505_patch(&baseline_voice(4), OperatorOrder::Register, AttackMode::Bias);
    entries.push(Op505PresetEntry {
        program: entries.len() as u8,
        name: "Con4Register".to_string(),
        patch: reg_patch,
    });

    let file = Op505PresetFile::Presets { bank: 0, presets: entries };
    let json = file.to_json().expect("serialize .op505");
    assert_golden(&golden_path("voices.op505"), &json);
}

/// [`Op505ChannelParams::default`]のフィルター/質感LFO系フィールドが、Phase 1の書き換え後も
/// x6 default 経由と同じ値になることを明示的に固定する。
#[test]
fn channel_defaults_survive_x6_roundtrip() {
    let voice = baseline_voice(0);
    let (patch, _) = conv::voice_to_op505_patch(&voice, OperatorOrder::Direct, AttackMode::Bias);
    let default_channel = Op505ChannelParams::default();
    assert_eq!(patch.channel.filter_resonance, default_channel.filter_resonance);
    assert_eq!(patch.channel.filter_type, default_channel.filter_type);
    assert_eq!(patch.channel.filter_self_oscillation, default_channel.filter_self_oscillation);
    assert_eq!(patch.channel.texture_lfo, default_channel.texture_lfo);
}
