//! `TimeEg`（ループ付きN点Time/Level EG）の触り心地を、現行`Eg`（レート方式）と
//! 聴き比べるための診断用の使い捨て例（memory `project_4point_tl_eg_decision.md`参照）。
//!
//! A群（音量に適用）: 現行Egは`Attack`と`LoopUp`が同じ`ar`を共有するため
//!   「速いアタック＋ゆっくりしたループ」が原理的に書けない。a1/a2でそれを実際に聴かせ、
//!   a3のTimeEgでアタックとループが独立に振る舞うことを示す。
//! B群（カットオフに適用）: 現行Egのループはfloor⇄peakの三角波1種類のみ。
//!   b1に対し、b2は非対称、b3は階段状——TimeEgの多点ループが任意の折れ線を描けることを示す。
//! C群（カットオフに適用）: TimeEgは1周の合計時間が定義できるため、
//!   同じ形のまま`speed_scale`だけでテンポに同期できることを示す（c1=8分、c2=4分）。
//!
//! 実行: cargo run -p sound-core --example time_eg_probe -- <出力ディレクトリ>

use std::path::Path;

use sound_core::{Eg, EgParams, FilterType, Svf, TimeEg, TimeEgParams, TimeStage, MAX_STAGES};

const SAMPLE_RATE: f32 = 44100.0;
const NOTE_FREQ: f32 = 110.0; // A2。振幅変調・カットオフ掃引の両方で聴き取りやすい帯域
const HOLD_SECS: f32 = 3.0;
const RELEASE_TAIL_SECS: f32 = 1.0;
const CUTOFF_MIN_HZ: f32 = 300.0;
const CUTOFF_MAX_HZ: f32 = 3000.0;
const RESONANCE: u8 = 180;

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let out_dir = Path::new(&out_dir);

    // --- A群：アタックとループの分離（音量に適用） ---

    // a1: 現行Eg、ar=250（速い）。ループの立ち上がりも同じarを共有するため速くなる。
    let a1 = render_current_eg_amplitude(EgParams {
        ar: 250,
        d1r: 150,
        d1l: 0,
        d2r: 0,
        rr: 150,
        floor: 0,
        loop_enabled: 1,
        curve: 0,
        delay: 0,
    });
    // a2: 現行Eg、ar=60（遅い）。アタックを遅くすると、ループの立ち上がりも道連れで鈍る。
    let a2 = render_current_eg_amplitude(EgParams {
        ar: 60,
        d1r: 150,
        d1l: 0,
        d2r: 0,
        rr: 150,
        floor: 0,
        loop_enabled: 1,
        curve: 0,
        delay: 0,
    });
    // a3: TimeEg。段0（L0→L1）だけを~3msの一発アタックにし、段1〜2のループは~125ms/脚の
    // ゆったりしたトレモロにする。a1/a2ではできない組み合わせ。
    let a3 = render_time_eg_amplitude(
        TimeEgParams {
            stages: stages(&[
                (28, 255, 0),  // L0→L1: 瞬時に近いワンショットアタック(~3ms)
                (120, 40, 0),  // ループ低点(~125ms)
                (120, 220, 0), // ループ高点(~125ms)
                (100, 0, 0),   // リリース：無音へ
            ]),
            stage_count: 4,
            loop_enabled: 1,
            loop_start: 1,
            loop_end: 2,
            release_start: 3,
        },
        1.0,
    );

    // --- B群：ループ形状（カットオフに適用） ---

    // b1: 現行Egのfloor⇄peak三角ループ（ar/d1rはレンジが異なるテーブルなので、
    // 概ね対称な~150ms/脚になるよう個別に較正した値）。
    let b1 = render_current_eg_cutoff(EgParams {
        ar: 134,
        d1r: 71,
        d1l: 0,
        d2r: 0,
        rr: 150,
        floor: 0,
        loop_enabled: 1,
        curve: 0,
        delay: 0,
    });
    // b2: TimeEg、非対称ループ（急上昇~13ms／緩降下~282ms）。三角波では作れない形。
    let b2 = render_time_eg_cutoff(
        TimeEgParams {
            stages: stages(&[
                (60, 255, 0),  // 急上昇(~13ms)
                (140, 20, 0),  // 緩降下(~282ms)
                (110, 0, 0),   // リリース
            ]),
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 1,
            release_start: 2,
        },
        1.0,
    );
    // b3: TimeEg、階段状ループ（4点3段の上りに、速いリセットを1段添えた形）。
    let b3 = render_time_eg_cutoff(
        TimeEgParams {
            stages: stages(&[
                (40, 90, 0),   // 第1段
                (40, 170, 0),  // 第2段
                (40, 255, 0),  // 第3段
                (30, 20, 0),   // 速いリセット
                (110, 0, 0),   // リリース
            ]),
            stage_count: 5,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 3,
            release_start: 4,
        },
        1.0,
    );

    // --- C群：テンポ同期（カットオフに適用、b2/b3とは別の対称ループ形状を使う） ---

    let sync_stages = stages(&[
        (100, 255, 0), // 上り
        (100, 20, 0),  // 下り
        (110, 0, 0),   // リリース
    ]);
    let nominal_cycle_secs =
        sound_core::time_to_seconds(100) as f64 + sound_core::time_to_seconds(100) as f64;

    // c1: 1周=0.25秒（BPM120の8分音符）に合わせる。
    let c1_scale = (nominal_cycle_secs / 0.25) as f32;
    let c1 = render_time_eg_cutoff(
        TimeEgParams {
            stages: sync_stages,
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 1,
            release_start: 2,
        },
        c1_scale,
    );
    // c2: 同じ形のまま、1周=0.5秒（4分音符）へ。speed_scaleだけを変えている。
    let c2_scale = (nominal_cycle_secs / 0.5) as f32;
    let c2 = render_time_eg_cutoff(
        TimeEgParams {
            stages: sync_stages,
            stage_count: 3,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 1,
            release_start: 2,
        },
        c2_scale,
    );

    write_wav(&out_dir.join("a1_current_fast.wav"), &a1);
    write_wav(&out_dir.join("a2_current_slow.wav"), &a2);
    write_wav(&out_dir.join("a3_time_split.wav"), &a3);
    write_wav(&out_dir.join("b1_current_triangle.wav"), &b1);
    write_wav(&out_dir.join("b2_time_asymmetric.wav"), &b2);
    write_wav(&out_dir.join("b3_time_stair.wav"), &b3);
    write_wav(&out_dir.join("c1_sync_8th.wav"), &c1);
    write_wav(&out_dir.join("c2_sync_quarter.wav"), &c2);

    println!("wrote 8 probe WAVs to {}", out_dir.display());
}

/// `entries`（time, level, curve）を先頭から`TimeStage`配列へ詰める。残りは既定値(0,0,0)。
fn stages(entries: &[(u8, u8, u8)]) -> [TimeStage; MAX_STAGES] {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    for (i, &(time, level, curve)) in entries.iter().enumerate() {
        stages[i] = TimeStage { time, level, curve };
    }
    stages
}

/// ナイーブなのこぎり波発振器（帯域制限なし。診断用の聴き比べには十分）。
struct Saw {
    phase: f32,
}

impl Saw {
    fn new() -> Self {
        Self { phase: 0.0 }
    }

    fn next(&mut self, freq: f32, sample_rate: f32) -> f32 {
        let sample = 2.0 * self.phase - 1.0;
        self.phase += freq / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        sample
    }
}

/// 現行`Eg`（ループ有効）の出力をそのまま振幅エンベロープとして、のこぎり波に掛ける。
fn render_current_eg_amplitude(params: EgParams) -> Vec<f32> {
    let mut eg = Eg::new();
    let mut saw = Saw::new();
    eg.note_on();
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = Vec::with_capacity(hold_samples + release_samples);
    for i in 0..(hold_samples + release_samples) {
        if i == hold_samples {
            eg.note_off();
        }
        let level = eg.tick(SAMPLE_RATE, params, 1.0);
        out.push(saw.next(NOTE_FREQ, SAMPLE_RATE) * level);
    }
    out
}

/// `TimeEg`（ループ有効）の出力をそのまま振幅エンベロープとして、のこぎり波に掛ける。
fn render_time_eg_amplitude(params: TimeEgParams, speed_scale: f32) -> Vec<f32> {
    let mut eg = TimeEg::new();
    let mut saw = Saw::new();
    eg.note_on();
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = Vec::with_capacity(hold_samples + release_samples);
    for i in 0..(hold_samples + release_samples) {
        if i == hold_samples {
            eg.note_off();
        }
        let level = eg.tick(SAMPLE_RATE, params, speed_scale);
        out.push(saw.next(NOTE_FREQ, SAMPLE_RATE) * level);
    }
    out
}

/// 現行`Eg`（ループ有効）の出力をSvf(LP)のカットオフへマッピングする。
fn render_current_eg_cutoff(params: EgParams) -> Vec<f32> {
    let mut eg = Eg::new();
    let mut saw = Saw::new();
    let mut svf = Svf::new();
    eg.note_on();
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = Vec::with_capacity(hold_samples + release_samples);
    for i in 0..(hold_samples + release_samples) {
        if i == hold_samples {
            eg.note_off();
        }
        let level = eg.tick(SAMPLE_RATE, params, 1.0);
        let cutoff_hz = CUTOFF_MIN_HZ + level * (CUTOFF_MAX_HZ - CUTOFF_MIN_HZ);
        let dry = saw.next(NOTE_FREQ, SAMPLE_RATE);
        let wet = svf.process(dry, SAMPLE_RATE, cutoff_hz, RESONANCE, false, FilterType::Lp);
        out.push(wet * 0.6);
    }
    out
}

/// `TimeEg`（ループ有効）の出力をSvf(LP)のカットオフへマッピングする。
fn render_time_eg_cutoff(params: TimeEgParams, speed_scale: f32) -> Vec<f32> {
    let mut eg = TimeEg::new();
    let mut saw = Saw::new();
    let mut svf = Svf::new();
    eg.note_on();
    let hold_samples = (HOLD_SECS * SAMPLE_RATE) as usize;
    let release_samples = (RELEASE_TAIL_SECS * SAMPLE_RATE) as usize;
    let mut out = Vec::with_capacity(hold_samples + release_samples);
    for i in 0..(hold_samples + release_samples) {
        if i == hold_samples {
            eg.note_off();
        }
        let level = eg.tick(SAMPLE_RATE, params, speed_scale);
        let cutoff_hz = CUTOFF_MIN_HZ + level * (CUTOFF_MAX_HZ - CUTOFF_MIN_HZ);
        let dry = saw.next(NOTE_FREQ, SAMPLE_RATE);
        let wet = svf.process(dry, SAMPLE_RATE, cutoff_hz, RESONANCE, false, FilterType::Lp);
        out.push(wet * 0.6);
    }
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
