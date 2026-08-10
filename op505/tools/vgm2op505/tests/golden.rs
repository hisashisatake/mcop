//! vgm2op505のゴールデンテスト（デフォーク前の現行実装で採取）。
//! 設計意図はop505/tools/opz2op505/tests/golden.rsのdocコメント参照（同じパターンを踏襲）。
//!
//! vgm2x6の演奏ロジック（`play::opm_to_smf`/`play::process_opn`/`PatchBank`）を通した
//! SMFバイト列・バンク内容・SSG人工パッチ・重複判定キーの4種を凍結する。
//! SMFにはプログラム番号が乗るため、Phase 2でPatchBankの重複判定キーを
//! `Ym38x6Patch`からソース値ベース（`SourceKey`）へ差し替えたとき、バンク番号ズレが
//! あれば`opm_stream_*`/`opn_stream_*`のハッシュ不一致として即座に検出できる。
//!
//! ゴールデン更新: `$env:UPDATE_GOLDEN=1; cargo test -p vgm2op505; Remove-Item Env:\UPDATE_GOLDEN`

use std::path::{Path, PathBuf};

use mucom2x6::conv::{OpnOperator, OpnVoice};
use op505_core::{Op505PresetEntry, Op505PresetFile};
use op505_tools::golden::{assert_golden, Fingerprint};
use vgm2op505::convert::{AttackMode, Op505Converter};
use vgm2x6::patch::PatchBank;
use vgm2x6::play::{self, OpnInfo, SmfSink};

const GOLDEN_VERSION: u32 = 1;

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

// ---------------------------------------------------------------------------
// 合成VGMストリーム（main.rsの#[cfg(test)]と同型。bin側は外部から参照できないため複製）。
// ---------------------------------------------------------------------------

fn push_opm(data: &mut Vec<u8>, reg: u8, val: u8) {
    data.push(0x54);
    data.push(reg);
    data.push(val);
}
fn push_opn2203(data: &mut Vec<u8>, reg: u8, val: u8) {
    data.push(0x55);
    data.push(reg);
    data.push(val);
}
fn push_wait(data: &mut Vec<u8>, n: u16) {
    data.push(0x61);
    data.extend_from_slice(&n.to_le_bytes());
}

/// ch0のみを使うOPM VGMコマンド列: 音色レジスタ設定→KC/KF→キーオン→wait→キーオフ→End。
fn opm_test_stream() -> Vec<u8> {
    let mut d = Vec::new();
    push_opm(&mut d, 0x20, 0xC7); // RL/FB/CON ch0
    push_opm(&mut d, 0x38, 0x00); // PMS/AMS ch0
    for op in 0..4u8 {
        let i = op * 8;
        push_opm(&mut d, 0x40 + i, 0x11);
        push_opm(&mut d, 0x60 + i, 0x18);
        push_opm(&mut d, 0x80 + i, 0x1F);
        push_opm(&mut d, 0xA0 + i, 0x0A);
        push_opm(&mut d, 0xC0 + i, 0x08);
        push_opm(&mut d, 0xE0 + i, 0x8F);
    }
    push_opm(&mut d, 0x28, 0x4A); // KC ch0
    push_opm(&mut d, 0x30, 0x00); // KF ch0
    push_opm(&mut d, 0x08, 0x78); // keyon ch0 全slot
    push_wait(&mut d, 4410);
    push_opm(&mut d, 0x08, 0x00); // keyoff ch0
    push_wait(&mut d, 100);
    d.push(0x66);
    d
}

/// YM2203(OPN)のch0 FM音声 + SSG(A)トーンを含むコマンド列。
fn opn_test_stream() -> Vec<u8> {
    let mut d = Vec::new();
    for slot in 0..4u8 {
        let i = slot * 4;
        push_opn2203(&mut d, 0x30 + i, 0x11);
        push_opn2203(&mut d, 0x40 + i, 0x18);
        push_opn2203(&mut d, 0x50 + i, 0x1F);
        push_opn2203(&mut d, 0x60 + i, 0x0A);
        push_opn2203(&mut d, 0x70 + i, 0x08);
        push_opn2203(&mut d, 0x80 + i, 0x8F);
    }
    push_opn2203(&mut d, 0xB0, 0x34); // fb/alg ch0
    push_opn2203(&mut d, 0xA4, 0x22); // block/fnum hi ch0
    push_opn2203(&mut d, 0xA0, 0x00); // fnum lo ch0
    push_opn2203(&mut d, 0x28, 0xF0); // keyon ch0 全op

    // SSG(A)トーン + ノイズ + 混合（psg/noise/mix全経路を踏むよう拡張）。
    push_opn2203(&mut d, 0x00, 0x34); // tone A period lo
    push_opn2203(&mut d, 0x01, 0x02); // tone A period hi
    push_opn2203(&mut d, 0x06, 0x0F); // noise period
    push_opn2203(&mut d, 0x07, 0b111_100); // トーンA + ノイズB 有効（トーンB/Cはミュート）
    push_opn2203(&mut d, 0x08, 0x0F); // vol A
    push_opn2203(&mut d, 0x09, 0x0F); // vol B

    push_wait(&mut d, 4410);
    d.push(0x66);
    d
}

fn opn2203_info() -> OpnInfo {
    OpnInfo {
        label: "YM2203(OPN)",
        write_kind: play::OpnWriteKind::Ym2203,
        clock: 4_000_000,
        fm_channels: 3,
        fm_divisor: vgm2x6::opn::FM_DIVISOR_3CH,
        ssg_clock_div: 2,
        has_ssg: true,
    }
}

// ---------------------------------------------------------------------------
// SMFバイト列・バンク内容
// ---------------------------------------------------------------------------

fn attack_modes() -> [(&'static str, AttackMode); 3] {
    [("bias", AttackMode::Bias), ("none", AttackMode::None), ("curve", AttackMode::Curve)]
}

#[test]
fn opm_stream_golden() {
    let data = opm_test_stream();
    for (label, attack) in attack_modes() {
        let mut bank = PatchBank::new(Op505Converter { attack });
        let mut smf = play::opm_to_smf(&data, 0, &mut bank);
        let mut fp = Fingerprint::new();
        fp.push(&smf.to_bytes());
        assert_golden(&golden_path(&format!("opm_stream_{label}.fnv")), &fp.finish(GOLDEN_VERSION));

        if attack == AttackMode::None {
            let entries: Vec<Op505PresetEntry> = bank
                .entries()
                .map(|(program, name, patch)| Op505PresetEntry {
                    program,
                    name: name.to_string(),
                    patch: *patch,
                })
                .collect();
            let file = Op505PresetFile::Presets { bank: 0, presets: entries };
            let json = file.to_json().expect("serialize .op505");
            assert_golden(&golden_path("opm_stream.op505"), &json);
        }
    }
}

#[test]
fn opn_stream_golden() {
    let data = opn_test_stream();
    let info = opn2203_info();
    let total_ch = info.fm_channels + 3;
    for (label, attack) in attack_modes() {
        let mut bank = PatchBank::new(Op505Converter { attack });
        let mut sink = SmfSink::new(total_ch);
        play::process_opn(&data, 0, info, &mut bank, &mut sink, 100, 100, None);
        let mut fp = Fingerprint::new();
        fp.push(&sink.smf.to_bytes());
        assert_golden(&golden_path(&format!("opn_stream_{label}.fnv")), &fp.finish(GOLDEN_VERSION));

        if attack == AttackMode::None {
            let entries: Vec<Op505PresetEntry> = bank
                .entries()
                .map(|(program, name, patch)| Op505PresetEntry {
                    program,
                    name: name.to_string(),
                    patch: *patch,
                })
                .collect();
            let file = Op505PresetFile::Presets { bank: 0, presets: entries };
            let json = file.to_json().expect("serialize .op505");
            assert_golden(&golden_path("opn_stream.op505"), &json);
        }
    }
}

// ---------------------------------------------------------------------------
// SSG人工パッチ（psg/noise/mix）の変換結果 = Phase 2ネイティブ化の移行ブリッジ。
// ---------------------------------------------------------------------------

/// Adapter変換結果とその警告文言をまとめてJSON化する。ネイティブ化後（Phase 2）は
/// この警告自体が消える（SSG人工パッチについての無意味な警告なので、意図的な差分。
/// spec-fm.md「デフォーク」節の「意図的な差分表」参照）が、ネイティブ実装が焼き込むべき
/// `Op505Patch`の値そのもの（`patch`フィールド）は移行のブリッジとして必須。
#[derive(serde::Serialize)]
struct SsgBridge {
    patch: op505_core::Op505Patch,
    warnings: Vec<String>,
}

#[test]
fn ssg_psg_golden() {
    let (patch, warnings) = op505_core::adapter::convert_patch(&vgm2x6::ssg::psg_patch());
    let json = serde_json::to_string_pretty(&SsgBridge { patch, warnings }).expect("serialize");
    assert_golden(&golden_path("ssg_psg.json"), &json);
}

#[test]
fn ssg_noise_np00_golden() {
    let (patch, warnings) = op505_core::adapter::convert_patch(&vgm2x6::ssg::noise_patch(0));
    let json = serde_json::to_string_pretty(&SsgBridge { patch, warnings }).expect("serialize");
    assert_golden(&golden_path("ssg_noise_np00.json"), &json);
}

#[test]
fn ssg_mix_np00_golden() {
    let (patch, warnings) = op505_core::adapter::convert_patch(&vgm2x6::ssg::mix_patch(0));
    let json = serde_json::to_string_pretty(&SsgBridge { patch, warnings }).expect("serialize");
    assert_golden(&golden_path("ssg_mix_np00.json"), &json);
}

/// [`vgm2x6::ssg::noise_patch`]/[`vgm2x6::ssg::mix_patch`]はnpが`waveform`1フィールドにしか
/// 効かない（F7）。np別ゴールデンを62本持つ代わりに、この不変性をテストで保証する。
#[test]
fn ssg_np_only_affects_waveform() {
    for np in 0u8..=31 {
        let base = vgm2x6::ssg::noise_patch(0);
        let varied = vgm2x6::ssg::noise_patch(np);
        for i in 0..4 {
            let mut b = base.operators[i];
            let mut v = varied.operators[i];
            b.waveform = 0;
            v.waveform = 0;
            assert_eq!(b, v, "noise_patch np={np} op{i} differs beyond waveform");
        }
        assert_eq!(varied.operators[0].waveform, 32 + np.min(31));

        let base_mix = vgm2x6::ssg::mix_patch(0);
        let varied_mix = vgm2x6::ssg::mix_patch(np);
        for i in 0..4 {
            let mut b = base_mix.operators[i];
            let mut v = varied_mix.operators[i];
            b.waveform = 0;
            v.waveform = 0;
            assert_eq!(b, v, "mix_patch np={np} op{i} differs beyond waveform");
        }
        assert_eq!(varied_mix.operators[2].waveform, 32 + np.min(31));
    }
}

// ---------------------------------------------------------------------------
// PatchBankの重複判定キー（現行実装=Ym38x6Patchキー）の挙動を凍結する。
// ---------------------------------------------------------------------------

fn make_opn_voice(algorithm: u8, feedback: u8, dt1: u8) -> OpnVoice {
    let op = OpnOperator { ar: 20, d1r: 15, d2r: 10, rr: 8, d1l: 10, mul: 1, dt1, ks: 1, tl: 20, am_enable: false };
    OpnVoice { operators: [op, op, op, op], algorithm, feedback }
}

/// C0-3判定ゲートで確認した具体的な衝突（`mucom2x6::conv::opn_dt1_to_x6`のTABLE、
/// dt1=0/4がどちらも128=無デチューンへ収束する冗長エンコード）。現行のx6_keyでは
/// この2ボイスは**同一エントリー**へ収束する。Phase 2で重複判定キーをソース値
/// （`SourceKey::Opn(OpnVoice)`、生の`dt1`フィールドで比較）へ置き換える場合は
/// この収束を維持できない（バンク番号がズレる）ため、案A（生ソース値キー）を
/// 却下し案B（x6マッピングと同じ結果になるバイト列キーをym38x6型なしで再現）を
/// 採用する根拠になったテスト。将来Phase 2でこのテストが指す挙動をそのまま
/// 複製するのが正しい移行（[`Fingerprint`]の`dedup_grid.fnv`で網羅的に固定する）。
#[test]
fn dt1_zero_and_four_collide_under_x6_key() {
    let mut bank = PatchBank::new(Op505Converter { attack: AttackMode::None });
    let idx0 = bank.find_or_insert_opn(&make_opn_voice(4, 3, 0), "dt1_0");
    let idx4 = bank.find_or_insert_opn(&make_opn_voice(4, 3, 4), "dt1_4");
    assert_eq!(idx0, idx4, "dt1=0/4は現行x6_keyでは同一エントリーに収束するはず");
    assert_eq!(bank.len(), 1);
}

/// OpnVoiceの広いグリッド + SSG固定パッチ（psg/noise×3/mix×3）を交互に登録し、
/// 返却インデックス列をハッシュ化する。これがPatchBankの重複判定セマンティクスそのもの。
#[test]
fn dedup_grid_golden() {
    let mut bank = PatchBank::new(Op505Converter { attack: AttackMode::None });
    let mut fp = Fingerprint::new();

    let ar_vals = [0u8, 1, 10, 20, 31];
    let dt1_vals = 0u8..=7;
    let algo_vals = [0u8, 4, 7];

    for algorithm in algo_vals {
        for feedback in [0u8, 3, 7] {
            for &ar in &ar_vals {
                for dt1 in dt1_vals.clone() {
                    let op = OpnOperator {
                        ar, d1r: 15, d2r: 10, rr: 8, d1l: 10, mul: 1, dt1, ks: 1, tl: 20, am_enable: false,
                    };
                    let voice = OpnVoice { operators: [op, op, op, op], algorithm, feedback };
                    let idx = bank.find_or_insert_opn(&voice, "grid");
                    fp.push(&idx);
                }
            }
            // SSG固定パッチを合間に挟み、OPNボイスの共有プールと衝突しないことも一緒に固定する。
            let psg_idx = bank.find_or_insert_fixed(vgm2x6::ssg::psg_patch(), "psg");
            fp.push(&psg_idx);
            for np in [0u8, 5, 31] {
                let noise_idx = bank.find_or_insert_fixed(vgm2x6::ssg::noise_patch(np), &format!("noise{np}"));
                fp.push(&noise_idx);
                let mix_idx = bank.find_or_insert_fixed(vgm2x6::ssg::mix_patch(np), &format!("mix{np}"));
                fp.push(&mix_idx);
            }
        }
    }
    fp.push(&bank.len());
    assert_golden(&golden_path("dedup_grid.fnv"), &fp.finish(GOLDEN_VERSION));
}
