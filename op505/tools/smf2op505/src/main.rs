//! smf2op505 — `.op505` 音色バンクで標準MIDIファイル（SMF）を再生し、WAV へ書き出す。
//!
//! 使い方:
//! ```text
//! smf2op505 <bank.op505> <song.mid> [out.wav] [--sr <Hz>] [--tail <秒>] [--no-normalize]
//!           [--reverb-send <N>] [--reverb-type <0-7>] [--reverb-time <N>]
//!           [--drum-bank <kit.op505>]...
//! ```
//! - `out.wav` 省略時は `<song>` と同じディレクトリに `<songの拡張子なし名>.wav` を出力する。
//! - プログラムチェンジ番号 = `.op505` のプログラム番号で音色を選ぶ
//!   （未定義番号は直下の最も近い定義済み番号へフォールバック）。
//! - `--sr` 出力サンプルレート（既定 44100）。
//! - `--tail` ノートオフ後の残響を伸ばす秒数（既定 2.0）。
//! - `--no-normalize` ピーク正規化（-6dBFS）を無効化する。
//! - `--max-secs <秒>` 出力をこの秒数（テール込み）で打ち切る（試聴の時短用）。
//! - `--reverb-send <N>` マスターリバーブセンド（0-255、既定 0=ドライ）。DAW で
//!   全chに掛けていたリバーブ（CC91 相当）を再現する診断用。0 のとき従来どおり完全ドライ。
//!   これは SMF 内蔵のマスターエフェクト（CC91/93・NRPN(0,2)〜(0,8) で駆動する
//!   `MasterEffects`、render 側で適用）とは**独立した後段の診断用リバーブ**（op505-tools::fx）で、
//!   SMF にエフェクト系 CC/NRPN が無い通常の曲では既定 0 のまま二重掛けにならない。
//! - `--reverb-type <0-7>` リバーブタイプ（既定 3=Hall1。0:Room1〜5:Plate,6:Delay,7:PanningDelay）。
//! - `--reverb-time <N>` リバーブタイム（0-255、既定 128）。
//! - `--max-voices <N>` 同時発音数上限を上書きする（実験用A/B計測、既定はエンジン既定）。
//! - `--drum-bank <kit.op505>` GM2リズムキットバンク（複数回指定可、`Op505PresetBank::merge_file`で
//!   重ねる）。ファイル内の`"bank"`は`15360 + キット番号`（15360 = Bank Select MSB=120 ×
//!   128 + キット0）で宣言すること。未指定時はリズムチャンネル機能を完全に無効化する
//!   （`op505_midi::rhythm`参照）。指定してもリズムバンク範囲(15360〜15487)に1件も
//!   エントリーが無ければエラー終了する（宣言忘れによる無音を起動時に検出するため）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use op505_core::{Op505PresetBank, Op505PresetFile};
use op505_midi::RHYTHM_BANK_RANGE;
use smf2op505::{
    apply_reverb, normalize_peak, render_smf_with_drums, write_wav_mono16, PatchBank, ReverbConfig,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("smf2op505: {msg}");
            eprintln!("usage: smf2op505 <bank.op505> <song.mid> [out.wav] [--sr <Hz>] [--tail <秒>] [--no-normalize]");
            eprintln!("       [--reverb-send <N>] [--reverb-type <0-7>] [--reverb-time <N>] [--max-voices <N>]");
            eprintln!("       [--drum-bank <kit.op505>]...");
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
    /// 試聴の時短用: 出力をこの秒数（テール込み）で打ち切る（None=全長）。
    max_secs: Option<f32>,
    /// EXPERIMENT(max-voices): 同時発音数上限を上書きする（None=エンジン既定）。
    max_voices: Option<usize>,
    /// GM2リズムキットバンクファイル（複数回指定可、順にmerge_fileで重ねる）。
    drum_banks: Vec<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut sample_rate: f32 = 44_100.0;
    let mut tail_secs: f32 = 2.0;
    let mut normalize = true;
    let mut reverb = ReverbConfig::default();
    let mut max_secs: Option<f32> = None;
    let mut max_voices: Option<usize> = None;
    let mut drum_banks: Vec<PathBuf> = Vec::new();
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
            "--drum-bank" => {
                let v = args.get(i + 1).ok_or("--drum-bank に値がありません")?;
                drum_banks.push(PathBuf::from(v));
                i += 2;
            }
            _ => {
                positional.push(&args[i]);
                i += 1;
            }
        }
    }
    if positional.len() < 2 {
        return Err("<bank.op505> と <song.mid> の2引数が必要です".to_string());
    }
    let bank = PathBuf::from(positional[0]);
    let song = PathBuf::from(positional[1]);
    let out = if positional.len() >= 3 {
        PathBuf::from(positional[2])
    } else {
        let stem = song.file_stem().and_then(|s| s.to_str()).unwrap_or("song");
        song.parent().unwrap_or(Path::new(".")).join(format!("{stem}.wav"))
    };
    Ok(Args { bank, song, out, sample_rate, tail_secs, normalize, reverb, max_secs, max_voices, drum_banks })
}

fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;

    let json = std::fs::read_to_string(&args.bank).map_err(|e| format!("{}: {e}", args.bank.display()))?;
    let file =
        Op505PresetFile::from_json(&json).map_err(|e| format!("{} のパースに失敗: {e}", args.bank.display()))?;
    let bank = PatchBank::from_preset_file(&file)?;

    // GM2リズムキットバンク（複数回指定可）。ファイルごとにパースしてmerge_fileで重ねる。
    let drums = if args.drum_banks.is_empty() {
        None
    } else {
        let mut drums = Op505PresetBank::default();
        for path in &args.drum_banks {
            let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let file = Op505PresetFile::from_json(&json)
                .map_err(|e| format!("{} のパースに失敗: {e}", path.display()))?;
            drums.merge_file(file);
        }
        if !drums.has_bank_in(RHYTHM_BANK_RANGE) {
            return Err(format!(
                "--drum-bank で指定したファイルにリズムバンク({}〜{})のエントリーが1件もありません。\
                 .op505ファイルの\"bank\"を15360+キット番号（例: kit0なら15360）で宣言してください。",
                RHYTHM_BANK_RANGE.start(),
                RHYTHM_BANK_RANGE.end()
            ));
        }
        for path in &args.drum_banks {
            eprintln!("smf2op505: ドラムキット読み込み: {}", path.display());
        }
        Some(drums)
    };

    let smf_data = std::fs::read(&args.song).map_err(|e| format!("{}: {e}", args.song.display()))?;

    let mut buf = render_smf_with_drums(
        &smf_data,
        &bank,
        drums.as_ref(),
        args.sample_rate,
        args.tail_secs,
        args.max_secs,
        args.max_voices,
    )?;
    // マスターリバーブ（send>0 のときのみ）。DAW での聴感を再現する後段適用。
    apply_reverb(&mut buf, 1, args.sample_rate, &args.reverb);
    if args.normalize {
        normalize_peak(&mut buf, 0.5);
    }

    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("出力ディレクトリ作成に失敗: {e}"))?;
        }
    }
    write_wav_mono16(&args.out, &buf, args.sample_rate as u32)
        .map_err(|e| format!("WAV 書き込みに失敗: {}: {e}", args.out.display()))?;
    println!("レンダリング: {} ({:.1}秒)", args.out.display(), buf.len() as f32 / args.sample_rate);
    Ok(())
}
