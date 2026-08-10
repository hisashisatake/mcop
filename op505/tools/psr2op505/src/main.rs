//! psr2op505 — PSR-70（OPQ/YM3806）ROM2音色データを OP505 の `.op505` プリセットバンクへ
//! **直接**変換する（中間の`.38x6`ファイルを経由しない。psr2x6 + op505-core adapter.rs の
//! EG変換ロジックを1ツール内で合成する。詳細は`psr2op505::conv`のdocコメント参照）。
//!
//! 使い方:
//! ```text
//! psr2op505 <ROM2.bin> <output_dir> [--bank <N>] [--voices panel|extended|all] [--wav]
//!           [--mod-cap <0-255>] [--sustain <0.0-1.0>] [--cutoff <0-255>] [--attack <bias|none|curve>]
//! ```
//! - 入力は PSR-70 サウンドROM（`ROM2.bin`、32KB）。psr2x6と同一形式。
//! - `--bank` の既定は 0。
//! - `--voices` の既定は `all`（全68音色）。`panel`=voice0-31、`extended`=voice0-31+52-67。
//! - 出力は `<output_dir>/b<bank>.op505`（128件超は連番バンクへ分割）。
//! - `--mod-cap <0-255|none|off>` モジュレーターTL天井（既定は天井なし）。
//! - `--sustain <0.0-1.0>` キャリアのサステイン延長（味付け、既定 0.0=実機準拠）。
//! - `--cutoff <0-255>` ローパスフィルターのカットオフ（味付け、既定=全開255）。
//! - `--attack <bias|none|curve>` アタック立ち上がりの表現方法（既定 `none`。opz2op505の
//!   聴感A/B判断を踏襲。詳細は`psr2op505::conv::AttackMode`）。
//! - `--wav` を指定すると `<output_dir>/wav/voiceNN.wav` へ試聴用WAVを出力する
//!   （`Op505Engine`で直接レンダリング）。

use std::path::PathBuf;
use std::process::ExitCode;

use psr2op505::conv::{self, AttackMode};
use psr2x6::conv::{NamedVoice, PsrConvOptions};
use psr2x6::rom2;

/// 出力対象の音色範囲（psr2x6と同一区分）。
#[derive(Clone, Copy, Debug, Default)]
enum VoiceMode {
    Panel,
    Extended,
    #[default]
    All,
}

impl VoiceMode {
    fn filter(&self, voices: Vec<NamedVoice>) -> Vec<NamedVoice> {
        match self {
            VoiceMode::Panel => voices.into_iter().take(32).collect(),
            VoiceMode::Extended => voices
                .into_iter()
                .enumerate()
                .filter(|(i, _)| *i < 32 || *i >= 52)
                .map(|(_, v)| v)
                .collect(),
            VoiceMode::All => voices,
        }
    }

    fn label(&self) -> &str {
        match self {
            VoiceMode::Panel => "panel (voice0-31, 32音色)",
            VoiceMode::Extended => "extended (voice0-31 + 52-67, 48音色)",
            VoiceMode::All => "all (全68音色)",
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("psr2op505: {msg}");
            eprintln!(
                "usage: psr2op505 <ROM2.bin> <output_dir> [--bank <N>] [--voices panel|extended|all] [--wav]"
            );
            eprintln!("        [--mod-cap <0-255>] [--sustain <0.0-1.0>] [--cutoff <0-255>] [--attack <bias|none|curve>]");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    input: PathBuf,
    output_dir: PathBuf,
    start_bank: u16,
    voice_mode: VoiceMode,
    opts: PsrConvOptions,
    attack_mode: AttackMode,
    wav: bool,
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
    let mut positional: Vec<&String> = Vec::new();
    let mut start_bank: u16 = 0;
    let mut voice_mode = VoiceMode::default();
    let mut opts = PsrConvOptions::default();
    let mut attack_mode = AttackMode::None;
    let mut wav = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                start_bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
                i += 2;
            }
            "--voices" => {
                let v = args.get(i + 1).ok_or("--voices に値がありません")?;
                voice_mode = match v.as_str() {
                    "panel" => VoiceMode::Panel,
                    "extended" => VoiceMode::Extended,
                    "all" => VoiceMode::All,
                    _ => return Err(format!("--voices の値が不正: {v}（panel / extended / all）")),
                };
                i += 2;
            }
            "--mod-cap" => {
                let v = args.get(i + 1).ok_or("--mod-cap に値がありません")?;
                opts.mod_tl_cap = if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("off") {
                    None
                } else {
                    Some(v.parse::<u8>().map_err(|_| format!("--mod-cap の値が不正(0-255 または none): {v}"))?)
                };
                i += 2;
            }
            "--sustain" => {
                let v = args.get(i + 1).ok_or("--sustain に値がありません")?;
                opts.carrier_sustain = v.parse::<f32>().map_err(|_| format!("--sustain の値が不正(0.0-1.0): {v}"))?;
                if !(0.0..=1.0).contains(&opts.carrier_sustain) {
                    return Err(format!("--sustain は 0.0〜1.0 で指定してください: {v}"));
                }
                i += 2;
            }
            "--cutoff" => {
                let v = args.get(i + 1).ok_or("--cutoff に値がありません")?;
                opts.filter_cutoff = Some(v.parse::<u8>().map_err(|_| format!("--cutoff の値が不正(0-255): {v}"))?);
                i += 2;
            }
            "--attack" => {
                let v = args.get(i + 1).ok_or("--attack に値がありません")?;
                attack_mode = parse_attack_mode(v)?;
                i += 2;
            }
            "--wav" => { wav = true; i += 1; }
            _ => { positional.push(&args[i]); i += 1; }
        }
    }
    if positional.len() != 2 {
        return Err("入力ファイルと出力ディレクトリの2引数が必要です".to_string());
    }
    Ok(Args {
        input: PathBuf::from(positional[0]),
        output_dir: PathBuf::from(positional[1]),
        start_bank,
        voice_mode,
        opts,
        attack_mode,
        wav,
    })
}

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    let rom = std::fs::read(&args.input)
        .map_err(|e| format!("ROM2の読み込みに失敗: {}: {e}", args.input.display()))?;
    let all_voices = rom2::parse_rom2_voices(&rom)?;
    let voices = args.voice_mode.filter(all_voices);
    if voices.is_empty() {
        return Err("変換対象のボイスが0件でした".to_string());
    }

    std::fs::create_dir_all(&args.output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {}: {e}", args.output_dir.display()))?;

    if args.wav {
        render_wavs(&voices, &args.output_dir, args.opts, args.attack_mode)?;
    }

    let (files, all_warnings) =
        conv::voices_to_op505_preset_files_opts(args.start_bank, &voices, args.opts, args.attack_mode);

    for file in &files {
        let bank = match file {
            op505_core::Op505PresetFile::Presets { bank, .. } | op505_core::Op505PresetFile::Programs { bank, .. } => *bank,
        };
        let count = match file {
            op505_core::Op505PresetFile::Presets { presets, .. } => presets.len(),
            op505_core::Op505PresetFile::Programs { programs, .. } => programs.len(),
        };
        let path = args.output_dir.join(format!("b{bank}.op505"));
        let json = file.to_json().map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("書き込みに失敗: {}: {e}", path.display()))?;
        println!("書き出し: {} ({count} 音色)", path.display());
    }
    println!("完了: {} バンク / {} 音色 [--voices {}]", files.len(), voices.len(), args.voice_mode.label());

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

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする（試聴検証用、`Op505Engine`直接使用）。
/// C4(261.63Hz)を1.2秒キーオン後、3秒リリース（psr2x6のrender_wavsと同一設定）。
fn render_wavs(
    voices: &[NamedVoice],
    output_dir: &std::path::Path,
    opts: PsrConvOptions,
    attack_mode: AttackMode,
) -> Result<(), String> {
    use op505_core::Op505Engine;
    use sound_core::Vco;
    const SR: f32 = 44_100.0;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| format!("wavディレクトリ作成に失敗: {e}"))?;

    for (i, nv) in voices.iter().enumerate() {
        let (patch, _warnings) = conv::voice_to_op505_patch(&nv.voice, opts, attack_mode);
        let mut engine = Op505Engine::new(SR);
        engine.set_patch(patch);
        engine.note_on(0, 261.63, 110);

        let on = (SR * 1.2) as usize;
        let off = (SR * 3.0) as usize;
        let mut samples = vec![0.0f32; on];
        engine.render(&mut samples, 1);
        engine.note_off(0);
        let mut tail = vec![0.0f32; off];
        engine.render(&mut tail, 1);
        samples.extend_from_slice(&tail);

        let path = wav_dir.join(format!("voice{i:02}.wav"));
        smf2wav::write_wav_mono16(&path, &samples, SR as u32)
            .map_err(|e| format!("WAV書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV書き出し: {} に {} 音色", wav_dir.display(), voices.len());
    Ok(())
}
