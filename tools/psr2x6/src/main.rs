//! psr2x6 — PSR-70（OPQ/YM3806）音色データを ym38x6 の `.38x6` プリセットバンクへ変換するツール。
//!
//! 使い方:
//! ```text
//! psr2x6 <ROM2.bin> <output_dir> [--bank <N>] [--voices panel|extended|all] [--wav]
//!        [--mod-cap <0-255>] [--sustain <0.0-1.0>] [--cutoff <0-255>]
//! ```
//! - 入力は PSR-70 サウンドROM（`ROM2.bin`、32KB）。出力ともに本クレートには同梱しない（パスは引数指定）。
//! - `--bank` の既定は `WAVEFORM_MEMORY_BANK + 1`（Bank 0はML自動生成用に空けておく）。
//! - `--voices` の既定は `all`（全68音色）。`panel`=voice0-31（32パネル音色）、
//!   `extended`=voice0-31+52-67（48音色、2op複製テーブルvoice32-51を除外）。
//! - 出力は `<output_dir>/b<bank>.38x6`（128件超は連番バンクへ分割）。
//! - `--mod-cap <0-255>` モジュレーターTL天井（既定180）。低いほど変調を抑えノイズが減る。
//!   PSR-70 ROM音色はfeedback/モジュレーターTLが高くノイズが乗りやすいため既定で圧縮する。opz2x6 と同一。
//! - `--sustain <0.0-1.0>` キャリアのサステイン延長（味付け、既定 0.0=実機準拠）。opz2x6 と同一。
//! - `--cutoff <0-255>` ローパスカットオフ（味付け、既定=全開255=20kHz）。低いほど倍音過多を抑える
//!   （180≈2.8kHz/200≈4.5kHz）。変調は保つので基音を失わず明るさだけ落とせる。opz2x6 と同一。
//!
//! ROM2内の音色テーブル（`0x0660`から68音色×64バイト）の抽出は [`rom2`]、
//! OPQ→38x6の変換は [`conv`]（spec-sound.mdの規約準拠）。

mod conv;
mod rom2;

use std::path::PathBuf;
use std::process::ExitCode;

use conv::{bank_of, preset_count, voices_to_preset_files_opts, NamedVoice, PsrConvOptions};

/// 出力対象の音色範囲。
///
/// ROM2 の 68 音色は 3 グループに分かれる（analyze8.ps1 で確定）:
/// - voice0-31 : 4op 通常音色・32パネル音色
/// - voice32-51: op0==op1 AND op2==op3 が平均 12/16 行で成立する 2op 複製テーブル（変換品質が低い）
/// - voice52-67: 4op 通常音色・追加16音色（パネル外）
#[derive(Clone, Copy, Debug, Default)]
enum VoiceMode {
    Panel,    // voice0-31 のみ（32音色）
    Extended, // voice0-31 + voice52-67（48音色、2op複製テーブルを除外）
    #[default]
    All, // 全68音色
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
            eprintln!("psr2x6: {msg}");
            eprintln!("usage: psr2x6 <ROM2.bin> <output_dir> [--bank <N>] [--voices panel|extended|all] [--wav]");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (input, output_dir, start_bank, voice_mode, opts) = parse_args(args)?;

    let rom = std::fs::read(&input)
        .map_err(|e| format!("ROM2の読み込みに失敗: {}: {e}", input.display()))?;
    let all_voices = rom2::parse_rom2_voices(&rom)?;
    let voices = voice_mode.filter(all_voices);
    if voices.is_empty() {
        return Err("変換対象のボイスが0件でした".to_string());
    }

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {}: {e}", output_dir.display()))?;

    // 試聴検証用: --wav 指定時は各音色を WAV(<output_dir>/wav/voiceNN.wav)へレンダリングする。
    if args.iter().any(|a| a == "--wav") {
        render_wavs(&voices, &output_dir, opts)?;
    }

    let files = voices_to_preset_files_opts(start_bank, &voices, opts);

    for file in &files {
        let path = output_dir.join(format!("b{}.38x6", bank_of(file)));
        let json = file
            .to_json()
            .map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("書き込みに失敗: {}: {e}", path.display()))?;
        println!("書き出し: {} ({} 音色)", path.display(), preset_count(file));
    }
    println!("完了: {} バンク / {} 音色 [--voices {}]", files.len(), voices.len(), voice_mode.label());
    Ok(())
}

fn parse_args(args: &[String]) -> Result<(PathBuf, PathBuf, u16, VoiceMode, PsrConvOptions), String> {
    let mut positional: Vec<&String> = Vec::new();
    // 既定: 波形メモリ音源バンクの直後(WAVEFORM_MEMORY_BANK+1)。Bank 0はML自動生成用に空けておく
    let mut start_bank: u16 = ym38x6_core::WAVEFORM_MEMORY_BANK + 1;
    let mut voice_mode = VoiceMode::default();
    let mut opts = PsrConvOptions::default();
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
                opts.mod_tl_cap = v.parse::<u8>().map_err(|_| format!("--mod-cap の値が不正(0-255): {v}"))?;
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
            "--wav" => {
                // 値なしフラグ（run側で処理）。positionalには含めない。
                i += 1;
            }
            _ => {
                positional.push(&args[i]);
                i += 1;
            }
        }
    }
    if positional.len() != 2 {
        return Err("入力ファイルと出力ディレクトリの2引数が必要です".to_string());
    }
    Ok((
        PathBuf::from(positional[0]),
        PathBuf::from(positional[1]),
        start_bank,
        voice_mode,
        opts,
    ))
}

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする（試聴検証用）。
/// C4(261.63Hz)を1.2秒キーオン後、3秒リリース。
/// リリースレートが遅い音色（RR=0で約284.9秒相当）でも減衰の様子を確認できるよう、
/// 通常より長めのテールを確保している。
fn render_wavs(voices: &[NamedVoice], output_dir: &std::path::Path, opts: PsrConvOptions) -> Result<(), String> {
    use ym38x6_core::{Vco, Ym38x6Engine};
    const SR: f32 = 44_100.0;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| format!("wavディレクトリ作成に失敗: {e}"))?;

    for (i, nv) in voices.iter().enumerate() {
        let patch = nv.voice.to_ym38x6_patch_opts(opts);
        let mut engine = Ym38x6Engine::new(SR);
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
        write_wav_mono16(&path, &samples, SR as u32)
            .map_err(|e| format!("WAV書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV書き出し: {} に {} 音色", wav_dir.display(), voices.len());
    Ok(())
}

/// mono / 16bit PCM の最小WAVライター。
fn write_wav_mono16(path: &std::path::Path, samples: &[f32], sr: u32) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmtチャンクサイズ
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits/sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out)
}
