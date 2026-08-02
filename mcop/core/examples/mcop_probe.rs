//! mcop-coreの試聴プローブ（使い捨て診断用）。
//!
//! - m1: TimeEgプローブ（`sound-core/examples/time_eg_probe.rs`）のe3（静止を挟んだ音量2値
//!   スイッチ）と全く同じ形をGain FGに設定し、FM合成後の質感で成立するか確認する（本命ユースケース）。
//! - m2: モジュレーターのオペレーターEGを多段ループ化し、レート方式では書けない
//!   「変調指数の折れ線ループ」を聴く。
//! - m3: Pitch FGに階段状プラトー（TimeEgプローブのd1と同じ形）を設定し、
//!   アルペジオ的なピッチステップを聴く。
//! - Adapter聴き比べ: 既存`.38x6`（GM2 Bank0 Acoustic Grand Piano、および引数で渡した
//!   `.38x6`ファイル）をAdapterでmcop形式へ変換し、`Ym38x6Engine`と`McopEngine`で
//!   同じフレーズを鳴らして`orig_*.wav`/`mcop_*.wav`として並べて出力する。変換警告はstdoutへ。
//! - `.mcop`書き出しサンプル: Adapter変換結果をJSONとして保存する（フォーマット実物確認用）。
//!
//! 実行: cargo run -p mcop-core --example mcop_probe -- <出力ディレクトリ> [<入力.38x6>...]

use std::path::Path;

use mcop_core::adapter::convert_patch;
use mcop_core::{McopBipolarFg, McopEngine, McopOperatorParams, McopPatch};
use sound_core::{TimeEgParams, TimeStage, Vco, MAX_STAGES};
use ym38x6_core::{gm2_bank0_patch, PresetFile, Ym38x6Engine, Ym38x6Patch};

const SAMPLE_RATE: f32 = 44100.0;
const NOTE_FREQ: f32 = 110.0; // A2
const HOLD_SECS: f32 = 3.0;
const RELEASE_TAIL_SECS: f32 = 1.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| ".".to_string());
    let out_dir = Path::new(&out_dir);
    let extra_patch_paths: Vec<String> = args.collect();

    write_wav(&out_dir.join("m1_gain_switch.wav"), &render_mcop(m1_patch(), NOTE_FREQ));
    write_wav(&out_dir.join("m2_op_eg_multistage.wav"), &render_mcop(m2_patch(), NOTE_FREQ));
    write_wav(&out_dir.join("m3_pitch_fg_steps.wav"), &render_mcop(m3_patch(), 220.0));
    println!("wrote m1/m2/m3 probe WAVs to {}", out_dir.display());

    // Adapter聴き比べ: GM2 Bank0 Acoustic Grand Piano（組み込み、常時実行）。
    compare_and_write(out_dir, "gm2_piano", gm2_bank0_patch(0).expect("gm2 program 0 should exist"));

    // 引数で渡された追加の.38x6ファイル（PresetFile形式、先頭のpresetを使う）。
    for path in &extra_patch_paths {
        let Ok(json) = std::fs::read_to_string(path) else {
            eprintln!("読み込み失敗: {path}");
            continue;
        };
        let Ok(file) = PresetFile::from_json(&json) else {
            eprintln!("パース失敗（PresetFile形式でない）: {path}");
            continue;
        };
        let entries = match file {
            PresetFile::Presets { presets, .. } => presets,
            PresetFile::Programs { programs, .. } => programs,
        };
        let Some(entry) = entries.into_iter().next() else {
            eprintln!("presetが空: {path}");
            continue;
        };
        let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("patch");
        let label = format!("{stem}_{}", sanitize(&entry.name));
        compare_and_write(out_dir, &label, entry.patch);
    }
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect()
}

/// `.38x6`パッチをAdapterで変換し、原音（Ym38x6Engine）とmcop変換後（McopEngine）を
/// 同じフレーズで並べて出力する。変換警告はラベル付きでstdoutへ表示する。
fn compare_and_write(out_dir: &Path, label: &str, src: Ym38x6Patch) {
    let (mcop_patch, warnings) = convert_patch(&src);
    if warnings.is_empty() {
        println!("[{label}] 変換警告なし");
    } else {
        println!("[{label}] 変換警告 {} 件:", warnings.len());
        for w in &warnings {
            println!("  - {w}");
        }
    }

    write_wav(&out_dir.join(format!("orig_{label}.wav")), &render_ym38x6(src, NOTE_FREQ));
    write_wav(&out_dir.join(format!("mcop_{label}.wav")), &render_mcop(mcop_patch, NOTE_FREQ));

    let json = serde_json::to_string_pretty(&mcop_patch).expect("serialize .mcop");
    std::fs::write(out_dir.join(format!("{label}.mcop")), json).expect(".mcop書き込み失敗");
}

// ---------------------------------------------------------------------------
// m1/m2/m3 パッチ定義
// ---------------------------------------------------------------------------

fn stages(entries: &[(u8, u8, u8)]) -> [TimeStage; MAX_STAGES] {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    for (i, &(time, level, curve)) in entries.iter().enumerate() {
        stages[i] = TimeStage { time, level, curve };
    }
    stages
}

/// 瞬時に満レベルへ到達しそのまま無限サスティンするEG（通常のオペレーターEG用の既定形）。
fn instant_sustain_eg() -> TimeEgParams {
    TimeEgParams {
        stages: stages(&[(0, 255, 0)]),
        stage_count: 1,
        loop_enabled: 0,
        loop_start: 0,
        loop_end: 0,
        release_start: 0,
    }
}

fn plain_operator(tl: u8, mul: u8, dt1: u8) -> McopOperatorParams {
    McopOperatorParams {
        tl,
        eg: instant_sustain_eg(),
        mul,
        dt1,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0,
        op_fine_tune: 128,
        eg_shift: 0,
        level_scale: 0,
        velocity_gain: 255,
    }
}

/// m1: e3（TimeEgプローブ）と全く同じ形の音量2値スイッチをGain FGに設定した、
/// アルゴリズム4（(O1→O2)+(O3→O4)の2系統FMペア）のパッチ。
fn m1_patch() -> McopPatch {
    let mut patch = McopPatch::default();
    patch.operators = [
        plain_operator(190, 2, 128),
        plain_operator(255, 1, 128),
        plain_operator(190, 2, 140), // 2系統目はわずかにデチューンして厚みを出す
        plain_operator(255, 1, 140),
    ];
    patch.channel.algorithm = 4;
    patch.channel.gain_fg = TimeEgParams {
        stages: stages(&[
            (15, 230, 0), // 高い位置へ
            (40, 230, 0), // 静止(高)
            (15, 40, 0),  // 低い位置へ
            (40, 40, 0),  // 静止(低)
            (100, 0, 0),  // リリース
        ]),
        stage_count: 5,
        loop_enabled: 1,
        loop_start: 0,
        loop_end: 3,
        release_start: 4,
    };
    patch
}

/// m2: アルゴリズム0（O1→O2→O3→O4、キャリアはO4）。キャリアを直接変調するO3(index 2)の
/// オペレーターEGを多段ループ化し、変調指数（＝明るさ）が折れ線状に往復する様子を聴く
/// （レート方式のEGでは「速いアタック＋任意形状のループ」が原理的に書けなかった）。
fn m2_patch() -> McopPatch {
    let mut patch = McopPatch::default();
    let silent = plain_operator(0, 1, 128);
    let mut modulator = plain_operator(220, 1, 128);
    modulator.eg = TimeEgParams {
        stages: stages(&[
            (20, 220, 0), // 明るい位置へ
            (50, 220, 0), // 静止(明)
            (20, 80, 0),  // 暗い位置へ
            (50, 80, 0),  // 静止(暗)
        ]),
        stage_count: 4,
        loop_enabled: 1,
        loop_start: 0,
        loop_end: 3,
        release_start: 3,
    };
    let carrier = plain_operator(255, 1, 128);
    patch.operators = [silent, silent, modulator, carrier];
    patch.channel.algorithm = 0;
    patch
}

/// m3: アルゴリズム7（単一キャリアop0のみ有効）。Pitch FGにTimeEgプローブd1と同じ形の
/// 階段状プラトーを設定し、アルペジオ的なピッチステップを聴く。
fn m3_patch() -> McopPatch {
    let mut patch = McopPatch::default();
    patch.operators = [plain_operator(255, 1, 128), plain_operator(0, 1, 128), plain_operator(0, 1, 128), plain_operator(0, 1, 128)];
    patch.channel.algorithm = 7;
    patch.channel.pitch_fg = McopBipolarFg {
        eg: TimeEgParams {
            stages: stages(&[
                (15, 90, 0),  // 第1段への上昇(速い)
                (45, 90, 0),  // 静止(プラトー)
                (15, 170, 0), // 第2段への上昇
                (45, 170, 0), // 静止(プラトー)
                (15, 255, 0), // 第3段への上昇
                (45, 255, 0), // 静止(プラトー)
                (25, 20, 0),  // 速いリセット
                (110, 0, 0),  // リリース
            ]),
            stage_count: 8,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 6,
            release_start: 7,
        },
        depth: 200, // バイポーラ、中心128超で+方向（最大約+675セント）
    };
    patch
}

// ---------------------------------------------------------------------------
// レンダリング + WAV書き出し
// ---------------------------------------------------------------------------

fn render_mcop(patch: McopPatch, freq: f32) -> Vec<f32> {
    let mut engine = McopEngine::new(SAMPLE_RATE);
    engine.set_patch(patch);
    engine.note_on(0, freq, 100);
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = vec![0.0f32; hold_samples + release_samples];
    engine.render(&mut out[..hold_samples], 1);
    engine.note_off(0);
    engine.render(&mut out[hold_samples..], 1);
    out
}

fn render_ym38x6(patch: Ym38x6Patch, freq: f32) -> Vec<f32> {
    let mut engine = Ym38x6Engine::new(SAMPLE_RATE);
    engine.set_patch(patch);
    engine.note_on(0, freq, 100);
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = vec![0.0f32; hold_samples + release_samples];
    engine.render(&mut out[..hold_samples], 1);
    engine.note_off(0);
    engine.render(&mut out[hold_samples..], 1);
    out
}

fn write_wav(path: &Path, samples: &[f32]) {
    let mut bytes = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("WAV書き込み失敗");
}
