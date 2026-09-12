//! 非正規化数(denormal/subnormal)がどこでどれだけ長く発生するかを定量化する診断用の使い捨て例。
//! FTZ/DAZ対策の要否を判断する材料にする（plans/purring-spinning-acorn.md参照）。
//! SVFフィルターとFDNリバーブは、前サンプルの出力を次の入力へ混ぜ戻す再帰構造を持ち、
//! 入力が無音になった後もその内部状態が指数的に0へ減衰し続けるため、非正規化数の
//! 典型的な発生源として先に疑う。
//!
//! 実行: cargo run --release -p sound-core --example denormal_probe

use std::time::Instant;

use sound_core::effects::{Reverb, ReverbType};
use sound_core::{FilterType, Svf};

const SAMPLE_RATE: f32 = 44100.0;

fn main() {
    println!("=== SVFフィルター ===");
    probe_svf();
    println!();
    println!("=== FDNリバーブ ===");
    probe_reverb();
    println!();
    println!("=== 演算コスト実測（通常値 vs 非正規化数）===");
    probe_cost();
}

struct Occupancy {
    first: Option<usize>,
    stay_samples: usize,
    still_subnormal_at_end: bool,
}

/// `total`サンプル分`f`を呼び、出力が非正規化数(0を除く)である区間を計測する。
fn observe(total: usize, mut f: impl FnMut(usize) -> f32) -> Occupancy {
    let mut first = None;
    let mut stay = 0usize;
    let mut last_val = 0.0f32;
    for i in 0..total {
        let v = f(i);
        last_val = v;
        if v != 0.0 && v.is_subnormal() {
            if first.is_none() {
                first = Some(i);
            }
            stay += 1;
        }
    }
    Occupancy { first, stay_samples: stay, still_subnormal_at_end: last_val != 0.0 && last_val.is_subnormal() }
}

fn report(label: &str, occ: &Occupancy, total: usize) {
    match occ.first {
        Some(f) => {
            println!(
                "{label}: 最初に非正規化数へ落ちたのはサンプル{f}({:.3}秒後)。観測window内の滞在サンプル数={}({:.3}秒相当)。観測終了時点でまだ非正規化数={}",
                f as f32 / SAMPLE_RATE,
                occ.stay_samples,
                occ.stay_samples as f32 / SAMPLE_RATE,
                occ.still_subnormal_at_end
            );
            if occ.still_subnormal_at_end {
                println!(
                    "  → 観測window({:.1}秒)内で収束しなかった。実際の滞在時間はさらに長い可能性が高い",
                    total as f32 / SAMPLE_RATE
                );
            }
        }
        None => println!("{label}: 非正規化数への到達なし（観測window内では通常の丸めでゼロへ収束）"),
    }
}

fn probe_svf() {
    for resonance in [0u8, 128, 200] {
        let mut svf = Svf::new();
        let cutoff = 800.0;
        let total = (SAMPLE_RATE * 8.0) as usize;
        let occ = observe(total, |i| {
            let input = if i == 0 { 1.0 } else { 0.0 };
            svf.process(input, SAMPLE_RATE, cutoff, resonance, false, FilterType::Lp)
        });
        report(&format!("SVF(cutoff=800Hz, resonance={resonance}, LP)"), &occ, total);
    }
}

fn probe_reverb() {
    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_type(ReverbType::Hall2); // 最長残響のタイプ
    reverb.set_time(255); // 最長のRT60
    let total = (SAMPLE_RATE * 20.0) as usize; // 長残響を見込み20秒観測
    let occ = observe(total, |i| {
        let input = if i == 0 { 1.0 } else { 0.0 };
        let (l, _r) = reverb.process(input, input);
        l
    });
    report("FDNリバーブ(Hall2, time=255)", &occ, total);
}

/// 非正規化数演算がこのマシン・このビルド設定で実際にどれだけ遅いかを実測する。
/// FTZ/DAZが有効なCPU/OS設定では差が出ない可能性がある点に注意（対策の効果測定にも使える）。
/// 通常値/非正規化数の両ループを**同一の分岐構造**（同じ間隔での値リセット）にして、
/// 分岐コストの違いが測定結果に混入しないようにする。
fn probe_cost() {
    const N: usize = 20_000_000;
    const RESET_INTERVAL: usize = 1000;

    let mut normal_val = 1.0f32;
    let start = Instant::now();
    for i in 0..N {
        normal_val *= 0.999_999;
        if i % RESET_INTERVAL == 0 {
            normal_val = 1.0; // 通常値域に留め続ける
        }
        std::hint::black_box(normal_val);
    }
    let normal_elapsed = start.elapsed();

    let mut denormal_val = f32::MIN_POSITIVE / 2.0; // 非正規化数域からスタート
    let start = Instant::now();
    for i in 0..N {
        denormal_val *= 0.999_999;
        if i % RESET_INTERVAL == 0 {
            denormal_val = f32::MIN_POSITIVE / 2.0; // 非正規化数域に留め続ける
        }
        std::hint::black_box(denormal_val);
    }
    let denormal_elapsed = start.elapsed();

    println!("通常値ループ({N}回の乗算、{RESET_INTERVAL}回ごとにリセット): {normal_elapsed:?}");
    println!("非正規化数ループ({N}回の乗算、{RESET_INTERVAL}回ごとにリセット): {denormal_elapsed:?}");
    println!(
        "比率: {:.1}倍",
        denormal_elapsed.as_secs_f64() / normal_elapsed.as_secs_f64().max(1e-12)
    );
}
