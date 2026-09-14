//! FGテンプレート波形（TRIANGLE/SAW UP/SAW DOWN/SQUARE）の周期精度を実測する探査ツール。
//!
//! 計画中のC案（textureを保存値とし、内部でカノニカルSTAGE表を生成して既存TimeEg状態機械へ
//! 流す）を手組みで先取りし、`speed_scale = base_freq_hz × template_period_seconds`で
//! 正規化したときに実際に基準周波数どおりの周期が出るかを測る。
//!
//! 目的は「基準周波数の上限160Hzが妥当か」の判断材料を得ること。
//!
//! 実行: cargo run -p sound-core --release --example fg_template_probe

use sound_core::time_eg::{time_to_seconds, TimeEg, TimeEgParams, TimeStage, MAX_STAGES};

/// カノニカル表の公称TIME値。speed_scaleで正規化するため値自体は任意だが、
/// 極端に短い/長いと丸めやspeed_scaleの絶対値が極端になるため中庸を選ぶ。
const T: u8 = 100;
/// 導入段（lead）の公称TIME値。1周目の傾きを揃えるためのもので、周期には影響しない。
const LEAD: u8 = 50;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Template {
    Triangle,
    SawUp,
    SawDown,
    Square,
}

impl Template {
    fn name(self) -> &'static str {
        match self {
            Template::Triangle => "TRIANGLE",
            Template::SawUp => "SAW UP",
            Template::SawDown => "SAW DOWN",
            Template::Square => "SQUARE",
        }
    }

    /// カノニカル表の1周（ループ区間）の公称秒数。
    fn period_seconds(self) -> f32 {
        let t = time_to_seconds(T);
        match self {
            Template::Triangle | Template::Square => 2.0 * t,
            Template::SawUp | Template::SawDown => t,
        }
    }

    /// ループ区間の段数（1サンプル1段の進行上限に効く）。
    fn loop_stages(self) -> usize {
        match self {
            Template::Triangle | Template::SawUp | Template::SawDown => 2,
            Template::Square => 4,
        }
    }

    fn params(self) -> TimeEgParams {
        let mut stages = [TimeStage::default(); MAX_STAGES];
        let (count, loop_start, release_point) = match self {
            Template::Triangle => {
                stages[0] = TimeStage { time: LEAD, level: 255, curve: 0 };
                stages[1] = TimeStage { time: T, level: 0, curve: 0 };
                stages[2] = TimeStage { time: T, level: 255, curve: 0 };
                (3u8, 1u8, 2u8)
            }
            Template::SawUp => {
                stages[0] = TimeStage { time: LEAD, level: 255, curve: 0 };
                stages[1] = TimeStage { time: 0, level: 0, curve: 0 };
                stages[2] = TimeStage { time: T, level: 255, curve: 0 };
                (3, 1, 2)
            }
            Template::SawDown => {
                stages[0] = TimeStage { time: LEAD, level: 0, curve: 0 };
                stages[1] = TimeStage { time: 0, level: 255, curve: 0 };
                stages[2] = TimeStage { time: T, level: 0, curve: 0 };
                (3, 1, 2)
            }
            Template::Square => {
                stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
                stages[1] = TimeStage { time: T, level: 255, curve: 0 };
                stages[2] = TimeStage { time: 0, level: 0, curve: 0 };
                stages[3] = TimeStage { time: T, level: 0, curve: 0 };
                (4, 0, 3)
            }
        };
        TimeEgParams {
            stages,
            stage_count: count,
            loop_enabled: 1,
            loop_start,
            release_point,
            ..Default::default()
        }
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

fn measure(template: Template, sample_rate: f32, target_hz: f32) -> Measurement {
    let params = template.params();
    // 計画の式: speed_scale = base_freq_hz × template_period_seconds
    let speed_scale = target_hz * template.period_seconds();

    let mut eg = TimeEg::new();
    eg.note_on();

    // 30周分（最低4800サンプル）を回し、最初の5周はlead/立ち上がりの影響を避けて捨てる。
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

/// カノニカル表の末尾へ「リリース段」を連結したparamsを作る（計画の`template_params`が
/// 元paramsの`release_point+1..stage_count`を末尾へ連結する挙動を模す）。
fn params_with_release(template: Template, release_time: u8) -> TimeEgParams {
    let mut p = template.params();
    let idx = p.stage_count as usize;
    p.stages[idx] = TimeStage { time: release_time, level: 0, curve: 0 };
    p.stage_count += 1;
    p
}

/// note_off後にidleへ到達するまでの実所要秒数を測る。
fn measure_release_seconds(template: Template, sample_rate: f32, target_hz: f32, release_time: u8) -> f64 {
    let params = params_with_release(template, release_time);
    let speed_scale = target_hz * template.period_seconds();

    let mut eg = TimeEg::new();
    eg.note_on();
    // 3周分回してからnote_off。
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
    let templates = [Template::Triangle, Template::SawUp, Template::SawDown, Template::Square];
    // 基準周波数の候補（計画: 約0.16〜160Hz、128=5Hz）と、可変幅で上振れした先の周波数。
    let targets: [f32; 9] = [0.15625, 1.0, 5.0, 20.0, 50.0, 100.0, 160.0, 640.0, 2560.0];

    for &sample_rate in &[24_000.0f32, 48_000.0f32] {
        println!("\n================ sample_rate = {sample_rate} Hz ================");
        println!("1サンプル1段の進行上限: TRIANGLE/SAW={:.0}Hz, SQUARE={:.0}Hz",
            sample_rate / 2.0, sample_rate / 4.0);
        for &template in &templates {
            println!("\n--- {} (公称T={}={:.4}s, 1周={:.4}s, ループ{}段) ---",
                template.name(), T, time_to_seconds(T), template.period_seconds(), template.loop_stages());
            println!("{:>10} | {:>12} | {:>9} | {:>12} | {:>7}",
                "目標Hz", "実測Hz", "誤差%", "サンプル/周", "周期数");
            for &target in &targets {
                let m = measure(template, sample_rate, target);
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
    // リリース区間がRATEに引きずられるか（speed_scaleは全段へ一律に効くため）
    // -----------------------------------------------------------------------
    println!("\n\n================ リリース区間の伸縮 (24000 Hz) ================");
    let release_time: u8 = 120;
    let nominal = time_to_seconds(release_time);
    println!("連結したリリース段: time={release_time} = {nominal:.4}s（元paramsの値そのまま）\n");
    println!("{:>10} | {:>12} | {:>14} | {:>10}", "基準Hz", "speed_scale", "実測リリース秒", "公称比");
    for &target in &[0.15625f32, 1.0, 5.0, 20.0, 160.0] {
        let scale = target * Template::Triangle.period_seconds();
        let secs = measure_release_seconds(Template::Triangle, 24_000.0, target, release_time);
        println!("{:>10.4} | {:>12.5} | {:>14.4} | {:>9.2}x", target, scale, secs, secs / nominal as f64);
    }
}
