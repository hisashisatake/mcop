//! `op505-core::operator::Operator::compute_env_amp`のキャッシュ条件
//! (`delta == self.cached_env_delta`という浮動小数点厳密一致)が実際にヒットするかを
//! 検証する診断用の使い捨て例。TimeEgが線形補間で一定速度のstageを進む間、連続する
//! サンプル間のlevel差分(delta)が丸め誤差により厳密に一致しなくなっていないかを見る。
//!
//! 実行: cargo run --release -p sound-core --example env_amp_cache_probe

use sound_core::{TimeEg, TimeEgParams, TimeStage};

const SAMPLE_RATE: f32 = 44100.0;

fn main() {
    // 十分に長い1段（1秒）で255→0へ線形に落ちるだけのシンプルなEG。
    let mut stages = [TimeStage::default(); sound_core::MAX_STAGES];
    stages[0] = TimeStage { time: sound_core::seconds_to_time(1.0), level: 0, curve: 0 };
    stages[1] = TimeStage { time: 0, level: 0, curve: 0 };
    let params = TimeEgParams {
        stages,
        stage_count: 2,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 1,
        ..TimeEgParams::default()
    };

    let mut eg = TimeEg::default();
    // 初期値255からスタートさせるためretrigger相当の状態にする。
    eg.tick(SAMPLE_RATE, params, 1.0); // 1回目でsegment_start/endが設定される

    let mut prev_level = eg.level();
    let mut prev_delta: Option<f32> = None;
    let mut total = 0usize;
    let mut delta_matches_prev = 0usize;
    let mut distinct_delta_count = std::collections::HashSet::new();

    for _ in 0..2000 {
        let level = eg.tick(SAMPLE_RATE, params, 1.0);
        let delta = level - prev_level;
        total += 1;
        if let Some(pd) = prev_delta {
            if delta == pd {
                delta_matches_prev += 1;
            }
        }
        distinct_delta_count.insert(delta.to_bits());
        prev_delta = Some(delta);
        prev_level = level;
    }

    println!("観測サンプル数: {total}");
    println!("前回と厳密一致したdeltaの回数: {delta_matches_prev} ({:.1}%)", delta_matches_prev as f32 / total as f32 * 100.0);
    println!("delta のユニークなビットパターン数: {}", distinct_delta_count.len());
    println!();
    println!("→ ユニーク数が多い(≒total)ほど、`delta == cached_env_delta`によるキャッシュは");
    println!("  ほぼ常にミスしていることになる。");
}
