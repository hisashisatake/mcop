//! vgm2op505 — YM2151(OPM) / OPN系(YM2612/YM2203/YM2608) の VGM/VGZ から
//! 音色（.op505）と SMF（.mid）または WAV を **直接** 抽出する（中間の`.38x6`ファイルを
//! 経由しない）。演奏ロジック（VGM逐次デコード・ピッチベンド・SSGコアレス処理等）は
//! かつてvgm2x6のlibクレートを再利用していたが、デフォーク（ym38x6依存排除）に伴い
//! `src/{vgm,opm,opn,ssg,smf,patch,play}.rs`へ複製・自立化した（各モジュール冒頭に
//! 由来コミットを記載）。音色変換は opm2op505/mucom2op505 の`voice_to_op505_patch`を
//! 直接使い、SSG合成パッチ（psg/noise/mix_patch）は[crate::ssg]がOP505ネイティブとして
//! 直接構築する（`op505_core::adapter`は経由しない）。
//!
//! 使い方:
//! ```text
//! vgm2op505 <input.vgz|.vgm> [--out <dir>] [--out-bank <file>] [--out-midi <file>] [--bank N]
//!           [--out-wav <file>] [--wav] [--gain <factor>] [--fm-gain <factor>] [--ssg-gain <factor>]
//!           [--attack <bias|none|curve>] [--only-ch <n>]
//! ```
//! vgm2x6と異なり `--dump-pitch`（ピッチロジック共通で新情報が無い）と
//! `--fb-max`/`--fb-2sample`/`--fb-1sample`（op505-coreはフィードバック実験フラグを
//! 搭載しない設計）は持たない。`--attack`は他の直接変換ツール群と同じ
//! （既定`none`。詳細は`opm2op505::conv::AttackMode`）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use op505_core::{Op505Engine, Op505PresetEntry, Op505PresetFile};
use vgm2op505::convert::{parse_attack_mode, AttackMode, Op505Converter};
use vgm2op505::patch::PatchBank;
use vgm2op505::play::{self, OpnInfo, SmfSink, SourceChip, WavSink};
use vgm2op505::vgm;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vgm2op505: {msg}");
            eprintln!(
                "usage: vgm2op505 <input.vgz|.vgm> [--out <dir>] \
                 [--out-bank <file>] [--out-midi <file>] [--bank N] [--out-wav <file>] [--wav] \
                 [--gain <factor>] [--fm-gain <factor>] [--ssg-gain <factor>] \
                 [--attack <bias|none|curve>] [--only-ch <n>]"
            );
            ExitCode::FAILURE
        }
    }
}

// ===========================================================================
// CLI引数
// ===========================================================================

struct Args {
    input: PathBuf,
    out_bank: PathBuf,
    out_midi: PathBuf,
    bank: u16,
    wav: Option<PathBuf>,
    gain: f32,
    fm_gain: f32,
    ssg_gain: f32,
    attack: AttackMode,
    only_ch: Option<usize>,
}

/// 音量ゲイン係数(線形)を基準velocity 100 に適用する。vgm2x6::scaled_velocityと同一式。
fn scaled_velocity(gain: f32) -> u8 {
    (100.0 * gain).round().clamp(1.0, 127.0) as u8
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut out_bank: Option<PathBuf> = None;
    let mut out_midi: Option<PathBuf> = None;
    let mut bank: u16 = 0;
    let mut fm_gain: f32 = 1.0;
    let mut ssg_gain: f32 = 1.0;
    let mut out_wav: Option<PathBuf> = None;
    let mut wav_flag = false;
    let mut gain: f32 = 1.0;
    let mut attack = AttackMode::None;
    let mut only_ch: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--only-ch" => {
                let v = args.get(i + 1).ok_or("--only-ch に値がありません")?;
                only_ch = Some(v.parse().map_err(|_| format!("--only-ch の値が不正: {v}"))?);
                i += 2;
            }
            "--out-wav" => {
                out_wav = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out-wav に値がありません")?,
                ));
                i += 2;
            }
            "--wav" => {
                wav_flag = true;
                i += 1;
            }
            "--gain" => {
                let v = args.get(i + 1).ok_or("--gain に値がありません")?;
                gain = v.parse().map_err(|_| format!("--gain の値が不正: {v}"))?;
                i += 2;
            }
            "--fm-gain" => {
                let v = args.get(i + 1).ok_or("--fm-gain に値がありません")?;
                fm_gain = v.parse().map_err(|_| format!("--fm-gain の値が不正: {v}"))?;
                i += 2;
            }
            "--ssg-gain" => {
                let v = args.get(i + 1).ok_or("--ssg-gain に値がありません")?;
                ssg_gain = v.parse().map_err(|_| format!("--ssg-gain の値が不正: {v}"))?;
                i += 2;
            }
            "--attack" => {
                let v = args.get(i + 1).ok_or("--attack に値がありません")?;
                attack = parse_attack_mode(v)?;
                i += 2;
            }
            "--out" => {
                out_dir = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out に値がありません")?,
                ));
                i += 2;
            }
            "--out-bank" => {
                out_bank = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out-bank に値がありません")?,
                ));
                i += 2;
            }
            "--out-midi" => {
                out_midi = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--out-midi に値がありません")?,
                ));
                i += 2;
            }
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
                i += 2;
            }
            s => {
                input = Some(PathBuf::from(s));
                i += 1;
            }
        }
    }
    let input = input.ok_or("入力ファイルのパスが必要です")?;
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    let base_dir = out_dir.as_deref().unwrap_or(Path::new("."));
    let out_bank = out_bank.unwrap_or_else(|| base_dir.join(format!("{stem}.op505")));
    let out_midi = out_midi.unwrap_or_else(|| base_dir.join(format!("{stem}.mid")));
    let wav = out_wav.or_else(|| wav_flag.then(|| base_dir.join(format!("{stem}.wav"))));

    Ok(Args { input, out_bank, out_midi, bank, wav, gain, fm_gain, ssg_gain, attack, only_ch })
}

// ===========================================================================
// バンク出力・警告レポート
// ===========================================================================

fn write_bank(bank: &PatchBank, path: &Path, bank_no: u16) -> Result<(), String> {
    if bank.len() == 0 {
        eprintln!("警告: 音色が抽出されませんでした");
        return Ok(());
    }
    let presets: Vec<Op505PresetEntry> = bank
        .entries()
        .map(|(program, name, patch)| Op505PresetEntry {
            program,
            name: name.to_string(),
            patch: *patch,
        })
        .collect();
    let file = Op505PresetFile::Presets { bank: bank_no, presets };
    let json = file.to_json().map_err(|e| format!(".op505 シリアライズに失敗: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

fn report_warnings(bank: &PatchBank) {
    let warnings = bank.warnings();
    if warnings.is_empty() {
        return;
    }
    println!("変換警告 ({} 音色):", warnings.len());
    for (name, ws) in warnings {
        println!("  [{name}]");
        for w in ws {
            println!("    - {w}");
        }
    }
}

// ===========================================================================
// メイン処理
// ===========================================================================

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    let data = vgm::load(&args.input)?;
    let header = vgm::parse_header(&data)?;

    let chip = play::detect_chip(&header).ok_or(
        "YM2151 も OPN系(YM2612/YM2203/YM2608) も見つかりません（対応外のVGMです）",
    )?;

    match chip {
        SourceChip::Opm => {
            eprintln!(
                "YM2151 clock: {} Hz, {} samples",
                header.ym2151_clock, header.total_samples
            );
            if let Some(wav_path) = &args.wav {
                return render_wav(&data, header.data_start, wav_path, args.gain, args.attack);
            }
            run_opm_smf(&data, &header, &args)
        }
        SourceChip::Opn(info) => {
            eprintln!(
                "{} clock: {} Hz, {} samples (FM {}ch{})",
                info.label, info.clock, header.total_samples, info.fm_channels,
                if info.has_ssg { " + SSG 3ch" } else { "" },
            );
            run_opn(&data, header.data_start, info, &args)
        }
    }
}

/// YM2151(OPM) の直接WAVレンダリング（--wav）。
fn render_wav(data: &[u8], data_start: usize, out: &Path, gain: f32, attack: AttackMode) -> Result<(), String> {
    const SR: f32 = 44_100.0;
    let mut engine = Op505Engine::new(SR);
    let audio = play::opm_to_audio(data, data_start, Op505Converter { attack }, &mut engine);
    play::finish_wav(&mut engine, audio, SR, out, gain)
}

/// YM2151(OPM) の SMF + .op505バンクを出力する。
fn run_opm_smf(data: &[u8], header: &vgm::VgmHeader, args: &Args) -> Result<(), String> {
    let mut bank = PatchBank::new(Op505Converter { attack: args.attack });
    let mut smf = play::opm_to_smf(data, header.data_start, &mut bank);

    write_bank(&bank, &args.out_bank, args.bank)?;
    println!("音色バンク: {} ({} 音色)", args.out_bank.display(), bank.len());
    report_warnings(&bank);

    smf.write(&args.out_midi)
        .map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
    println!("MIDI: {}", args.out_midi.display());

    Ok(())
}

/// OPN系の出力（SMF+.op505バンク または WAV）。
fn run_opn(data: &[u8], data_start: usize, info: OpnInfo, args: &Args) -> Result<(), String> {
    let total_ch = info.fm_channels + if info.has_ssg { 3 } else { 0 };
    let mut bank = PatchBank::new(Op505Converter { attack: args.attack });

    let fm_vel = scaled_velocity(args.fm_gain);
    let ssg_vel = scaled_velocity(args.ssg_gain);

    if let Some(c) = args.only_ch {
        eprintln!("実験: エンジンチャンネル {c} のみ発音");
    }

    if let Some(wav_path) = &args.wav {
        let mut sink = WavSink::new(44_100.0, total_ch);
        play::process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);
        return sink.finish_and_write(wav_path, args.gain);
    }

    let mut sink = SmfSink::new(total_ch);
    play::process_opn(data, data_start, info, &mut bank, &mut sink, fm_vel, ssg_vel, args.only_ch);

    write_bank(&bank, &args.out_bank, args.bank)?;
    println!("音色バンク: {} ({} 音色)", args.out_bank.display(), bank.len());
    report_warnings(&bank);
    sink.smf
        .write(&args.out_midi)
        .map_err(|e| format!("MIDI書き出し失敗: {e}"))?;
    println!("MIDI: {}", args.out_midi.display());
    Ok(())
}
