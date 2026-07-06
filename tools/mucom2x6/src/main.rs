//! mucom2x6 — MUCOM88 バイナリ音色バンク（`voice.dat`）を ym38x6 の `.38x6` プリセットバンクへ変換。
//!
//! 使い方:
//! ```text
//! mucom2x6 <voice.dat> <output_dir> [--bank <N>] [--wav] [--on <秒>] [--octave <N>] [--note <音階>] [--slot <N>]
//! ```
//! - 入力は MUCOM88 バイナリ音色バンク（256スロット × 32バイト = 8192バイト固定）。
//! - `--bank` の既定は `WAVEFORM_MEMORY_BANK + 1`。
//! - 出力は `<output_dir>/b<bank>.38x6`（slot 0-127 は Bank N、128-255 は Bank N+1）。
//! - 全AR=0のスロットは除外する。
//! - `--wav` を指定すると `<output_dir>/wav/slotNNN.wav` へ試聴用WAVを出力する。
//! - `--on <秒>` でキーオン時間（既定 4.0 秒）を指定する。
//! - `--octave <N>` でオクターブ（既定 4）を指定する。
//! - `--note <音階>` で音階（C/D/E/F/G/A/B、既定 C）を指定する。
//! - `--slot <N>` を指定すると WAV 出力をその音色番号（MUCOM88 の @N）1件のみに絞る。

use std::path::PathBuf;
use std::process::ExitCode;

use mucom2x6::{conv, mucom88};
use mucom2x6::conv::{bank_of, preset_count, voices_to_preset_files};

/// WAV 試聴レンダリングの設定。
struct WavConfig {
    /// キーオン持続時間（秒）。
    on_secs: f32,
    /// リリース収録時間（秒）。
    off_secs: f32,
    /// 発音周波数（Hz）。
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
            eprintln!("mucom2x6: {msg}");
            eprintln!(
                "usage: mucom2x6 <voice.dat> <output_dir> [--bank <N>] [--wav] \
                 [--on <秒>] [--octave <N>] [--note <C/D/E/F/G/A/B>] [--slot <N>] [--fb-max <f>] [--fb-2sample]"
            );
            ExitCode::FAILURE
        }
    }
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

    // 【実験用】フィードバック帰還方式の上書き（音色のFB調査用）。WAVレンダリング前に
    // プロセスグローバルへ設定する。production既定は1サンプル帰還・max=1.8。
    if cli.fb_2sample {
        ym38x6_core::set_feedback_two_sample(true);
        eprintln!("実験: feedback帰還を2サンプル平均 (out[n-1]+out[n-2])/2 に切替");
    }
    if let Some(m) = cli.fb_max {
        ym38x6_core::set_feedback_scale_max(Some(m));
        eprintln!("実験: feedback_to_scale 最大値を {m} に上書き（既定1.8）");
    }

    if args.iter().any(|a| a == "--wav") {
        render_wavs(&voices, &cli.output_dir, &cli.wav_cfg, cli.slot_filter)?;
    }

    let (output_dir, start_bank) = (cli.output_dir, cli.start_bank);

    let files = voices_to_preset_files(start_bank, &voices);
    for file in &files {
        let path = output_dir.join(format!("b{}.38x6", bank_of(file)));
        let json = file
            .to_json()
            .map_err(|e| format!("JSON シリアライズに失敗: {e}"))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("書き込みに失敗: {}: {e}", path.display()))?;
        println!("書き出し: {} ({} 音色)", path.display(), preset_count(file));
    }
    println!("完了: {} バンク / {} 音色", files.len(), voices.len());
    Ok(())
}

/// 音階名（C/D/E/F/G/A/B）→半音オフセット（Cを0とする）。
fn note_name_to_semitone(note: &str) -> Result<i32, String> {
    match note.to_uppercase().as_str() {
        "C" => Ok(0),
        "D" => Ok(2),
        "E" => Ok(4),
        "F" => Ok(5),
        "G" => Ok(7),
        "A" => Ok(9),
        "B" => Ok(11),
        _ => Err(format!("不正な音階: {note}（C/D/E/F/G/A/B で指定してください）")),
    }
}

/// オクターブ + 音階名 → 周波数（Hz）。MIDI 番号経由で A4=440Hz を基準に計算。
fn note_to_freq(octave: i32, note: &str) -> Result<f32, String> {
    let semitone = note_name_to_semitone(note)?;
    let midi = (octave + 1) * 12 + semitone;
    Ok(440.0 * 2f32.powf((midi - 69) as f32 / 12.0))
}

/// コマンドライン解析結果。
struct Cli {
    input: PathBuf,
    output_dir: PathBuf,
    start_bank: u16,
    wav_cfg: WavConfig,
    slot_filter: Option<u16>,
    /// 【実験用】feedback_to_scale 上限の上書き（None=既定1.8）。
    fb_max: Option<f32>,
    /// 【実験用】2サンプル平均帰還に切替えるか。
    fb_2sample: bool,
}

fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut start_bank: u16 = ym38x6_core::WAVEFORM_MEMORY_BANK + 1;
    let mut on_secs: f32 = 4.0;
    let mut octave: i32 = 4;
    let mut note: String = "C".to_string();
    let mut slot_filter: Option<u16> = None;
    let mut fb_max: Option<f32> = None;
    let mut fb_2sample = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fb-max" => {
                let v = args.get(i + 1).ok_or("--fb-max に値がありません")?;
                fb_max = Some(v.parse().map_err(|_| format!("--fb-max の値が不正: {v}"))?);
                i += 2;
            }
            "--fb-2sample" => {
                fb_2sample = true;
                i += 1;
            }
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
            "--wav" => {
                i += 1;
            }
            _ => {
                positional.push(&args[i]);
                i += 1;
            }
        }
    }
    if positional.len() != 2 {
        return Err("入力ファイルと出力ディレクトリの 2 引数が必要です".to_string());
    }
    let frequency = note_to_freq(octave, &note)?;
    let wav_cfg = WavConfig { on_secs, off_secs: 1.5, frequency };
    Ok(Cli {
        input: PathBuf::from(positional[0]),
        output_dir: PathBuf::from(positional[1]),
        start_bank,
        wav_cfg,
        slot_filter,
        fb_max,
        fb_2sample,
    })
}

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする（試聴検証用）。
/// ファイル名は MUCOM88 の @N 番号に対応するスロット番号（slot000.wav 等）。
/// `slot_filter` が `Some(N)` のときは音色番号 N の1件のみを出力する。
fn render_wavs(
    voices: &[conv::NamedVoice],
    output_dir: &std::path::Path,
    cfg: &WavConfig,
    slot_filter: Option<u16>,
) -> Result<(), String> {
    use ym38x6_core::Ym38x6Engine;
    const SR: f32 = 44_100.0;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| format!("wav ディレクトリ作成に失敗: {e}"))?;

    let targets: Vec<&conv::NamedVoice> = match slot_filter {
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
        let patch = nv.voice.to_ym38x6_patch();
        if slot_filter.is_some() {
            let v = &nv.voice;
            eprintln!("=== slot {} '{}' ===", nv.slot, nv.name);
            eprintln!("OPN raw: algorithm={} feedback={}", v.algorithm, v.feedback);
            for (i, op) in v.operators.iter().enumerate() {
                eprintln!(
                    "  OP{} tl={:3} ar={:2} d1r={:2} d2r={:2} d1l={:2} rr={:2} mul={:2} dt1={} ks={} am={}",
                    i + 1, op.tl, op.ar, op.d1r, op.d2r, op.d1l, op.rr, op.mul, op.dt1, op.ks, op.am_enable
                );
            }
            let carriers = ym38x6_core::algorithm::ALGORITHMS[patch.channel.algorithm as usize].carriers;
            eprintln!(
                "38x6 patch: algorithm={} feedback={} carriers(op_index)={:?}",
                patch.channel.algorithm, patch.channel.feedback, carriers
            );
            for (i, op) in patch.operators.iter().enumerate() {
                eprintln!(
                    "  OP{} tl={:3} ar={:3} d1r={:3} d2r={:3} d1l={:3} rr={:3} mul={:2} dt1={:3} ksr={:3}",
                    i + 1, op.tl, op.ar, op.d1r, op.d2r, op.d1l, op.rr, op.mul, op.dt1, op.ksr
                );
            }
        }
        let mut engine = Ym38x6Engine::new(SR);
        engine.note_on(0, cfg.frequency, 110, patch);

        let on = (SR * cfg.on_secs) as usize;
        let off = (SR * cfg.off_secs) as usize;
        let mut samples = vec![0.0f32; on];
        engine.render(&mut samples, 1);
        engine.note_off(0);
        let mut tail = vec![0.0f32; off];
        engine.render(&mut tail, 1);
        samples.extend_from_slice(&tail);

        let path = wav_dir.join(format!("slot{:03}.wav", nv.slot));
        write_wav_mono16(&path, &samples, SR as u32)
            .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV 書き出し: {} に {} 音色", wav_dir.display(), targets.len());
    Ok(())
}

/// mono / 16bit PCM の最小 WAV ライター。
fn write_wav_mono16(path: &std::path::Path, samples: &[f32], sr: u32) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());  // PCM
    out.extend_from_slice(&1u16.to_le_bytes());  // mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out)
}
