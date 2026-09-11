//! FTZ/DAZ(非正規化数のフラッシュ・ゼロ化)がレンダリング時間へ与える実際の効果を測る
//! 実験用の使い捨て例。`sound-core/examples/denormal_probe.rs`でSVFフィルターが無音入力後も
//! 長時間非正規化数域に留まることを確認したため、それが実際のOp505Engine全体のCPU時間に
//! どれだけ効くかを、FTZ/DAZ ON/OFFのA/Bで直接確かめる。
//!
//! 実行: cargo run --release -p op505-core --example ftz_daz_effect_probe

use std::time::{Duration, Instant};

use op505_core::{Op505Engine, Op505Patch};
use sound_core::{TimeStage, Vco};

const SAMPLE_RATE: f32 = 44100.0;
const NUM_VOICES: usize = 32;
const WARMUP_BLOCK: usize = 4096;
const RENDER_SECONDS: f32 = 30.0; // 発音後の長い無音区間を含めて観測する

fn main() {
    println!("=== FTZ/DAZ OFF（既定） ===");
    let off = bench();
    println!("time: {off:?}");

    println!();
    println!("=== FTZ/DAZ ON ===");
    unsafe {
        enable_ftz_daz();
    }
    let on = bench();
    unsafe {
        disable_ftz_daz();
    }
    println!("time: {on:?}");

    println!();
    println!(
        "比率(ON/OFF): {:.3}倍（1.0未満ならFTZ/DAZが速い）",
        on.as_secs_f64() / off.as_secs_f64()
    );
}

fn bench() -> Duration {
    let mut engine = Op505Engine::new(SAMPLE_RATE);

    let mut patch = Op505Patch::default();
    // フィルターを効かせつつ、リリースは十分長くしてキャリア無音後も
    // (is_collectibleの猶予0.2秒経過まで)ボイスが生存し、フィルターが動き続けるようにする。
    patch.channel.filter_cutoff = 100;
    patch.channel.filter_resonance = 150;
    for op in patch.operators.iter_mut() {
        op.tl = 40;
        op.eg.stage_count = 3;
        op.eg.stages[0] = TimeStage { time: 10, level: 255, curve: 0 };
        op.eg.stages[1] = TimeStage { time: 80, level: 0, curve: 0 };
        op.eg.stages[2] = TimeStage { time: 200, level: 0, curve: 0 };
        op.eg.release_point = 1;
    }
    engine.set_patch(patch);

    for ch in 0..NUM_VOICES {
        engine.note_on(ch, 220.0 + ch as f32 * 3.0, 100);
    }

    // 少し発音させてからnote_offし、以後は長い無音（フィルター残響）区間を作る。
    let mut warmup = vec![0.0f32; WARMUP_BLOCK];
    engine.render(&mut warmup, 1);
    for ch in 0..NUM_VOICES {
        engine.note_off(ch);
    }

    let total_samples = (SAMPLE_RATE * RENDER_SECONDS) as usize;
    let mut buf = vec![0.0f32; total_samples];
    let start = Instant::now();
    engine.render(&mut buf, 1);
    start.elapsed()
}

#[cfg(target_arch = "x86_64")]
unsafe fn enable_ftz_daz() {
    use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
    const FLUSH_ZERO_ON: u32 = 0x8000;
    const DENORMALS_ZERO_ON: u32 = 0x0040;
    let csr = _mm_getcsr();
    _mm_setcsr(csr | FLUSH_ZERO_ON | DENORMALS_ZERO_ON);
}

#[cfg(target_arch = "x86_64")]
unsafe fn disable_ftz_daz() {
    use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
    const FLUSH_ZERO_ON: u32 = 0x8000;
    const DENORMALS_ZERO_ON: u32 = 0x0040;
    let csr = _mm_getcsr();
    _mm_setcsr(csr & !(FLUSH_ZERO_ON | DENORMALS_ZERO_ON));
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn enable_ftz_daz() {}
#[cfg(not(target_arch = "x86_64"))]
unsafe fn disable_ftz_daz() {}
