//! op505-vstフェーズ2のREAPER実機確認（`op505/vst/private/phase2_verify.mid`）向けテスト音色を
//! `.op505`として書き出す使い捨てツール（`op505_probe.rs`と同じ「使い捨て診断用」の位置づけ）。
//!
//! bank=4に3音色を書き出す：
//! - program=0「Test Lead」: Algorithm=4(FM)ベース、Pitch FGループ(ビブラート)・中庸のFilter
//!   Cutoffを持つ持続音。フェーズ2確認用MIDIファイルの大半（01〜15区間）で使う主役パッチ。
//! - program=1「Test Pluck」: Algorithm=7(全並列)の減衰が速いプラック音。Program Change
//!   （16区間）で切り替えたときに明確に音色が変わることを確認するための対比パッチ。
//! - program=2「Test Vibrato」: 単一サイン波・フィードバック無し・大きめのPitch FG Depth(220)。
//!   Test LeadはFM+フィードバックで音色が複雑なためピッチの揺れが埋もれやすく、CC1/76/77/78の
//!   効果を最大限聴き取りやすくするための検証専用パッチ（01〜04区間用に差し替え可能）。
//!
//! 実行: cargo run -p op505-core --example phase2_test_patches
//! 出力先: `op505_presets_dir()`（`%APPDATA%\op505\presets`優先）へ`phase2_test_patches.op505`。

use op505_core::{
    op505_presets_dir, Op505BipolarFg, Op505ChannelParams, Op505OperatorParams, Op505Patch, Op505PresetEntry,
    Op505PresetFile,
};
use sound_core::{seconds_to_time, TextureLfo, TimeEgParams, TimeStage, MAX_STAGES};

const TEST_BANK: u16 = 4;

fn stages(entries: &[(f32, u8, u8)]) -> [TimeStage; MAX_STAGES] {
    let mut out = [TimeStage::default(); MAX_STAGES];
    for (i, &(secs, level, curve)) in entries.iter().enumerate() {
        out[i] = TimeStage { time: seconds_to_time(secs), level, curve };
    }
    out
}

/// 透過的なGain FG（常時フルレベル、ゲートは閉じない。発音終了は各OPのEGのみで決める。
/// `op505-core::default_gain_fg`と同形、private関数のためここでも同じ形を複製する）。
fn transparent_gain_fg() -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[(0.0, 255, 0)]),
        stage_count: 1,
        loop_enabled: 0,
        loop_start: 0,
        // release_point=最終段＝リリース区間が空。note-offで何も起きずゲートは開いたまま。
        release_point: 0,
     ..Default::default()}
}

/// バイポーラFGの「無効化」状態（Depth=0＝振れ幅ゼロ、レベルは全段中央128）。
fn neutral_bipolar_fg() -> Op505BipolarFg {
    Op505BipolarFg { eg: op505_core::neutral_bipolar_eg(), depth: 0 }
}

/// アタック→ディケイ→サステイン(release_pointで静止)→ノートオフでリリースの標準的なオペレーターEG。
fn ad_sustain_release_eg(decay_level: u8, release_secs: f32) -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[
            (0.01, 255, 0),         // Attack
            (0.3, decay_level, 0),  // Decay -> サステインレベル
            (release_secs, 0, 1),   // Release（release_pointの次から辿る、サイン風）
        ]),
        stage_count: 3,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 1, // stage1(Decay到達点)で静止=サステイン
        ..Default::default()
    }
}

/// ノートオンから自然減衰し、ノートオフ前にほぼ無音まで落ちる「プラック」型EG。
/// 段2は無音へ着地させるためのリリース段（OP EGは必ずレベル0で終わる）。
fn pluck_eg(decay_secs: f32) -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[(0.0, 255, 0), (decay_secs, 0, 1), (0.01, 0, 0)]),
        stage_count: 3,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 1,
     ..Default::default()}
}

/// Pitch FG用のディレイ付きビブラートEG（stage0=遅延(level=128＝中央/無変調、CC78の相対補正対象)→
/// stage1(255=上方向いっぱい)/stage2(0=下方向いっぱい)をピンポンループし、ピッチが上下**両方向**へ
/// 振れる本来のビブラートを作る）。CC78テストは「ノートオンからビブラートが始まるまでの時間」
/// として聴こえるようにするため、`build_patch()`のCC78補正条件（第0段level==中央128）に
/// 合わせてstage0を無変調の待ち段にしてある。
/// 段3はノートオフでピッチを中心(level=128＝変調量ゼロ)へ戻すリリース段。
/// 旧定義は`release_start=0`でリリースが段0へ「後退」する形だったが、保持区間とリリース区間を
/// `release_point`1本で分割する現行モデルでは表現できない（そもそも段が両区間に属する状態は
/// グラフの二重描画の原因だった）。ノートオフでビブラートを止めて中心へ戻す方が意味も自然。
fn vibrato_eg(delay_secs: f32, half_period_secs: f32) -> TimeEgParams {
    const NEUTRAL: u8 = sound_core::BIPOLAR_NEUTRAL_RAW;
    TimeEgParams {
        stages: stages(&[
            (delay_secs, NEUTRAL, 0),
            (half_period_secs, 255, 1),
            (half_period_secs, 0, 1),
            (half_period_secs, NEUTRAL, 1),
        ]),
        stage_count: 4,
        loop_enabled: 1,
        loop_start: 1,
        release_point: 2,
     ..Default::default()}
}

fn test_lead() -> Op505Patch {
    let carrier_eg = ad_sustain_release_eg(200, 1.0);
    let op = |tl: u8, mul: u8| Op505OperatorParams {
        tl,
        eg: carrier_eg,
        mul,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: 128,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    };
    let operators = [op(180, 1), op(160, 2), op(140, 1), op(200, 1)];

    let channel = Op505ChannelParams {
        algorithm: 4, // (O1->O2)+(O3->O4)。09区間でNRPN(0,9)=7へ切り替えると全並列になり音色が変わる
        feedback: 50,
        filter_cutoff: 130, // 中庸に下げておく(CC4手動ワウのヘッドルーム確保)
        filter_resonance: 50,
        filter_type: 0, // LP
        filter_self_oscillation: false,
        pitch_fg: Op505BipolarFg { eg: vibrato_eg(0.15, 0.09), depth: 16 }, // 0.15s遅延、~5.5Hz、控えめな深さ(片振幅≈75セント)
        cutoff_fg: neutral_bipolar_fg(),
        gain_fg: transparent_gain_fg(),
        gain_fg_to_master: true,
        gain_fg_to_operators: false,
        texture_lfo: TextureLfo::default(),
    };

    Op505Patch { operators, channel }
}

/// CC1/76/77/78の効果を誰の耳にも一目瞭然にするための単純な検証専用パッチ。Test Leadは
/// Algorithm4のFM+フィードバックで音色自体が複雑なため、ピッチの揺れが音色変化に埋もれやすい。
/// こちらは単一サイン波・フィードバック無し・大きめのPitch FG Depth(183、片振幅183/255*1200
/// ≈862セント≈7.2半音、上下で計約14半音のスイング)にして、ビブラートの深さ/速さの変化を
/// 最大限聴き取りやすくする。
fn test_vibrato() -> Op505Patch {
    let eg = ad_sustain_release_eg(255, 1.0); // 減衰させずサステインをTLそのままにする(揺れに集中)
    let carrier = Op505OperatorParams {
        tl: 220,
        eg,
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0, // サイン波
        op_fine_tune: 128,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    };
    let silent = Op505OperatorParams { tl: 0, ..carrier };
    let operators = [carrier, silent, silent, silent];

    let channel = Op505ChannelParams {
        algorithm: 7, // 全並列、Op1のみ可聴(TL=0の他3opは無音)。FM変調を一切受けないクリーンなサイン
        feedback: 0,
        filter_cutoff: 255, // サイン波は倍音が無いためフィルターは全開のままでよい
        filter_resonance: 0,
        filter_type: 0,
        filter_self_oscillation: false,
        pitch_fg: Op505BipolarFg { eg: vibrato_eg(0.15, 0.09), depth: 183 },
        cutoff_fg: neutral_bipolar_fg(),
        gain_fg: transparent_gain_fg(),
        gain_fg_to_master: true,
        gain_fg_to_operators: false,
        texture_lfo: TextureLfo::default(),
    };

    Op505Patch { operators, channel }
}

fn test_pluck() -> Op505Patch {
    let op = |tl: u8, mul: u8, decay: f32| Op505OperatorParams {
        tl,
        eg: pluck_eg(decay),
        mul,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: 128,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    };
    let operators = [op(200, 1, 0.5), op(150, 2, 0.35), op(100, 3, 0.25), op(80, 4, 0.18)];

    let channel = Op505ChannelParams {
        algorithm: 7, // 全並列。Program Changeの切替が明確な音色差として聴こえるようにする
        feedback: 20,
        filter_cutoff: 200,
        filter_resonance: 30,
        filter_type: 0,
        filter_self_oscillation: false,
        pitch_fg: neutral_bipolar_fg(),
        cutoff_fg: neutral_bipolar_fg(),
        gain_fg: transparent_gain_fg(),
        gain_fg_to_master: true,
        gain_fg_to_operators: false,
        texture_lfo: TextureLfo::default(),
    };

    Op505Patch { operators, channel }
}

fn main() {
    let file = Op505PresetFile::Presets {
        bank: TEST_BANK,
        presets: vec![
            Op505PresetEntry { program: 0, name: "Test Lead".to_string(), patch: test_lead() },
            Op505PresetEntry { program: 1, name: "Test Pluck".to_string(), patch: test_pluck() },
            Op505PresetEntry { program: 2, name: "Test Vibrato".to_string(), patch: test_vibrato() },
        ],
    };

    let json = file.to_json().expect("serialize");
    let round_tripped = Op505PresetFile::from_json(&json).expect("deserialize (round-trip check)");
    assert_eq!(round_tripped, file, "JSON round-trip mismatch");

    let dir = op505_presets_dir();
    std::fs::create_dir_all(&dir).expect("create presets dir");
    let out_path = dir.join("phase2_test_patches.op505");
    std::fs::write(&out_path, &json).expect("write .op505");

    let bank = op505_core::Op505PresetBank::load_from_dir(&dir);
    let lead = bank.get(TEST_BANK, 0).expect("Test Lead should load from disk");
    let pluck = bank.get(TEST_BANK, 1).expect("Test Pluck should load from disk");
    let vibrato = bank.get(TEST_BANK, 2).expect("Test Vibrato should load from disk");
    println!("Wrote {} (bank={TEST_BANK})", out_path.display());
    println!("  program 0: {} (loaded ok, tl[0]={})", lead.name, lead.patch.operators[0].tl);
    println!("  program 1: {} (loaded ok, tl[0]={})", pluck.name, pluck.patch.operators[0].tl);
    println!("  program 2: {} (loaded ok, tl[0]={})", vibrato.name, vibrato.patch.operators[0].tl);
}
