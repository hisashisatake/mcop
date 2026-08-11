//! opm2op505 — VOPM形式の.opm音色ファイル（YM2151/OPM）を OP505 の `.op505` プリセットへ
//! **直接**変換する（中間の`.38x6`ファイルを経由しない。パーサ・実機レート写像は自クレート内に
//! 複製済み（fork-on-write）。詳細は`opm2op505::conv`/`opm2op505::map`のdocコメント参照）。
//!
//! 使い方:
//! ```text
//! opm2op505 <input.opm> [output_dir] [--operator-order direct|register] [--bank <N>] [--split]
//!           [--wav] [--attack <bias|none|curve>]
//! ```
//! - `output_dir` 省略時は入力ファイルと同じディレクトリに出力する。
//! - `--bank` の既定は 1（Bank0はGM2準拠のため通常は使わない、opm2x6と同一）。
//! - `--split` を指定すると音色ごとに個別ファイル（Programs形式）を出力する。
//!   省略時は全音色を1ファイル（Presets形式）にまとめる。
//! - `--wav` を指定すると `<output_dir>/wav/<NNN>_<name>.wav` へ試聴用WAVを出力する
//!   （`Op505Engine`で直接レンダリング）。
//! - `--attack <bias|none|curve>` アタック立ち上がりの表現方法（既定 `none`。opz2op505の
//!   聴感A/B判断を踏襲。詳細は`opm2op505::conv::AttackMode`）。

use opm2op505::conv::{self, AttackMode};
use opm2op505::parse::{self, OperatorOrder};

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("opm2op505: {msg}");
            eprintln!(
                "usage: opm2op505 <input.opm> [output_dir] \
                 [--operator-order direct|register] [--bank <N>] [--split] [--wav] [--attack <bias|none|curve>]"
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
    wav: bool,
    attack_mode: AttackMode,
}

fn parse_attack_mode(v: &str) -> Result<AttackMode, String> {
    match v.to_ascii_lowercase().as_str() {
        "bias" => Ok(AttackMode::Bias),
        "none" => Ok(AttackMode::None),
        "curve" => Ok(AttackMode::Curve),
        _ => Err(format!("--attack の値が不正(bias/none/curve): {v}")),
    }
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut op_order = OperatorOrder::Direct;
    let mut bank: u16 = 1;
    let mut split = false;
    let mut wav = false;
    let mut attack_mode = AttackMode::None;
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
            "--wav" => { wav = true; i += 1; }
            "--attack" => {
                let v = args.get(i + 1).ok_or("--attack に値がありません")?;
                attack_mode = parse_attack_mode(v)?;
                i += 2;
            }
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
    Ok(Args { input, output_dir, op_order, bank, split, wav, attack_mode })
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
                 (OP505の音色LFOは三角波固定のため波形差は反映されません)",
                voice.number, voice.name, voice.lfo_wf
            );
        }
    }

    if args.wav {
        render_wavs(&voices, &args.output_dir, args.op_order, args.attack_mode)?;
    }

    let mut all_warnings: Vec<(String, Vec<String>)> = Vec::new();

    if args.split {
        for voice in &voices {
            let (entry, warnings) = conv::voice_to_entry(voice, args.op_order, args.attack_mode);
            if !warnings.is_empty() {
                all_warnings.push((voice.name.clone(), warnings));
            }
            let file = op505_core::Op505PresetFile::Programs {
                bank: args.bank,
                programs: vec![entry],
            };
            let safe = parse::sanitize_filename(&voice.name);
            let basename = if safe.is_empty() {
                format!("{:03}.op505", voice.number)
            } else {
                format!("{:03}_{safe}.op505", voice.number)
            };
            let out_path = args.output_dir.join(&basename);
            let json = file.to_json()
                .map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
            std::fs::write(&out_path, json)
                .map_err(|e| format!("書き込みに失敗: {}: {e}", out_path.display()))?;
            println!("書き出し: {}", out_path.display());
        }
    } else {
        let presets: Vec<_> = voices.iter().map(|v| {
            let (entry, warnings) = conv::voice_to_entry(v, args.op_order, args.attack_mode);
            if !warnings.is_empty() {
                all_warnings.push((v.name.clone(), warnings));
            }
            entry
        }).collect();
        let file = op505_core::Op505PresetFile::Presets {
            bank: args.bank,
            presets,
        };
        let stem = args.input.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out");
        let out_path = args.output_dir.join(format!("{stem}.op505"));
        let json = file.to_json()
            .map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
        std::fs::write(&out_path, json)
            .map_err(|e| format!("書き込みに失敗: {}: {e}", out_path.display()))?;
        println!("書き出し: {} ({} 音色)", out_path.display(), voices.len());
    }

    if !all_warnings.is_empty() {
        println!("変換警告 ({} 音色):", all_warnings.len());
        for (name, warnings) in &all_warnings {
            println!("  [{name}]");
            for w in warnings {
                println!("    - {w}");
            }
        }
    }

    Ok(())
}

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする（試聴用、`Op505Engine`直接使用）。
fn render_wavs(
    voices: &[parse::OpmVoice],
    output_dir: &Path,
    op_order: OperatorOrder,
    attack_mode: AttackMode,
) -> Result<(), String> {
    use op505_core::Op505Engine;
    use sound_core::Vco;
    const SR: f32 = 44_100.0;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| format!("wav ディレクトリ作成に失敗: {e}"))?;

    for (idx, voice) in voices.iter().enumerate() {
        let (patch, _warnings) = conv::voice_to_op505_patch(voice, op_order, attack_mode);
        let mut engine = Op505Engine::new(SR);
        engine.set_patch(patch);
        engine.note_on(0, 261.63, 100);

        let on_samples = (SR * 2.0) as usize;
        let off_samples = (SR * 1.5) as usize;
        let mut buf = vec![0.0f32; on_samples];
        engine.render(&mut buf, 1);
        engine.note_off(0);
        let mut tail = vec![0.0f32; off_samples];
        engine.render(&mut tail, 1);
        buf.extend_from_slice(&tail);

        let safe = parse::sanitize_filename(&voice.name);
        let filename = if safe.is_empty() {
            format!("{idx:03}.wav")
        } else {
            format!("{idx:03}_{safe}.wav")
        };
        let path = wav_dir.join(&filename);
        op505_tools::wav::write_wav_mono16(&path, &buf, SR as u32)
            .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV 書き出し: {} に {} 音色", wav_dir.display(), voices.len());
    Ok(())
}
