//! opm2x6 — VOPM形式の.opm音色ファイル（YM2151/OPM）を ym38x6 の .38x6 プリセットに変換。
//!
//! 使い方:
//! ```text
//! opm2x6 <input.opm> [output_dir] [--operator-order direct|register] [--bank <N>] [--split]
//! ```
//! - `output_dir` 省略時は入力ファイルと同じディレクトリに出力する。
//! - `--bank` の既定は 1（Bank0はGM2準拠のため通常は使わない）。
//! - `--split` を指定すると音色ごとに個別ファイル（Programs形式）を出力する。
//!   省略時は全音色を1ファイル（Presets形式）にまとめる。

mod conv;
mod parse;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use parse::OperatorOrder;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("opm2x6: {msg}");
            eprintln!(
                "usage: opm2x6 <input.opm> [output_dir] \
                 [--operator-order direct|register] [--bank <N>] [--split]"
            );
            ExitCode::FAILURE
        }
    }
}

struct Args {
    input: PathBuf,
    output_dir: PathBuf,
    op_order: OperatorOrder,
    bank: u16,
    split: bool,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut op_order = OperatorOrder::Direct;
    let mut bank: u16 = 1;
    let mut split = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--operator-order" => {
                let v = args.get(i + 1).ok_or("--operator-order に値がありません")?;
                op_order = match v.as_str() {
                    "direct"   => OperatorOrder::Direct,
                    "register" => OperatorOrder::Register,
                    _ => return Err(format!(
                        "--operator-order は direct か register で指定してください: {v}"
                    )),
                };
                i += 2;
            }
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
                i += 2;
            }
            "--split" => { split = true; i += 1; }
            _ => { positional.push(&args[i]); i += 1; }
        }
    }
    if positional.is_empty() {
        return Err("入力 .opm ファイルのパスが必要です".to_string());
    }
    let input = PathBuf::from(positional[0]);
    let output_dir = if positional.len() >= 2 {
        PathBuf::from(positional[1])
    } else {
        input.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    Ok(Args { input, output_dir, op_order, bank, split })
}

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    let text = std::fs::read_to_string(&args.input)
        .map_err(|e| format!("{}: {e}", args.input.display()))?;

    let voices = parse::parse_opm(&text)
        .map_err(|e| format!("パース失敗: {e}"))?;

    if voices.is_empty() {
        eprintln!(
            "warning: {} に @: で始まる音色定義が見つかりませんでした",
            args.input.display()
        );
        return Ok(());
    }

    std::fs::create_dir_all(&args.output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {e}"))?;

    // 変換前の注記出力
    for voice in &voices {
        if voice.slot != 120 {
            eprintln!(
                "note: voice {} ({:?}): SLOT={} \
                 (実機では一部OPが無効のため、該当OPのTLを0に設定します)",
                voice.number, voice.name, voice.slot
            );
        }
        if voice.lfo_wf != 2 {
            eprintln!(
                "note: voice {} ({:?}): LFO WF={} \
                 (38x6の音色LFOは三角波固定のため波形差は反映されません)",
                voice.number, voice.name, voice.lfo_wf
            );
        }
    }

    if args.split {
        // 音色ごとに個別ファイル: "{num:03}_{name}.38x6"（Programs形式）
        for voice in &voices {
            let entry = conv::voice_to_entry(voice, args.op_order);
            let file = ym38x6_core::PresetFile::Programs {
                bank: args.bank,
                programs: vec![entry],
            };
            let safe = parse::sanitize_filename(&voice.name);
            let basename = if safe.is_empty() {
                format!("{:03}.38x6", voice.number)
            } else {
                format!("{:03}_{safe}.38x6", voice.number)
            };
            let out_path = args.output_dir.join(&basename);
            let json = file.to_json()
                .map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
            std::fs::write(&out_path, json)
                .map_err(|e| format!("書き込みに失敗: {}: {e}", out_path.display()))?;
            println!("書き出し: {}", out_path.display());
        }
    } else {
        // 全音色を1ファイルに: "{stem}.38x6"（Presets形式）
        let presets: Vec<_> = voices.iter()
            .map(|v| conv::voice_to_entry(v, args.op_order))
            .collect();
        let file = ym38x6_core::PresetFile::Presets {
            bank: args.bank,
            presets,
        };
        let stem = args.input.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out");
        let out_path = args.output_dir.join(format!("{stem}.38x6"));
        let json = file.to_json()
            .map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
        std::fs::write(&out_path, json)
            .map_err(|e| format!("書き込みに失敗: {}: {e}", out_path.display()))?;
        println!("書き出し: {} ({} 音色)", out_path.display(), voices.len());
    }
    Ok(())
}
