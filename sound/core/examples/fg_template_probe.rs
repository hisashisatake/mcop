//! FGテンプレート波形（TRIANGLE/SAW UP/SAW DOWN/SQUARE）の周期精度を、実装済みの
//! `sound_core::time_eg` API（`TimeEgParams::texture`/`base_freq`/`time_eg_speed_scale`/
//! `template_period_seconds`）を使って実測する探査ツール。
//!
//! ステップ1(time=0真0秒化)完了時点では、C案（カノニカルSTAGE表＋
//! `speed_scale = base_freq_hz × template_period_seconds`）を手組みで先取りして測っていたが、
//! ステップ2(free_rate/rate_range/base_freqとTEXTUREテンプレート波形の実装)完了後は
//! 実際のAPIをそのまま使う形に書き換えた（手組みの理論値と実装の実際の挙動が一致することの確認）。
//!
//! 実行: cargo run -p sound-core --release --example fg_template_probe

use sound_core::time_eg::{
    template_period_seconds, time_eg_speed_scale, time_to_seconds, TimeEg, TimeEgParams, TimeStage,
    TEXTURE_SAW_DOWN, TEXTURE_SAW_UP, TEXTURE_SQUARE, TEXTURE_TRIANGLE,
};

fn texture_name(texture: u8) -> &'static str {
    match texture {
        TEXTURE_TRIANGLE => "TRIANGLE",
        TEXTURE_SAW_UP => "SAW UP",
        TEXTURE_SAW_DOWN => "SAW DOWN",
        TEXTURE_SQUARE => "SQUARE",
        _ => "?",
    }
}

/// ループ区間の段数（1サンプル1段の進行上限に効く、参考表示用）。
fn loop_stages(texture: u8) -> usize {
    match texture {
        TEXTURE_SQUARE => 4,
        _ => 2,
    }
}

/// レベルが0.5を上向きに横切るサンプル位置（線形補間つき）を集める。
fn rising_crossings(samples: &[f32]) -> Vec<f64> {
    let mut out = Vec::new();
    for i in 1..samples.len() {
        let (a, b) = (samples[i - 1], samples[i]);
        if a < 0.5 && b >= 0.5 {
            let frac = if (b - a).abs() < 1e-12 { 0.0 } else { (0.5 - a) / (b - a) };
            out.push((i - 1) as f64 + frac as f64);
        }
    }
    out
}

struct Measurement {
    measured_hz: f64,
    error_pct: f64,
    samples_per_cycle: f64,
    cycles_found: usize,
}

fn measure(texture: u8, sample_rate: f32, target_hz: f32) -> Measurement {
    // base_freq(0〜255)から狙ったHzちょうどを作るのは丸めが乗るため、ここではbase_freq_hzを
    // 経由せず`time_eg_speed_scale`と同じ式を直接使う（実装の式そのものを検証する）。
    let params = TimeEgParams { texture, ..Default::default() };
    let speed_scale = target_hz * template_period_seconds(texture);

    let mut eg = TimeEg::new();
    eg.note_on();

    let want_cycles = 30.0;
    let total = ((want_cycles / target_hz as f64) * sample_rate as f64).max(4800.0) as usize;
    let mut samples = Vec::with_capacity(total);
    for _ in 0..total {
        samples.push(eg.tick(sample_rate, params, speed_scale));
    }

    let crossings = rising_crossings(&samples);
    let skip = 5.min(crossings.len().saturating_sub(2));
    let used = &crossings[skip..];
    if used.len() < 2 {
        return Measurement { measured_hz: 0.0, error_pct: f64::NAN, samples_per_cycle: 0.0, cycles_found: used.len() };
    }
    let span = used[used.len() - 1] - used[0];
    let cycles = (used.len() - 1) as f64;
    let samples_per_cycle = span / cycles;
    let measured_hz = sample_rate as f64 / samples_per_cycle;
    let error_pct = (measured_hz - target_hz as f64) / target_hz as f64 * 100.0;
    Measurement { measured_hz, error_pct, samples_per_cycle, cycles_found: used.len() }
}

/// カノニカル表の末尾へ「リリース段」を連結したparamsを作る（`template_params`が元paramsの
/// `release_point+1..stage_count`を末尾へ連結する挙動を、テストの元paramsとして与える）。
fn params_with_release(texture: u8, release_time: u8) -> TimeEgParams {
    let mut stages = [TimeStage::default(); sound_core::time_eg::MAX_STAGES];
    stages[1] = TimeStage { time: release_time, level: 0, curve: 0 };
    TimeEgParams { stages, stage_count: 2, release_point: 0, texture, ..Default::default() }
}

/// note_off後にidleへ到達するまでの実所要秒数を測る。
fn measure_release_seconds(texture: u8, sample_rate: f32, target_hz: f32, release_time: u8) -> f64 {
    let params = params_with_release(texture, release_time);
    let speed_scale = target_hz * template_period_seconds(texture);

    let mut eg = TimeEg::new();
    eg.note_on();
    let warm = ((3.0 / target_hz as f64) * sample_rate as f64).max(64.0) as usize;
    for _ in 0..warm {
        eg.tick(sample_rate, params, speed_scale);
    }
    eg.note_off();

    let limit = (sample_rate as f64 * 120.0) as usize;
    let mut n = 0usize;
    while !eg.is_idle() && n < limit {
        eg.tick(sample_rate, params, speed_scale);
        n += 1;
    }
    if n >= limit {
        return f64::INFINITY;
    }
    n as f64 / sample_rate as f64
}

fn main() {
    let textures = [TEXTURE_TRIANGLE, TEXTURE_SAW_UP, TEXTURE_SAW_DOWN, TEXTURE_SQUARE];
    let targets: [f32; 9] = [0.15625, 1.0, 5.0, 20.0, 50.0, 100.0, 160.0, 640.0, 2560.0];

    for &sample_rate in &[24_000.0f32, 48_000.0f32] {
        println!("\n================ sample_rate = {sample_rate} Hz ================");
        println!("1サンプル1段の進行上限: TRIANGLE/SAW={:.0}Hz, SQUARE={:.0}Hz",
            sample_rate / 2.0, sample_rate / 4.0);
        for &texture in &textures {
            println!("\n--- {} (1周={:.4}s, ループ{}段) ---",
                texture_name(texture), template_period_seconds(texture), loop_stages(texture));
            println!("{:>10} | {:>12} | {:>9} | {:>12} | {:>7}",
                "目標Hz", "実測Hz", "誤差%", "サンプル/周", "周期数");
            for &target in &targets {
                let m = measure(texture, sample_rate, target);
                let flag = if m.error_pct.is_nan() {
                    "  <-- 測定不能"
                } else if m.error_pct.abs() > 5.0 {
                    "  <-- 破綻"
                } else if m.error_pct.abs() > 0.5 {
                    "  <-- 要注意"
                } else {
                    ""
                };
                println!("{:>10.4} | {:>12.4} | {:>+9.3} | {:>12.2} | {:>7}{}",
                    target, m.measured_hz, m.error_pct, m.samples_per_cycle, m.cycles_found, flag);
            }
        }
    }

    // -----------------------------------------------------------------------
    // リリース区間のスケール分離の実測（`time_eg_speed_scale`＋`self.releasing`ガード）
    // -----------------------------------------------------------------------
    println!("\n\n================ リリース区間の伸縮 (24000 Hz) ================");
    let release_time: u8 = 120;
    let nominal = time_to_seconds(release_time);
    println!("連結したリリース段: time={release_time} = {nominal:.4}s（元paramsの値そのまま）\n");
    println!("{:>10} | {:>12} | {:>14} | {:>10}", "基準Hz", "speed_scale", "実測リリース秒", "公称比");
    for &target in &[0.15625f32, 1.0, 5.0, 20.0, 160.0] {
        let scale = target * template_period_seconds(TEXTURE_TRIANGLE);
        let secs = measure_release_seconds(TEXTURE_TRIANGLE, 24_000.0, target, release_time);
        println!("{:>10.4} | {:>12.5} | {:>14.4} | {:>9.2}x", target, scale, secs, secs / nominal as f64);
    }

    // -----------------------------------------------------------------------
    // 実際のtime_eg_speed_scale経由（base_freq/free_rateパラメーターそのもの）での確認
    // -----------------------------------------------------------------------
    println!("\n\n================ time_eg_speed_scale経由 (44100 Hz, TRIANGLE) ================");
    println!("{:>10} | {:>10} | {:>12} | {:>12} | {:>9}",
        "base_freq", "free_rate", "speed_scale", "実測Hz", "誤差%");
    for &base_freq in &[0u8, 64, 128, 192, 255] {
        let params = TimeEgParams { texture: TEXTURE_TRIANGLE, base_freq, ..Default::default() };
        let speed_scale = time_eg_speed_scale(&params, 0.0);
        let target_hz = sound_core::time_eg::base_freq_hz(base_freq);

        let mut eg = TimeEg::new();
        eg.note_on();
        let want_cycles = 20.0;
        let total = ((want_cycles / target_hz as f64) * 44_100.0).max(4800.0) as usize;
        let mut samples = Vec::with_capacity(total);
        for _ in 0..total {
            samples.push(eg.tick(44_100.0, params, speed_scale));
        }
        let crossings = rising_crossings(&samples);
        let skip = 3.min(crossings.len().saturating_sub(2));
        let used = &crossings[skip..];
        let measured_hz = if used.len() >= 2 {
            let span = used[used.len() - 1] - used[0];
            44_100.0 / (span / (used.len() - 1) as f64)
        } else {
            f64::NAN
        };
        let error_pct = (measured_hz - target_hz as f64) / target_hz as f64 * 100.0;
        println!("{:>10} | {:>10} | {:>12.5} | {:>12.4} | {:>+9.3}",
            base_freq, 128, speed_scale, measured_hz, error_pct);
    }
}
