//! mucom2op505 — MUCOM88 バイナリ音色バンク（`voice.dat`）を OP505 の `.op505` プリセット
//! バンクへ**直接**変換する（中間の`.38x6`ファイルを経由しない。mucom2x6 + op505-core
//! adapter.rs のEG変換ロジックを1ツール内で合成する。詳細は`mucom2op505::conv`のdocコメント参照）。
//!
//! 使い方:
//! ```text
//! mucom2op505 <voice.dat> <output_dir> [--bank <N>] [--wav] [--on <秒>] [--octave <N>]
//!             [--note <音階>] [--slot <N>] [--attack <bias|none|curve>]
//! ```
//! - 入力は MUCOM88 バイナリ音色バンク（256スロット × 32バイト = 8192バイト固定）。
//! - `--bank` の既定は 0。
//! - 出力は `<output_dir>/b<bank>.op505`（slot 0-127 は Bank N、128-255 は Bank N+1）。
//! - 全AR=0のスロットは除外する。
//! - `--wav` を指定すると `<output_dir>/wav/slotNNN_<name>.wav` へ試聴用WAVを出力する
//!   （`Op505Engine`で直接レンダリング）。
//! - `--on <秒>` でキーオン時間（既定 4.0 秒）を指定する。
//! - `--octave <N>` でオクターブ（既定 4）を指定する。
//! - `--note <音階>` で音階（C/D/E/F/G/A/B、既定 C）を指定する。
//! - `--slot <N>` を指定すると WAV 出力をその音色番号（MUCOM88 の @N）1件のみに絞る。
//! - `--attack <bias|none|curve>` アタック立ち上がりの表現方法（既定 `none`。opz2op505の
//!   聴感A/B判断を踏襲。詳細は`mucom2op505::conv::AttackMode`）。
//!
//! op505-coreはフィードバック帰還を常時2サンプル平均・上限1.8固定で扱うため
//! （mucom2x6の`--fb-max`/`--fb-2sample`実験フラグに相当する上書き機構は持たない）、
//! このツールに対応オプションはない。

use std::path::PathBuf;
use std::process::ExitCode;

use mucom2op505::conv::{self, AttackMode};
use mucom2x6::conv::NamedVoice;
use mucom2x6::mucom88;

struct WavConfig {
    on_secs: f32,
    off_secs: f32,
    frequency: f32,
}

impl Default for WavConfig {
    fn default() -> Self {
        Self { on_secs: 4.0, off_secs: 1.5, frequency: 261.63 }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("mucom2op505: {msg}");
            eprintln!(
                "usage: mucom2op505 <voice.dat> <output_dir> [--bank <N>] [--wav] \
                 [--on <秒>] [--octave <N>] [--note <C/D/E/F/G/A/B>] [--slot <N>] [--attack <bias|none|curve>]"
            );
            ExitCode::FAILURE
        }
    }
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

struct Cli {
    input: PathBuf,
    output_dir: PathBuf,
    start_bank: u16,
    wav_cfg: WavConfig,
    slot_filter: Option<u16>,
    attack_mode: AttackMode,
    wav: bool,
}

fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut start_bank: u16 = 0;
    let mut on_secs: f32 = 4.0;
    let mut octave: i32 = 4;
    let mut note: String = "C".to_string();
    let mut slot_filter: Option<u16> = None;
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
            "--on" => {
                let v = args.get(i + 1).ok_or("--on に値がありません")?;
                on_secs = v.parse::<f32>().map_err(|_| format!("--on の値が不正: {v}"))?;
                if on_secs <= 0.0 {
                    return Err(format!("--on は正の値を指定してください: {v}"));
                }
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
            "--slot" => {
                let v = args.get(i + 1).ok_or("--slot に値がありません")?;
                let n: u16 = v.parse().map_err(|_| format!("--slot の値が不正: {v}"))?;
                if n > 255 {
                    return Err(format!("--slot は 0〜255 で指定してください: {v}"));
                }
                slot_filter = Some(n);
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
        return Err("入力ファイルと出力ディレクトリの 2 引数が必要です".to_string());
    }
    let frequency = note_to_freq(octave, &note)?;
    Ok(Cli {
        input: PathBuf::from(positional[0]),
        output_dir: PathBuf::from(positional[1]),
        start_bank,
        wav_cfg: WavConfig { on_secs, off_secs: 1.5, frequency },
        slot_filter,
        attack_mode,
        wav,
    })
}

fn run(args: &[String]) -> Result<(), String> {
    let cli = parse_args(args)?;

    let dat = std::fs::read(&cli.input)
        .map_err(|e| format!("voice.dat の読み込みに失敗: {}: {e}", cli.input.display()))?;
    let voices = mucom88::parse_voice_dat(&dat)?;
    if voices.is_empty() {
        return Err("有効なボイスが 0 件でした".to_string());
    }

    std::fs::create_dir_all(&cli.output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {}: {e}", cli.output_dir.display()))?;

    if cli.wav {
        render_wavs(&voices, &cli.output_dir, &cli.wav_cfg, cli.slot_filter, cli.attack_mode)?;
    }

    let (files, all_warnings) = conv::voices_to_op505_preset_files(cli.start_bank, &voices, cli.attack_mode);
    for file in &files {
        let (bank, count) = match file {
            op505_core::Op505PresetFile::Presets { bank, presets } => (*bank, presets.len()),
            op505_core::Op505PresetFile::Programs { bank, programs } => (*bank, programs.len()),
        };
        let path = cli.output_dir.join(format!("b{bank}.op505"));
        let json = file.to_json().map_err(|e| format!("JSON シリアライズに失敗: {e}"))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("書き込みに失敗: {}: {e}", path.display()))?;
        println!("書き出し: {} ({count} 音色)", path.display());
    }
    println!("完了: {} バンク / {} 音色", files.len(), voices.len());

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
/// ファイル名は MUCOM88 の @N 番号に対応するスロット番号（slot000.wav 等）。
/// `slot_filter` が `Some(N)` のときは音色番号 N の1件のみを出力する。
fn render_wavs(
    voices: &[NamedVoice],
    output_dir: &std::path::Path,
    cfg: &WavConfig,
    slot_filter: Option<u16>,
    attack_mode: AttackMode,
) -> Result<(), String> {
    use op505_core::Op505Engine;
    use sound_core::Vco;
    const SR: f32 = 44_100.0;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| format!("wav ディレクトリ作成に失敗: {e}"))?;

    let targets: Vec<&NamedVoice> = match slot_filter {
        Some(n) => voices.iter().filter(|nv| nv.slot == n).collect(),
        None => voices.iter().collect(),
    };
    if let Some(n) = slot_filter {
        if targets.is_empty() {
            return Err(format!(
                "--slot {n} に対応する音色が見つかりません（AR=0で除外された可能性があります）"
            ));
        }
    }

    for nv in &targets {
        let (patch, _warnings) = conv::voice_to_op505_patch(&nv.voice, attack_mode);
        let mut engine = Op505Engine::new(SR);
        engine.set_patch(patch);
        engine.note_on(0, cfg.frequency, 110);

        let on = (SR * cfg.on_secs) as usize;
        let off = (SR * cfg.off_secs) as usize;
        let mut samples = vec![0.0f32; on];
        engine.render(&mut samples, 1);
        engine.note_off(0);
        let mut tail = vec![0.0f32; off];
        engine.render(&mut tail, 1);
        samples.extend_from_slice(&tail);

        let full_name = mucom88::halfwidth_kana_to_fullwidth(&nv.name);
        let safe = mucom88::sanitize_filename_keep_japanese(&full_name);
        let filename = if safe.is_empty() {
            format!("slot{:03}.wav", nv.slot)
        } else {
            format!("slot{:03}_{safe}.wav", nv.slot)
        };
        let path = wav_dir.join(filename);
        smf2wav::write_wav_mono16(&path, &samples, SR as u32)
            .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV 書き出し: {} に {} 音色", wav_dir.display(), targets.len());
    Ok(())
}
