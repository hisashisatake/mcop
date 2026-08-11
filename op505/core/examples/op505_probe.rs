//! op505-coreの試聴プローブ（使い捨て診断用）。
//!
//! - m1: TimeEgプローブ（`sound-core/examples/time_eg_probe.rs`）のe3（静止を挟んだ音量2値
//!   スイッチ）と全く同じ形をGain FGに設定し、FM合成後の質感で成立するか確認する（本命ユースケース）。
//! - m2: モジュレーターのオペレーターEGを多段ループ化し、レート方式では書けない
//!   「変調指数の折れ線ループ」を聴く。
//! - m3: Pitch FGに階段状プラトー（TimeEgプローブのd1と同じ形）を設定し、
//!   アルペジオ的なピッチステップを聴く。
//! - Adapter聴き比べ: 既存`.38x6`（GM2 Bank0 Acoustic Grand Piano、および引数で渡した
//!   `.38x6`ファイル）をAdapterでOP505形式へ変換し、`Ym38x6Engine`と`Op505Engine`で
//!   同じフレーズを鳴らして`orig_*.wav`/`op505_*.wav`として並べて出力する。変換警告はstdoutへ。
//! - `.op505`書き出しサンプル: Adapter変換結果をJSONとして保存する（フォーマット実物確認用）。
//! - `--convert-bank <in.38x6> <out.op505>`: `.38x6`バンクファイルの全プリセットをAdapterで
//!   一括変換し`.op505`として書き出す。gesture-appの`op505_set_program`はAdapterのその場変換を
//!   廃止した（op505/デフォーク計画Phase 3）ため、既存`.38x6`をOP505で鳴らしたい場合は
//!   このコマンドで`op505_presets_dir()`向けの`.op505`を事前生成する。
//!
//! 実行: cargo run -p op505-core --example op505_probe -- <出力ディレクトリ> [<入力.38x6>...]
//! 実行（一括変換）: cargo run -p op505-core --example op505_probe -- --convert-bank <in.38x6> <out.op505>

use std::path::Path;

use op505_core::adapter::convert_patch;
use op505_core::demo::demo_patch;
use op505_core::{Op505Engine, Op505Patch, Op505PresetEntry, Op505PresetFile};
use sound_core::Vco;
use ym38x6_core::{gm2_bank0_patch, PresetEntry, PresetFile, Ym38x6Engine, Ym38x6Patch};

const SAMPLE_RATE: f32 = 44100.0;
const NOTE_FREQ: f32 = 110.0; // A2
const HOLD_SECS: f32 = 3.0;
const RELEASE_TAIL_SECS: f32 = 1.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_else(|| ".".to_string());

    if first == "--convert-bank" {
        let in_path = args.next().expect("--convert-bank <in.38x6> <out.op505>: 入力パス省略");
        let out_path = args.next().expect("--convert-bank <in.38x6> <out.op505>: 出力パス省略");
        convert_bank(&in_path, &out_path);
        return;
    }

    let out_dir = Path::new(&first);
    let extra_patch_paths: Vec<String> = args.collect();

    // m1/m2/m3の定義は`op505_core::demo`へ昇格済み（0=Gain Switch / 1=Modulator Multi-stage / 2=Pitch Steps）。
    write_wav(&out_dir.join("m1_gain_switch.wav"), &render_op505(demo_patch(0).unwrap(), NOTE_FREQ));
    write_wav(&out_dir.join("m2_op_eg_multistage.wav"), &render_op505(demo_patch(1).unwrap(), NOTE_FREQ));
    write_wav(&out_dir.join("m3_pitch_fg_steps.wav"), &render_op505(demo_patch(2).unwrap(), 220.0));
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

/// `--convert-bank`本体。`.38x6`バンクファイルの全プリセットをAdapterで一括変換し、
/// 同じ`bank`・同じvariant(Presets/Programs)・同じ`program`/`name`を保った`.op505`として書き出す
/// （`op505_presets_dir()`へ配置すればgesture-appの`op505_set_program`から引ける）。
fn convert_bank(in_path: &str, out_path: &str) {
    let json = std::fs::read_to_string(in_path).unwrap_or_else(|e| panic!("読み込み失敗: {in_path}: {e}"));
    let file = PresetFile::from_json(&json).unwrap_or_else(|e| panic!("パース失敗（PresetFile形式でない）: {in_path}: {e}"));

    let convert_entries = |entries: Vec<PresetEntry>| -> Vec<Op505PresetEntry> {
        entries
            .into_iter()
            .map(|entry| {
                let (patch, warnings) = convert_patch(&entry.patch);
                if !warnings.is_empty() {
                    println!("[program {} \"{}\"] 変換警告 {} 件:", entry.program, entry.name, warnings.len());
                    for w in &warnings {
                        println!("  - {w}");
                    }
                }
                Op505PresetEntry { program: entry.program, name: entry.name, patch }
            })
            .collect()
    };

    let out_file = match file {
        PresetFile::Presets { bank, presets } => {
            Op505PresetFile::Presets { bank, presets: convert_entries(presets) }
        }
        PresetFile::Programs { bank, programs } => {
            Op505PresetFile::Programs { bank, programs: convert_entries(programs) }
        }
    };

    let out_json = out_file.to_json().expect("serialize .op505");
    std::fs::write(out_path, out_json).unwrap_or_else(|e| panic!("書き込み失敗: {out_path}: {e}"));
    println!("wrote {out_path}");
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect()
}

/// `.38x6`パッチをAdapterで変換し、原音（Ym38x6Engine）とOP505変換後（Op505Engine）を
/// 同じフレーズで並べて出力する。変換警告はラベル付きでstdoutへ表示する。
fn compare_and_write(out_dir: &Path, label: &str, src: Ym38x6Patch) {
    let (op505_patch, warnings) = convert_patch(&src);
    if warnings.is_empty() {
        println!("[{label}] 変換警告なし");
    } else {
        println!("[{label}] 変換警告 {} 件:", warnings.len());
        for w in &warnings {
            println!("  - {w}");
        }
    }

    write_wav(&out_dir.join(format!("orig_{label}.wav")), &render_ym38x6(src, NOTE_FREQ));
    write_wav(&out_dir.join(format!("op505_{label}.wav")), &render_op505(op505_patch, NOTE_FREQ));

    let json = serde_json::to_string_pretty(&op505_patch).expect("serialize .op505");
    std::fs::write(out_dir.join(format!("{label}.op505")), json).expect(".op505書き込み失敗");
}

// ---------------------------------------------------------------------------
// レンダリング + WAV書き出し
// ---------------------------------------------------------------------------

fn render_op505(patch: Op505Patch, freq: f32) -> Vec<f32> {
    let mut engine = Op505Engine::new(SAMPLE_RATE);
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
