//! mucom2x6 — MUCOM88 バイナリ音色バンク（`voice.dat`）を ym38x6 の `.38x6` プリセットバンクへ変換。
//!
//! 使い方:
//! ```text
//! mucom2x6 <voice.dat> <output_dir> [--bank <N>] [--wav]
//! ```
//! - 入力は MUCOM88 バイナリ音色バンク（256スロット × 32バイト = 8192バイト固定）。
//! - `--bank` の既定は `WAVEFORM_MEMORY_BANK + 1`。
//! - 出力は `<output_dir>/b<bank>.38x6`（slot 0-127 は Bank N、128-255 は Bank N+1）。
//! - 全AR=0のスロットは除外する。
//! - `--wav` を指定すると `<output_dir>/wav/slotNNN.wav` へ試聴用WAVを出力する。

mod conv;
mod mucom88;

use std::path::PathBuf;
use std::process::ExitCode;

use conv::{bank_of, preset_count, voices_to_preset_files};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("mucom2x6: {msg}");
            eprintln!("usage: mucom2x6 <voice.dat> <output_dir> [--bank <N>] [--wav]");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (input, output_dir, start_bank) = parse_args(args)?;

    let dat = std::fs::read(&input)
        .map_err(|e| format!("voice.dat の読み込みに失敗: {}: {e}", input.display()))?;
    let voices = mucom88::parse_voice_dat(&dat)?;
    if voices.is_empty() {
        return Err("有効なボイスが 0 件でした".to_string());
    }

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {}: {e}", output_dir.display()))?;

    if args.iter().any(|a| a == "--wav") {
        render_wavs(&voices, &output_dir)?;
    }

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

fn parse_args(args: &[String]) -> Result<(PathBuf, PathBuf, u16), String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut start_bank: u16 = ym38x6_core::WAVEFORM_MEMORY_BANK + 1;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                start_bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
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
    Ok((PathBuf::from(positional[0]), PathBuf::from(positional[1]), start_bank))
}

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする（試聴検証用）。
/// C4（261.63Hz）を 1.2 秒キーオン後、3 秒リリース。
/// ファイル名は MUCOM88 の @N 番号に対応するスロット番号（slot000.wav 等）。
fn render_wavs(voices: &[conv::NamedVoice], output_dir: &std::path::Path) -> Result<(), String> {
    use ym38x6_core::{SoundEngine, Ym38x6Engine};
    const SR: f32 = 44_100.0;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| format!("wav ディレクトリ作成に失敗: {e}"))?;

    for nv in voices {
        let patch = nv.voice.to_ym38x6_patch();
        let mut engine = Ym38x6Engine::new(SR);
        // MUCOM88win のプレビューは C5（octave:1 相当）で長めに保持する
        engine.note_on_with_velocity(0, 523.25, 110, patch);

        let on = (SR * 3.0) as usize;
        let off = (SR * 1.5) as usize;
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
    println!("WAV 書き出し: {} に {} 音色", wav_dir.display(), voices.len());
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
