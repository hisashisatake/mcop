// ---------------------------------------------------------------------------
// Performance LFO (vibrato / tremolo)
// ---------------------------------------------------------------------------

pub mod lfo;
pub use lfo::{LfoWaveform, PerformanceLfo, LfoDestination, PerformanceLfoTarget, apply_lfo_modulation,
    pitch_depth_cents, volume_depth};

// ---------------------------------------------------------------------------
// Master effects (Reverb / Chorus)
// ---------------------------------------------------------------------------

pub mod effects;
pub use effects::{ChorusType, MasterEffects, ReverbType};

// ---------------------------------------------------------------------------
// VCO 抽象境界 / 後段プロセッサー境界（フェーズ7の土台）
// ---------------------------------------------------------------------------

pub mod vco;
pub use vco::{AudioProcessor, Vco};

pub mod eg;
pub use eg::Eg;

pub mod vcf;
pub use vcf::{cutoff_to_hz, effective_cutoff, FilterType, Svf, Vcf, VoiceFilter};

pub mod vca;
pub use vca::{Vca, VoiceAmp};

// ---------------------------------------------------------------------------
// Wave table format (ymfm-compatible log encoding)
// ---------------------------------------------------------------------------

const WAVE_SIZE: usize = 1024;
const LOG_SILENCE: u16 = 0x7FFF;

/// Internal wave table: 1024 × u16 log-encoded samples.
///
///   bit14~0 : −log₂|amplitude| in 4.8 fixed point (0 = peak, 0x7FFF = silence)
///   bit15   : sign flag (1 = negative)
pub struct WaveTable {
    data: [u16; WAVE_SIZE],
}

impl WaveTable {
    fn new() -> Self {
        Self { data: [LOG_SILENCE; WAVE_SIZE] }
    }

    /// Decode one sample at the given table index to linear [-1.0, 1.0].
    pub fn sample_at(&self, idx: usize) -> f32 {
        log_to_linear(self.data[idx % WAVE_SIZE])
    }

    pub fn len(&self) -> usize {
        WAVE_SIZE
    }
}

fn linear_to_log(sample: f32) -> u16 {
    let sign: u16 = if sample < 0.0 { 0x8000 } else { 0 };
    let abs = sample.abs().min(1.0);
    if abs < 1e-9 {
        return sign | LOG_SILENCE;
    }
    let log_val = ((-abs.log2()) * 256.0) as i32;
    sign | (log_val.clamp(0, LOG_SILENCE as i32) as u16)
}

fn log_to_linear(entry: u16) -> f32 {
    let log_val = (entry & 0x7FFF) as f32;
    if log_val >= 0x7E00 as f32 {
        return 0.0;
    }
    let sign = if entry & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    sign * 2.0f32.powf(-(log_val / 256.0))
}

// ---------------------------------------------------------------------------
// Built-in wave generators
// ---------------------------------------------------------------------------

pub fn gen_sine() -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let phase = i as f32 / WAVE_SIZE as f32;
        t.data[i] = linear_to_log((2.0 * std::f32::consts::PI * phase).sin());
    }
    t
}

pub fn gen_square() -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        t.data[i] = linear_to_log(if i < WAVE_SIZE / 2 { 1.0 } else { -1.0 });
    }
    t
}

pub fn gen_sawtooth() -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let phase = i as f32 / WAVE_SIZE as f32;
        t.data[i] = linear_to_log(2.0 * phase - 1.0);
    }
    t
}

pub fn gen_triangle() -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let p = i as f32 / WAVE_SIZE as f32;
        let s = if p < 0.5 { 4.0 * p - 1.0 } else { 3.0 - 4.0 * p };
        t.data[i] = linear_to_log(s);
    }
    t
}

/// Build a wave table from an arbitrary waveform function.
/// `f` maps phase (0.0–1.0) to amplitude (-1.0–1.0); out-of-range values are clamped.
pub fn gen_from_fn(f: impl Fn(f32) -> f32) -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let phase = i as f32 / WAVE_SIZE as f32;
        t.data[i] = linear_to_log(f(phase).clamp(-1.0, 1.0));
    }
    t
}

/// Convert 32 × i8 user wave input to internal 1024-entry log format.
pub fn convert_wave_32(input: &[i8; 32]) -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let pos = i as f32 * 32.0 / WAVE_SIZE as f32;
        let idx = pos as usize;
        let frac = pos - idx as f32;
        let a = input[idx % 32] as f32 / 128.0;
        let b = input[(idx + 1) % 32] as f32 / 128.0;
        t.data[i] = linear_to_log((a + frac * (b - a)).clamp(-1.0, 1.0));
    }
    t
}

// ---------------------------------------------------------------------------
// ADSR parameters
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct AdsrParams {
    /// Time from key-on to peak level (0 = slowest, 255 = fastest)
    pub attack: u8,
    /// Time from peak to sustain level
    pub decay: u8,
    /// Sustain amplitude (0–255 maps to 0.0–1.0)
    pub sustain: u8,
    /// Time from key-off to silence
    pub release: u8,
}

impl Default for AdsrParams {
    fn default() -> Self {
        Self { attack: 200, decay: 150, sustain: 180, release: 100 }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_roundtrip_log() {
        for i in 0..100 {
            let x = i as f32 / 100.0;
            let encoded = linear_to_log(x);
            let decoded = log_to_linear(encoded);
            assert!((decoded - x).abs() < 0.01, "roundtrip failed at x={x}: got {decoded}");
        }
    }

    #[test]
    fn wave_convert_32_length() {
        let input = [0i8; 32];
        let t = convert_wave_32(&input);
        assert_eq!(t.len(), WAVE_SIZE);
    }

    #[test]
    fn gen_from_fn_matches_gen_sine() {
        let sine = gen_sine();
        let from_fn = gen_from_fn(|p| (2.0 * std::f32::consts::PI * p).sin());
        for i in 0..WAVE_SIZE {
            assert!((sine.sample_at(i) - from_fn.sample_at(i)).abs() < 1e-6);
        }
    }
}
