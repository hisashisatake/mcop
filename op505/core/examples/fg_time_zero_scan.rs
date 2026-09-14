//! `.op505`バンク内の全音色を走査し、TimeEg（OP1〜4 EG／Pitch・Cutoff・Gain FG）の
//! 保持区間・リリース区間に`time==0`の段を持つものを機械的に列挙する。
//!
//! `time=0`真0秒化（現状は1サンプル消費＋経過時間破棄）の修正前に「差分が出るべき音色」の
//! 予測集合を作るための探査ツール。予測集合と修正後の実測差分を突き合わせることで、
//! `time=0`以外の経路を壊していないかを厳密に確認する。
//!
//! 実行: cargo run -p op505-core --release --example fg_time_zero_scan -- <bank1.op505> [bank2.op505 ...]

use op505_core::{Op505Patch, Op505PresetFile};
use sound_core::TimeEgParams;

/// 保持区間（`0..=release_point`）とリリース区間（`release_point+1..stage_count`）に分けて
/// `time==0`の段のindexを返す（`clamp_stage_count`相当の下限処理も併せて行う）。
fn zero_time_stage_indices(eg: &TimeEgParams) -> (Vec<usize>, Vec<usize>) {
    let stage_count = (eg.stage_count as usize).max(1).min(sound_core::MAX_STAGES);
    let release_point = (eg.release_point as usize).min(stage_count - 1);

    let hold: Vec<usize> = (0..=release_point).filter(|&i| eg.stages[i].time == 0).collect();
    let release: Vec<usize> =
        (release_point + 1..stage_count).filter(|&i| eg.stages[i].time == 0).collect();
    (hold, release)
}

fn report_channel(prefix: &str, name: &str, eg: &TimeEgParams, hits: &mut Vec<String>) {
    let (hold, release) = zero_time_stage_indices(eg);
    if !hold.is_empty() {
        hits.push(format!("{prefix}{name} 保持区間 time=0 段: {hold:?}（stage_count={} release_point={} loop_enabled={}）",
            eg.stage_count, eg.release_point, eg.loop_enabled));
    }
    if !release.is_empty() {
        hits.push(format!("{prefix}{name} リリース区間 time=0 段: {release:?}（stage_count={} release_point={}）",
            eg.stage_count, eg.release_point));
    }
}

/// `stage_count==0`はUI側のFG無効化（`op505/core/src/lib.rs:684,715,785`）で、この場合
/// エンジンは`TimeEg::tick`自体を呼ばない（Pitch FG=0セント／Cutoff FG=基準そのまま／
/// Gain FG=透過1.0で即座に確定する）。したがってtime=0段があってもtime=0真0秒化の
/// 影響を一切受けない、真の偽陽性なので別枠（`disabled_hits`）へ分離する。
/// OP EGにはこのスキップが無い（`operator.rs:286`、idleでない限り常にtickを呼ぶ）。
fn scan_patch(patch: &Op505Patch, hits: &mut Vec<String>, disabled_hits: &mut Vec<String>) {
    for (i, op) in patch.operators.iter().enumerate() {
        report_channel("", &format!("OP{}", i + 1), &op.eg, hits);
    }
    for (name, eg) in [
        ("Pitch FG", &patch.channel.pitch_fg.eg),
        ("Cutoff FG", &patch.channel.cutoff_fg.eg),
        ("Gain FG", &patch.channel.gain_fg.eg),
    ] {
        if eg.stage_count == 0 {
            report_channel("[tickされずスキップ] ", name, eg, disabled_hits);
        } else {
            report_channel("", name, eg, hits);
        }
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("使い方: fg_time_zero_scan <bank1.op505> [bank2.op505 ...]");
        std::process::exit(1);
    }

    let mut total_files = 0usize;
    let mut total_entries = 0usize;
    let mut total_hit_entries = 0usize;
    let mut total_disabled_only_entries = 0usize;

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
        total_files += 1;
        let entries = match &file {
            Op505PresetFile::Presets { presets, .. } => presets,
            Op505PresetFile::Programs { programs, .. } => programs,
        };

        for entry in entries {
            total_entries += 1;
            let mut hits = Vec::new();
            let mut disabled_hits = Vec::new();
            scan_patch(&entry.patch, &mut hits, &mut disabled_hits);
            if !hits.is_empty() {
                total_hit_entries += 1;
                println!("[{path}] program={} name=\"{}\"", entry.program, entry.name);
                for h in &hits {
                    println!("    {h}");
                }
                for h in &disabled_hits {
                    println!("    {h}");
                }
            } else if !disabled_hits.is_empty() {
                total_disabled_only_entries += 1;
            }
        }
    }

    println!(
        "\n=== 集計: {total_files}ファイル中、{total_entries}音色中 {total_hit_entries}音色が予測集合（tick経路でtime=0を通過しうる） ==="
    );
    println!(
        "    （うち{total_disabled_only_entries}音色はFG無効化(stage_count=0)のtime=0のみで、tickされないため差分は出ないはず）"
    );
}
