//! `.op505`バンク内の全音色を`bank_audition`と同一条件でレンダリングし、音色ごとの
//! 出力波形をFNV-1aでハッシュ化する。`time=0`真0秒化の修正前後でこれを実行し、
//! `fg_time_zero_scan`が出した予測集合（time=0がtick経路を通る音色）と実測差分を
//! 突き合わせるためのベースライン取得ツール。
//!
//! 実行: cargo run -p op505-core --release --example fg_time_zero_render_hash -- <bank1.op505> [bank2.op505 ...] > baseline.txt

use op505_core::{Op505Engine, Op505Patch, Op505PresetFile};
use sound_core::Vco;

const SAMPLE_RATE: f32 = 44100.0;
const HOLD_SECS: f32 = 3.5;
const RELEASE_TAIL_SECS: f32 = 1.5;
const NOTE_FREQ: f32 = 220.0;

/// FNV-1a 64bit（`op505-tools::golden::fnv1a64`と同じアルゴリズムの複製）。
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
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

fn hash_samples(samples: &[f32]) -> u64 {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for &s in samples {
        bytes.extend_from_slice(&s.to_bits().to_le_bytes());
    }
    fnv1a64(&bytes)
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("使い方: fg_time_zero_render_hash <bank1.op505> [bank2.op505 ...]");
        std::process::exit(1);
    }

    let mut rows: Vec<(String, u8, String, u64)> = Vec::new();

    for path in &paths {
        let json = match std::fs::read_to_string(path) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("読み込み失敗 {path}: {e}");
                continue;
            }
        };
        let file: Op505PresetFile = match serde_json::from_str(&json) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("JSONパース失敗 {path}: {e}");
                continue;
            }
        };
        let entries = match &file {
            Op505PresetFile::Presets { presets, .. } => presets,
            Op505PresetFile::Programs { programs, .. } => programs,
        };

        // 同名ファイルが複数ディレクトリに存在するため（例: voices.op505）、キーは
        // 引数として渡されたパスそのものを使う（前後比較は同一コマンドで実行する前提）。
        let key = path.clone();

        for entry in entries {
            let samples = render(entry.patch);
            let hash = hash_samples(&samples);
            rows.push((key.clone(), entry.program, entry.name.clone(), hash));
        }
    }

    // ファイル名・program順に安定ソートしてから出力する（diffで見やすくするため）。
    rows.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));
    for (file, program, name, hash) in &rows {
        println!("{file}|{program}|{name}|{hash:016x}");
    }
    eprintln!("合計 {} 音色をレンダリングした", rows.len());
}
