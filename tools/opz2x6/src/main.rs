//! opz2x6 — TX81Z sysex (.syx) を ym38x6 の .38x6 プリセットに変換。
//!
//! 使い方:
//! ```text
//! opz2x6 <input.syx> [output_dir] [--bank <N>] [--split]
//! ```
//! - `output_dir` 省略時は入力ファイルと同じディレクトリに出力する。
//! - `--bank` の既定は 1。
//! - `--split` を指定すると音色ごとに個別ファイル（Programs形式）を出力する。
//!   省略時は全音色を1ファイル（Presets形式）にまとめる。

use opz2x6::conv;
use opz2x6::parse;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("opz2x6: {msg}");
            eprintln!("usage: opz2x6 <input.syx> [output_dir] [--bank <N>] [--split]");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    input: PathBuf,
    output_dir: PathBuf,
    bank: u16,
    split: bool,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut bank: u16 = 1;
    let mut split = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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
        return Err("入力 .syx ファイルのパスが必要です".to_string());
    }
    let input = PathBuf::from(positional[0]);
    let output_dir = if positional.len() >= 2 {
        PathBuf::from(positional[1])
    } else {
        input.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    Ok(Args { input, output_dir, bank, split })
}

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    let data = std::fs::read(&args.input)
        .map_err(|e| format!("{}: {e}", args.input.display()))?;

    let voices = parse::parse_syx(&data)
        .map_err(|e| format!("パース失敗: {e}"))?;

    if voices.is_empty() {
        eprintln!("warning: {} にボイスが見つかりませんでした", args.input.display());
        return Ok(());
    }

    // ACED なし＝波形サインのまま、の警告
    let has_ow = voices.iter().any(|v| v.ops.iter().any(|op| op.ow != 0));
    if !has_ow {
        eprintln!(
            "note: 全ボイスの OW（オペレーター波形）がサイン波(0)です。\
             ACED データが含まれていない .syx では OPZ 固有の非サイン波形は再現されません。\
             TX81Z の「All Data」バルクダンプ（VMEM+ACED同梱）を使用してください。"
        );
    }

    std::fs::create_dir_all(&args.output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {e}"))?;

    if args.split {
        for voice in &voices {
            let entry = conv::voice_to_entry(voice);
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
        let presets: Vec<_> = voices.iter().map(|v| conv::voice_to_entry(v)).collect();
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
