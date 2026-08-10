//! opz2op505 — TX81Z sysex (.syx) を OP505 の `.op505` プリセットへ**直接**変換する
//! （中間の`.38x6`ファイルを経由しない。opz2x6 + op505-core adapter.rs のEG変換ロジックを
//! 1ツール内で合成する。詳細は`opz2op505::conv`のdocコメント参照）。
//!
//! 使い方:
//! ```text
//! opz2op505 <input.syx> [output_dir] [--bank <N>] [--split] [--wav]
//!           [--on <秒>] [--octave <N>] [--note <音階>] [--voice <N>] [--mod-cap <N>] [--fb <N>]
//!           [--attack <bias|none|curve>]
//! ```
//! - `output_dir` 省略時は入力ファイルと同じディレクトリに出力する。
//! - `--bank` の既定は 0。
//! - `--split` を指定すると音色ごとに個別ファイル（Programs形式）を出力する。
//!   省略時は全音色を1ファイル（Presets形式）にまとめる。
//! - `--wav` を指定すると `<output_dir>/wav/<NNN>_<name>.wav` へ試聴用WAVを出力する
//!   （`Op505Engine`で直接レンダリング）。
//! - `--on <秒>` でキーオン時間（既定 2.0 秒）を指定する。
//! - `--octave <N>` でオクターブ（既定 4）を指定する。
//! - `--note <音階>` で音階（C/D/E/F/G/A/B、既定 C）を指定する。
//! - `--voice <N>` を指定すると WAV 出力をその音色番号（0始まり）1件のみに絞る。
//! - `--wav-prefix <str>` で WAV ファイル名の先頭に付ける文字列を指定する。
//! - `--mod-cap <N>` でモジュレーター TL に天井をかける（既定は天井なし、opz2x6と同じ）。
//! - `--fb <N>` でチャンネルフィードバック（0-255）を全音色一律で上書きする。
//! - `--ksr <N>` で全オペレーターの KSR（0-255）を上書きする。
//! - `--sustain <0.0-1.0>` キャリアのサステイン延長（味付け、既定 0.0=実機忠実）。
//! - `--cutoff <0-255>` ローパスフィルターのカットオフ（味付け、既定=全開255）。
//! - `--attack <bias|none|curve>` アタック立ち上がりの表現方法（既定 `none`。聴感A/Bの結果、
//!   `bias`との差が小さいと判断され2026-08-10にユーザー判断で採用）。
//!   `none`=補正なし、`bias`=opz2x6と同じATTACK_ONSET_BIAS補正（旧2段変換とビット一致、比較用）、
//!   `curve`=補正なし+stage0のみレイズドコサイン。詳細は`opz2op505::conv::AttackMode`。

use opz2op505::conv::{self, AttackMode};
use opz2x6::conv::ConvOptions;
use opz2x6::parse;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("opz2op505: {msg}");
            eprintln!(
                "usage: opz2op505 <input.syx> [output_dir] [--bank <N>] [--split] [--wav]"
            );
            eprintln!(
                "        [--on <秒>] [--octave <N>] [--note <C/D/E/F/G/A/B>] [--voice <N>] [--mod-cap <N>] [--fb <N>] [--attack <bias|none|curve>]"
            );
            ExitCode::FAILURE
        }
    }
}

struct WavConfig {
    on_secs: f32,
    off_secs: f32,
    frequency: f32,
}

impl Default for WavConfig {
    fn default() -> Self {
        Self { on_secs: 2.0, off_secs: 1.5, frequency: 261.63 }
    }
}

struct Args {
    input: PathBuf,
    output_dir: PathBuf,
    bank: u16,
    split: bool,
    wav: bool,
    wav_cfg: WavConfig,
    wav_prefix: String,
    voice_filter: Option<usize>,
    opts: ConvOptions,
    attack_mode: AttackMode,
}

fn note_name_to_semitone(note: &str) -> Result<i32, String> {
    match note.to_uppercase().as_str() {
        "C" => Ok(0), "D" => Ok(2), "E" => Ok(4), "F" => Ok(5),
        "G" => Ok(7), "A" => Ok(9), "B" => Ok(11),
        _ => Err(format!("不正な音階: {note}（C/D/E/F/G/A/B で指定してください）")),
    }
}

fn note_to_freq(octave: i32, note: &str) -> Result<f32, String> {
    let semitone = note_name_to_semitone(note)?;
    let midi = (octave + 1) * 12 + semitone;
    Ok(440.0 * 2f32.powf((midi - 69) as f32 / 12.0))
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
    let mut bank: u16 = 0;
    let mut split = false;
    let mut wav = false;
    let mut on_secs: f32 = 2.0;
    let mut octave: i32 = 4;
    let mut note = "C".to_string();
    let mut voice_filter: Option<usize> = None;
    let mut wav_prefix = String::new();
    let mut mod_cap: Option<u8> = None;
    let mut fb_override: Option<u8> = None;
    let mut ksr_override: Option<u8> = None;
    let mut carrier_sustain: f32 = 0.0;
    let mut filter_cutoff: Option<u8> = None;
    let mut pitch_normalize = true;
    let mut attack_mode = AttackMode::None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
                i += 2;
            }
            "--split" => { split = true; i += 1; }
            "--wav"   => { wav = true; i += 1; }
            "--no-pitch-normalize" => { pitch_normalize = false; i += 1; }
            "--on" => {
                let v = args.get(i + 1).ok_or("--on に値がありません")?;
                on_secs = v.parse::<f32>().map_err(|_| format!("--on の値が不正: {v}"))?;
                if on_secs <= 0.0 { return Err(format!("--on は正の値を指定してください: {v}")); }
                i += 2;
            }
            "--octave" => {
                let v = args.get(i + 1).ok_or("--octave に値がありません")?;
                octave = v.parse::<i32>().map_err(|_| format!("--octave の値が不正: {v}"))?;
                i += 2;
            }
            "--note" => {
                let v = args.get(i + 1).ok_or("--note に値がありません")?;
                note_name_to_semitone(v)?;
                note = v.to_string();
                i += 2;
            }
            "--voice" => {
                let v = args.get(i + 1).ok_or("--voice に値がありません")?;
                voice_filter = Some(v.parse::<usize>().map_err(|_| format!("--voice の値が不正: {v}"))?);
                i += 2;
            }
            "--wav-prefix" => {
                let v = args.get(i + 1).ok_or("--wav-prefix に値がありません")?;
                wav_prefix = v.to_string();
                i += 2;
            }
            "--mod-cap" => {
                let v = args.get(i + 1).ok_or("--mod-cap に値がありません")?;
                mod_cap = if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("off") {
                    None
                } else {
                    Some(v.parse::<u8>().map_err(|_| format!("--mod-cap の値が不正(0-255 または none): {v}"))?)
                };
                i += 2;
            }
            "--fb" => {
                let v = args.get(i + 1).ok_or("--fb に値がありません")?;
                fb_override = Some(v.parse::<u8>().map_err(|_| format!("--fb の値が不正(0-255): {v}"))?);
                i += 2;
            }
            "--ksr" => {
                let v = args.get(i + 1).ok_or("--ksr に値がありません")?;
                ksr_override = Some(v.parse::<u8>().map_err(|_| format!("--ksr の値が不正(0-255): {v}"))?);
                i += 2;
            }
            "--sustain" => {
                let v = args.get(i + 1).ok_or("--sustain に値がありません")?;
                carrier_sustain = v.parse::<f32>().map_err(|_| format!("--sustain の値が不正(0.0-1.0): {v}"))?;
                if !(0.0..=1.0).contains(&carrier_sustain) {
                    return Err(format!("--sustain は 0.0〜1.0 で指定してください: {v}"));
                }
                i += 2;
            }
            "--cutoff" => {
                let v = args.get(i + 1).ok_or("--cutoff に値がありません")?;
                filter_cutoff = Some(v.parse::<u8>().map_err(|_| format!("--cutoff の値が不正(0-255): {v}"))?);
                i += 2;
            }
            "--attack" => {
                let v = args.get(i + 1).ok_or("--attack に値がありません")?;
                attack_mode = parse_attack_mode(v)?;
                i += 2;
            }
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
    let frequency = note_to_freq(octave, &note)?;
    Ok(Args {
        input, output_dir, bank, split, wav,
        wav_cfg: WavConfig { on_secs, off_secs: 1.5, frequency },
        wav_prefix,
        voice_filter,
        opts: ConvOptions { mod_tl_cap: mod_cap, fb_override, ksr_override, carrier_sustain, filter_cutoff, pitch_normalize },
        attack_mode,
    })
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

    if args.wav {
        render_wavs(&voices, &args.output_dir, &args.wav_cfg, &args.wav_prefix, args.voice_filter, args.opts, args.attack_mode)?;
    }

    let mut all_warnings: Vec<(String, Vec<String>)> = Vec::new();

    if args.split {
        for voice in &voices {
            let (entry, warnings) = conv::voice_to_entry_opts(voice, args.opts, args.attack_mode);
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
            let (entry, warnings) = conv::voice_to_entry_opts(v, args.opts, args.attack_mode);
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
/// ファイル名は `<prefix><NNN>_<name>.wav`（NNN=音色番号0始まり、prefixは`--wav-prefix`指定時のみ）。
/// `voice_filter` が `Some(N)` のときは音色番号 N の1件のみを出力する。
fn render_wavs(
    voices: &[parse::OpzVoice],
    output_dir: &Path,
    cfg: &WavConfig,
    prefix: &str,
    voice_filter: Option<usize>,
    opts: ConvOptions,
    attack_mode: AttackMode,
) -> Result<(), String> {
    use op505_core::Op505Engine;
    use sound_core::Vco;
    const SR: f32 = 44_100.0;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| format!("wav ディレクトリ作成に失敗: {e}"))?;

    let targets: Vec<(usize, &parse::OpzVoice)> = match voice_filter {
        Some(n) => {
            let v: Vec<_> = voices.iter().enumerate().filter(|(i, _)| *i == n).collect();
            if v.is_empty() {
                return Err(format!("--voice {n} に対応する音色が見つかりません（範囲: 0〜{}）", voices.len().saturating_sub(1)));
            }
            v
        }
        None => voices.iter().enumerate().collect(),
    };

    for (idx, voice) in &targets {
        let (patch, _warnings) = conv::voice_to_op505_patch(voice, opts, attack_mode);
        let mut engine = Op505Engine::new(SR);
        engine.set_patch(patch);
        engine.note_on(0, cfg.frequency, 80);

        let on_samples = (SR * cfg.on_secs) as usize;
        let off_samples = (SR * cfg.off_secs) as usize;
        let mut buf = vec![0.0f32; on_samples];
        engine.render(&mut buf, 1);
        engine.note_off(0);
        let mut tail = vec![0.0f32; off_samples];
        engine.render(&mut tail, 1);
        buf.extend_from_slice(&tail);

        let safe = parse::sanitize_filename(&voice.name);
        let filename = if safe.is_empty() {
            format!("{prefix}{idx:03}.wav")
        } else {
            format!("{prefix}{idx:03}_{safe}.wav")
        };
        let peak = buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        if peak > 1e-4 {
            let norm = 0.5 / peak;
            for s in &mut buf { *s *= norm; }
        }

        let path = wav_dir.join(&filename);
        op505_tools::wav::write_wav_mono16(&path, &buf, SR as u32)
            .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV 書き出し: {} に {} 音色", wav_dir.display(), targets.len());
    Ok(())
}
