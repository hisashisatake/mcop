//! wavetest — OPZ準拠8波形をFMのキャリア/モジュレーターに使ったときの
//! 音色変化を試聴で確認するツール（op505向け）。
//!
//! 9系統の基本音色（pure/ピアノ/E.ピアノ/シンセベース/リード/ブラス/ベル/オルガン/プラック）+
//! TimeEgネイティブデモ2系統（多段リリース/非単調EG。ループGain FGゲートは別枠）に対し、
//! ビルトイン32波形（0-7サイン/8-15ノコギリ/16-23矩形/24-31三角）をオペレーター全体に
//! 適用したバリエーションを作り、`<output_dir>/wav/NN_<音色>_<波形>.wav` と
//! `<output_dir>/b<bank>.op505` を書き出す。
//!
//! 使い方:
//! ```text
//! wavetest <output_dir> [--bank <N>] [--note <C/D/...>] [--octave <N>] [--on <秒>] [--release <秒>] [--timbres <名前,...>]
//! ```
//! - 既定: bank=1 / C4 / on=1.2秒 + リリース3.0秒 / 全音色 + フィルターデモ。
//! - `--timbres pure,pluck` 等で基本音色を絞ると検証が速い（指定時はフィルターデモを省略）。
//!
//! 波形はモジュレーターにも適用される（FM下での倍音変化が本ツールの主目的のため、
//! キャリアだけでなく全オペレーターに同じ波形を割り当てる）。
//!
//! 由来: ym38x6/tools/wavetest4x6/src/main.rs（コミット ef3d309 時点の複製、2026-08-13）。
//! デフォーク後のop505ツール群向け複製（fork-on-write）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use op505_core::{Op505PresetEntry, Op505PresetFile};
use sound_core::Vco;
use wavetest::{
    base_timbres, build_filter_demo, expand_voices, filter_demos, note_to_freq, note_to_semitone,
    BaseTimbre, Voice, WAVE_VARIANTS,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("wavetest: {msg}");
            eprintln!("usage: wavetest <output_dir> [--bank <N>] [--note <C..B>] [--octave <N>] [--on <秒>] [--release <秒>] [--timbres pure,pluck,...]");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (output_dir, bank, override_freq, on_secs, release_secs, timbres) = parse_args(args)?;

    let all_bases: Vec<BaseTimbre> = base_timbres();
    // --timbres 指定時はその基本音色名のみに絞る（検証用の時短）。
    let bases: Vec<BaseTimbre> = match &timbres {
        Some(names) => {
            for n in names {
                if !all_bases.iter().any(|b| b.name == n) {
                    return Err(format!(
                        "--timbres の不明な音色名: {n}（既知: {}）",
                        all_bases.iter().map(|b| b.name).collect::<Vec<_>>().join(",")
                    ));
                }
            }
            all_bases.into_iter().filter(|b| names.iter().any(|n| n == b.name)).collect()
        }
        None => all_bases,
    };
    let mut voices = expand_voices(&bases, override_freq);
    let grid_count = voices.len();

    // フィルター入りデモ（ノコギリ）。--timbres で絞り込んだ検証時はスキップする。
    let demos = if timbres.is_some() { Vec::new() } else { filter_demos() };
    for (i, demo) in demos.iter().enumerate() {
        let program = (grid_count + i) as u8;
        voices.push(build_filter_demo(&bases, demo, program, override_freq));
    }

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("出力ディレクトリ作成に失敗: {}: {e}", output_dir.display()))?;

    render_wavs(&voices, &output_dir, on_secs, release_secs)?;

    let presets = voices.iter().map(|v| Op505PresetEntry { program: v.program, name: v.name.clone(), patch: v.patch }).collect();
    let file = Op505PresetFile::Presets { bank, presets };
    let path = output_dir.join(format!("b{bank}.op505"));
    let json = file.to_json().map_err(|e| format!("JSONシリアライズに失敗: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("書き込みに失敗: {}: {e}", path.display()))?;

    println!("書き出し: {} ({} 音色)", path.display(), voices.len());
    println!(
        "完了: {} 系統 × {} 波形 ({}) + フィルターデモ {} = {} 音色",
        bases.len(),
        WAVE_VARIANTS.len(),
        grid_count,
        demos.len(),
        voices.len()
    );
    Ok(())
}

/// 戻り値の3つ目は試聴周波数の上書き指定（`--note`/`--octave` のどちらかが指定された場合のみ
/// `Some`。未指定なら `None` で、音色ごとの基準音を使う）。
type ParsedArgs = (PathBuf, u16, Option<f32>, f32, f32, Option<Vec<String>>);

fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut bank: u16 = 1;
    let mut octave: Option<i32> = None;
    let mut note: Option<String> = None;
    let mut on_secs: f32 = 1.2;
    let mut release_secs: f32 = 3.0;
    let mut timbres: Option<Vec<String>> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bank" => {
                let v = args.get(i + 1).ok_or("--bank に値がありません")?;
                bank = v.parse().map_err(|_| format!("--bank の値が不正: {v}"))?;
                i += 2;
            }
            "--octave" => {
                let v = args.get(i + 1).ok_or("--octave に値がありません")?;
                octave = Some(v.parse().map_err(|_| format!("--octave の値が不正: {v}"))?);
                i += 2;
            }
            "--note" => {
                let v = args.get(i + 1).ok_or("--note に値がありません")?;
                note_to_semitone(v)?; // バリデーション
                note = Some(v.to_string());
                i += 2;
            }
            "--on" => {
                let v = args.get(i + 1).ok_or("--on に値がありません")?;
                on_secs = v.parse().map_err(|_| format!("--on の値が不正: {v}"))?;
                if on_secs <= 0.0 {
                    return Err(format!("--on は正の値を指定してください: {v}"));
                }
                i += 2;
            }
            "--release" => {
                let v = args.get(i + 1).ok_or("--release に値がありません")?;
                release_secs = v.parse().map_err(|_| format!("--release の値が不正: {v}"))?;
                if release_secs < 0.0 {
                    return Err(format!("--release は0以上の値を指定してください: {v}"));
                }
                i += 2;
            }
            "--timbres" => {
                let v = args.get(i + 1).ok_or("--timbres に値がありません（例: pure,pluck,brass）")?;
                timbres = Some(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
                i += 2;
            }
            _ => {
                positional.push(&args[i]);
                i += 1;
            }
        }
    }
    if positional.len() != 1 {
        return Err("出力ディレクトリの1引数が必要です".to_string());
    }
    // --note / --octave のどちらかが指定されたら全音色をその音程で上書きする。
    let override_freq = match (note, octave) {
        (None, None) => None,
        (n, o) => Some(note_to_freq(o.unwrap_or(4), n.as_deref().unwrap_or("C"))?),
    };
    Ok((PathBuf::from(positional[0]), bank, override_freq, on_secs, release_secs, timbres))
}

/// 各音色を WAV（mono 44.1kHz 16bit）へレンダリングする。各音色は自身の試聴周波数
/// （[`Voice::freq`]）で鳴らす。ノコギリ全Op等のホットな音色のクリップを抑えるため、
/// 書き出し時に控えめなマスターゲイン(0.7)を掛ける。
fn render_wavs(voices: &[Voice], output_dir: &Path, on_secs: f32, release_secs: f32) -> Result<(), String> {
    const SR: f32 = 44_100.0;
    const MASTER_GAIN: f32 = 0.7;
    let wav_dir = output_dir.join("wav");
    std::fs::create_dir_all(&wav_dir).map_err(|e| format!("wavディレクトリ作成に失敗: {e}"))?;

    for (idx, v) in voices.iter().enumerate() {
        let mut engine = op505_core::Op505Engine::new(SR);
        engine.set_patch(v.patch);
        engine.note_on(0, v.freq, 110);

        let on = (SR * on_secs) as usize;
        let off = (SR * release_secs) as usize;
        let mut samples = vec![0.0f32; on];
        engine.render(&mut samples, 1);
        engine.note_off(0);
        let mut tail = vec![0.0f32; off];
        engine.render(&mut tail, 1);
        samples.extend_from_slice(&tail);
        for s in samples.iter_mut() {
            *s *= MASTER_GAIN;
        }

        let file_name = format!("{idx:02}_{}.wav", v.name.replace(' ', "_"));
        let path = wav_dir.join(&file_name);
        op505_tools::wav::write_wav_mono16(&path, &samples, SR as u32)
            .map_err(|e| format!("WAV書き込みに失敗: {}: {e}", path.display()))?;
    }
    println!("WAV書き出し: {} に {} 音色", wav_dir.display(), voices.len());
    Ok(())
}
