//! opz2x6 — TX81Z sysex (.syx) を ym38x6 の .38x6 プリセットに変換。
//!
//! 使い方:
//! ```text
//! opz2x6 <input.syx> [output_dir] [--bank <N>] [--split] [--wav]
//!        [--on <秒>] [--octave <N>] [--note <音階>] [--voice <N>] [--mod-cap <N>] [--fb <N>]
//! ```
//! - `output_dir` 省略時は入力ファイルと同じディレクトリに出力する。
//! - `--bank` の既定は 0。
//! - `--split` を指定すると音色ごとに個別ファイル（Programs形式）を出力する。
//!   省略時は全音色を1ファイル（Presets形式）にまとめる。
//! - `--wav` を指定すると `<output_dir>/wav/<NNN>_<name>.wav` へ試聴用WAVを出力する。
//! - `--on <秒>` でキーオン時間（既定 2.0 秒）を指定する。
//! - `--octave <N>` でオクターブ（既定 4）を指定する。
//! - `--note <音階>` で音階（C/D/E/F/G/A/B、既定 C）を指定する。
//! - `--voice <N>` を指定すると WAV 出力をその音色番号（0始まり）1件のみに絞る。
//! - `--mod-cap <N>` でモジュレーター TL 天井（既定 180）を上書きする。
//!   大きいほど変調が深く明るい（254 でノイズ寄り）。音質追い込み用。
//! - `--fb <N>` でチャンネルフィードバック（0-255）を全音色一律で上書きする。
//!   省略時は .syx 由来。切り分け診断用（`--fb 0` でフィードバック無効）。
//! - `--ksr <N>` で全オペレーターの KSR（0-255）を上書きする。
//!   省略時は .syx 由来。`--ksr 0` で高音のエンベロープ加速を弱める。
//! - `--sustain <0.0-1.0>` キャリアのサステイン延長（味付け、既定 0.0=実機忠実）。
//!   大きいほどエレピ/ピアノの鳴りが伸びる（実機の打鍵的な減衰から意図的に離す）。
//! - `--cutoff <0-255>` ローパスフィルターのカットオフ（味付け、既定=全開255=20kHz）。
//!   低いほど高域を削り倍音過多を抑える（180≈2.8kHz / 200≈4.5kHz）。変調は保つので
//!   基音（FMサイドバンド）を失わずに明るさだけ落とせる（--mod-capより音程が壊れない）。
//! - `--smf <song.mid>` を指定すると、変換した音色バンクで SMF を ym38x6-core 再生し、
//!   `<output_dir>/smf/<midistem>.wav` へ出力する（プログラム番号＝ボイス番号）。

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
            eprintln!(
                "usage: opz2x6 <input.syx> [output_dir] [--bank <N>] [--split] [--wav]"
            );
            eprintln!(
                "       [--on <秒>] [--octave <N>] [--note <C/D/E/F/G/A/B>] [--voice <N>] [--mod-cap <N>] [--fb <N>]"
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
    voice_filter: Option<usize>,
    opts: conv::ConvOptions,
    smf: Option<PathBuf>,
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

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut bank: u16 = 0;
    let mut split = false;
    let mut wav = false;
    let mut on_secs: f32 = 2.0;
    let mut octave: i32 = 4;
    let mut note = "C".to_string();
    let mut voice_filter: Option<usize> = None;
    let mut mod_cap: u8 = opz2x6::conv::DEFAULT_MOD_TL_CAP;
    let mut fb_override: Option<u8> = None;
    let mut ksr_override: Option<u8> = None;
    let mut carrier_sustain: f32 = 0.0;
    let mut filter_cutoff: Option<u8> = None;
    let mut smf: Option<PathBuf> = None;
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
            "--mod-cap" => {
                let v = args.get(i + 1).ok_or("--mod-cap に値がありません")?;
                mod_cap = v.parse::<u8>().map_err(|_| format!("--mod-cap の値が不正(0-255): {v}"))?;
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
            "--smf" => {
                let v = args.get(i + 1).ok_or("--smf に値がありません")?;
                smf = Some(PathBuf::from(v));
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
        voice_filter,
        opts: conv::ConvOptions { mod_tl_cap: mod_cap, fb_override, ksr_override, carrier_sustain, filter_cutoff },
        smf,
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

    if let Some(smf_path) = &args.smf {
        render_smf_to_wav(&voices, smf_path, &args.output_dir, args.opts)?;
    }

    if args.wav {
        render_wavs(&voices, &args.output_dir, &args.wav_cfg, args.voice_filter, args.opts)?;
    }

    if args.split {
        for voice in &voices {
            let entry = conv::voice_to_entry_opts(voice, args.opts);
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
        let presets: Vec<_> = voices.iter().map(|v| conv::voice_to_entry_opts(v, args.opts)).collect();
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

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする（試聴用）。
/// ファイル名は `<NNN>_<name>.wav`（NNN=音色番号0始まり）。
/// `voice_filter` が `Some(N)` のときは音色番号 N の1件のみを出力する。
fn render_wavs(
    voices: &[parse::OpzVoice],
    output_dir: &Path,
    cfg: &WavConfig,
    voice_filter: Option<usize>,
    opts: conv::ConvOptions,
) -> Result<(), String> {
    use ym38x6_core::{SoundEngine, Ym38x6Engine};
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
        let patch = conv::voice_to_patch_opts(voice, opts);
        let mut engine = Ym38x6Engine::new(SR);
        engine.note_on_with_velocity(0, cfg.frequency, 80, patch);

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
            format!("{idx:03}.wav")
        } else {
            format!("{idx:03}_{safe}.wav")
        };
        // ピーク正規化: -6dBFS（0.5）を目標。無音に近い場合はスキップ。
        let peak = buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        if peak > 1e-4 {
            let norm = 0.5 / peak;
            for s in &mut buf { *s *= norm; }
        }

        let path = wav_dir.join(&filename);
        smf2wav::write_wav_mono16(&path, &buf, SR as u32)
            .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV 書き出し: {} に {} 音色", wav_dir.display(), targets.len());
    Ok(())
}

/// SMF を変換済み音色バンクで再生し、`<output_dir>/smf/<midistem>.wav` へ出力する。
/// プログラム番号 = ボイス番号で音色を選ぶ（smf2wav に委譲）。
fn render_smf_to_wav(
    voices: &[parse::OpzVoice],
    smf_path: &Path,
    output_dir: &Path,
    opts: conv::ConvOptions,
) -> Result<(), String> {
    const SR: f32 = 44_100.0;
    let smf_data = std::fs::read(smf_path)
        .map_err(|e| format!("{}: {e}", smf_path.display()))?;
    let patches: Vec<_> = voices.iter().map(|v| conv::voice_to_patch_opts(v, opts)).collect();
    let bank = smf2wav::PatchBank::from_patches(&patches)?;

    let mut buf = smf2wav::render_smf(&smf_data, &bank, SR, 2.0, None)?;
    smf2wav::normalize_peak(&mut buf, 0.5);

    let smf_dir = output_dir.join("smf");
    std::fs::create_dir_all(&smf_dir)
        .map_err(|e| format!("smf ディレクトリ作成に失敗: {e}"))?;
    let stem = smf_path.file_stem().and_then(|s| s.to_str()).unwrap_or("song");
    let safe = parse::sanitize_filename(stem);
    let out_path = smf_dir.join(format!("{}.wav", if safe.is_empty() { "song".into() } else { safe }));
    smf2wav::write_wav_mono16(&out_path, &buf, SR as u32)
        .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", out_path.display()))?;
    println!("SMF レンダリング: {} ({:.1}秒)", out_path.display(), buf.len() as f32 / SR);
    Ok(())
}
