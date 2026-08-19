//! `.op505`バンクの各音色を同一条件でレンダリングし、WAVへ書き出す聴き比べツール。
//!
//! CHIP LFO退役（memory `project_chip_lfo_retirement_investigation.md`）の
//! 移設前バンクと移設後バンクを**同じエンジン・同じ音符**で鳴らして比較するために作った。
//! 変換器側でWAVを出す（`opz2op505 --wav`）と移設前後でバイナリを切り替える必要があるが、
//! こちらはパッチデータだけを差し替えられるので、差分がパッチ由来であることが保証される。
//!
//! 音色名でファイル名を作るので、2つの出力ディレクトリを並べれば同名ファイル同士がA/Bになる。
//!
//! 実行: cargo run -p op505-core --example bank_audition -- <bank.op505> <出力ディレクトリ> [音色名フィルタ...]

use std::path::Path;

use op505_core::{Op505Engine, Op505Patch, Op505PresetFile};
use sound_core::Vco;

const SAMPLE_RATE: f32 = 44100.0;
/// LFO/AMの周期が数回まわる長さ。移設で問題になるのは「揺れ始めるまでの時間」なので、
/// アタック直後を聴き取れるだけの余裕を取る。
const HOLD_SECS: f32 = 3.5;
const RELEASE_TAIL_SECS: f32 = 1.5;
/// A3。低すぎるとAMの谷が聴き取りにくく、高すぎるとKSRでEG速度が変わる。
const NOTE_FREQ: f32 = 220.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let bank_path = args.next().expect("使い方: bank_audition <bank.op505> <出力ディレクトリ> [音色名...]");
    let out_dir = args.next().expect("出力ディレクトリを指定してください");
    let filters: Vec<String> = args.map(|s| s.to_lowercase()).collect();

    let out_dir = Path::new(&out_dir);
    std::fs::create_dir_all(out_dir).expect("出力ディレクトリ作成失敗");

    let json = std::fs::read_to_string(&bank_path).expect("バンク読み込み失敗");
    let file: Op505PresetFile = serde_json::from_str(&json).expect("バンクのJSONパース失敗");
    let entries = match &file {
        Op505PresetFile::Presets { presets, .. } => presets,
        Op505PresetFile::Programs { programs, .. } => programs,
    };

    let mut count = 0;
    for entry in entries {
        let lower = entry.name.to_lowercase();
        if !filters.is_empty() && !filters.iter().any(|f| lower.contains(f.as_str())) {
            continue;
        }
        let safe: String = entry
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let path = out_dir.join(format!("{:03}_{safe}.wav", entry.program));
        write_wav(&path, &render(entry.patch));
        println!("  {}", path.display());
        count += 1;
    }
    println!("[bank_audition] {count} 音色を書き出した ({})", bank_path);
}

fn render(patch: Op505Patch) -> Vec<f32> {
    let mut engine = Op505Engine::new(SAMPLE_RATE);
    engine.set_patch(patch);
    engine.note_on(0, NOTE_FREQ, 100);
    let hold = (HOLD_SECS * SAMPLE_RATE) as usize;
    let tail = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = vec![0.0f32; hold + tail];
    engine.render(&mut out[..hold], 1);
    engine.note_off(0);
    engine.render(&mut out[hold..], 1);
    out
}

fn write_wav(path: &Path, samples: &[f32]) {
    let mut bytes = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("WAV書き込み失敗");
}
