//! smf2wav — `.38x6` 音色バンクで標準MIDIファイル（SMF）を再生し、WAV へ書き出す。
//!
//! 使い方:
//! ```text
//! smf2wav <bank.38x6> <song.mid> [out.wav] [--sr <Hz>] [--tail <秒>] [--no-normalize]
//!         [--reverb-send <N>] [--reverb-type <0-7>] [--reverb-time <N>]
//! ```
//! - `out.wav` 省略時は `<song>` と同じディレクトリに `<songの拡張子なし名>.wav` を出力する。
//! - プログラムチェンジ番号 = `.38x6` のプログラム番号で音色を選ぶ
//!   （未定義番号は直下の最も近い定義済み番号へフォールバック）。
//! - `--sr` 出力サンプルレート（既定 44100）。
//! - `--tail` ノートオフ後の残響を伸ばす秒数（既定 2.0）。
//! - `--no-normalize` ピーク正規化（-6dBFS）を無効化する。
//! - `--max-secs <秒>` 出力をこの秒数（テール込み）で打ち切る（試聴の時短用）。
//! - `--reverb-send <N>` マスターリバーブセンド（0-255、既定 0=ドライ）。DAW で
//!   全chに掛けていたリバーブ（CC91 相当）を再現する診断用。0 のとき従来どおり完全ドライ。
//!   これは SMF 内蔵のマスターエフェクト（CC91/93・NRPN(0,2)〜(0,8) で駆動する
//!   `MasterEffects`、render 側で適用）とは**独立した後段の診断用リバーブ**（fx.rs）で、
//!   SMF にエフェクト系 CC/NRPN が無い通常の曲では既定 0 のまま二重掛けにならない。
//! - `--reverb-type <0-7>` リバーブタイプ（既定 3=Hall1。0:Room1〜5:Plate,6:Delay,7:PanningDelay）。
//! - `--reverb-time <N>` リバーブタイム（0-255、既定 128）。
//! - `--fb-two-sample`（実験用 EXPERIMENT(fb-2sample)）feedback 帰還を実機準拠の2サンプル平均にする。
//! - `--fb-scale-max <f>`（実験用 EXPERIMENT(fb-2sample)）feedback_to_scale の最大値を override（既定 1.8）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use smf2wav::{apply_reverb, normalize_peak, render_smf, write_wav_mono16, PatchBank, ReverbConfig};
use ym38x6_core::PresetFile;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("smf2wav: {msg}");
            eprintln!("usage: smf2wav <bank.38x6> <song.mid> [out.wav] [--sr <Hz>] [--tail <秒>] [--no-normalize]");
            eprintln!("       [--reverb-send <N>] [--reverb-type <0-7>] [--reverb-time <N>]");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    bank: PathBuf,
    song: PathBuf,
    out: PathBuf,
    sample_rate: f32,
    tail_secs: f32,
    normalize: bool,
    reverb: ReverbConfig,
    /// EXPERIMENT(fb-2sample): feedback を2サンプル平均帰還にする。
    feedback_two_sample: bool,
    /// EXPERIMENT(fb-2sample): feedback_to_scale 最大値 override（None=既定1.8）。
    feedback_scale_max: Option<f32>,
    /// 試聴の時短用: 出力をこの秒数（テール込み）で打ち切る（None=全長）。
    max_secs: Option<f32>,
    /// EXPERIMENT(max-voices): 同時発音数上限を上書きする（None=エンジン既定48）。
    max_voices: Option<usize>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut sample_rate: f32 = 44_100.0;
    let mut tail_secs: f32 = 2.0;
    let mut normalize = true;
    let mut reverb = ReverbConfig::default();
    let mut feedback_two_sample = false;
    let mut feedback_scale_max: Option<f32> = None;
    let mut max_secs: Option<f32> = None;
    let mut max_voices: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--reverb-send" => {
                let v = args.get(i + 1).ok_or("--reverb-send に値がありません")?;
                reverb.send = v.parse::<u8>().map_err(|_| format!("--reverb-send の値が不正(0-255): {v}"))?;
                i += 2;
            }
            "--reverb-type" => {
                let v = args.get(i + 1).ok_or("--reverb-type に値がありません")?;
                let t = v.parse::<u8>().map_err(|_| format!("--reverb-type の値が不正(0-7): {v}"))?;
                if t > 7 {
                    return Err(format!("--reverb-type は 0〜7 で指定してください: {v}"));
                }
                reverb.reverb_type = t;
                i += 2;
            }
            "--reverb-time" => {
                let v = args.get(i + 1).ok_or("--reverb-time に値がありません")?;
                reverb.time = v.parse::<u8>().map_err(|_| format!("--reverb-time の値が不正(0-255): {v}"))?;
                i += 2;
            }
            "--sr" => {
                let v = args.get(i + 1).ok_or("--sr に値がありません")?;
                sample_rate = v.parse::<f32>().map_err(|_| format!("--sr の値が不正: {v}"))?;
                if sample_rate <= 0.0 {
                    return Err(format!("--sr は正の値を指定してください: {v}"));
                }
                i += 2;
            }
            "--tail" => {
                let v = args.get(i + 1).ok_or("--tail に値がありません")?;
                tail_secs = v.parse::<f32>().map_err(|_| format!("--tail の値が不正: {v}"))?;
                if tail_secs < 0.0 {
                    return Err(format!("--tail は0以上を指定してください: {v}"));
                }
                i += 2;
            }
            "--no-normalize" => {
                normalize = false;
                i += 1;
            }
            "--max-secs" => {
                let v = args.get(i + 1).ok_or("--max-secs に値がありません")?;
                let s = v.parse::<f32>().map_err(|_| format!("--max-secs の値が不正: {v}"))?;
                if s <= 0.0 {
                    return Err(format!("--max-secs は正の値を指定してください: {v}"));
                }
                max_secs = Some(s);
                i += 2;
            }
            // EXPERIMENT(fb-2sample): feedback 帰還方式の測定用フラグ。
            "--fb-two-sample" => {
                feedback_two_sample = true;
                i += 1;
            }
            "--fb-scale-max" => {
                let v = args.get(i + 1).ok_or("--fb-scale-max に値がありません")?;
                let m = v.parse::<f32>().map_err(|_| format!("--fb-scale-max の値が不正: {v}"))?;
                if m <= 0.0 {
                    return Err(format!("--fb-scale-max は正の値を指定してください: {v}"));
                }
                feedback_scale_max = Some(m);
                i += 2;
            }
            // EXPERIMENT(max-voices): 同時発音数上限のA/B計測用。
            "--max-voices" => {
                let v = args.get(i + 1).ok_or("--max-voices に値がありません")?;
                let n = v.parse::<usize>().map_err(|_| format!("--max-voices の値が不正: {v}"))?;
                if n == 0 {
                    return Err(format!("--max-voices は1以上を指定してください: {v}"));
                }
                max_voices = Some(n);
                i += 2;
            }
            _ => {
                positional.push(&args[i]);
                i += 1;
            }
        }
    }
    if positional.len() < 2 {
        return Err("<bank.38x6> と <song.mid> の2引数が必要です".to_string());
    }
    let bank = PathBuf::from(positional[0]);
    let song = PathBuf::from(positional[1]);
    let out = if positional.len() >= 3 {
        PathBuf::from(positional[2])
    } else {
        let stem = song.file_stem().and_then(|s| s.to_str()).unwrap_or("song");
        song.parent().unwrap_or(Path::new(".")).join(format!("{stem}.wav"))
    };
    Ok(Args {
        bank,
        song,
        out,
        sample_rate,
        tail_secs,
        normalize,
        reverb,
        feedback_two_sample,
        feedback_scale_max,
        max_secs,
        max_voices,
    })
}

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    let json = std::fs::read_to_string(&args.bank)
        .map_err(|e| format!("{}: {e}", args.bank.display()))?;
    let file = PresetFile::from_json(&json)
        .map_err(|e| format!("{} のパースに失敗: {e}", args.bank.display()))?;
    let bank = PatchBank::from_preset_file(&file)?;

    // EXPERIMENT(fb-2sample): フィードバック帰還方式の切替（プロセスグローバル、既定は1サンプル帰還）。
    ym38x6_core::set_feedback_two_sample(args.feedback_two_sample);
    ym38x6_core::set_feedback_scale_max(args.feedback_scale_max);

    let smf_data = std::fs::read(&args.song)
        .map_err(|e| format!("{}: {e}", args.song.display()))?;

    let mut buf =
        render_smf(&smf_data, &bank, args.sample_rate, args.tail_secs, args.max_secs, args.max_voices)?;
    // マスターリバーブ（send>0 のときのみ）。DAW での聴感を再現する後段適用。
    apply_reverb(&mut buf, 1, args.sample_rate, &args.reverb);
    if args.normalize {
        normalize_peak(&mut buf, 0.5);
    }

    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("出力ディレクトリ作成に失敗: {e}"))?;
        }
    }
    write_wav_mono16(&args.out, &buf, args.sample_rate as u32)
        .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", args.out.display()))?;
    println!(
        "レンダリング: {} ({:.1}秒)",
        args.out.display(),
        buf.len() as f32 / args.sample_rate
    );
    Ok(())
}
