// ---------------------------------------------------------------------------
// Performance LFO (vibrato / tremolo)
// ---------------------------------------------------------------------------

pub mod lfo;
pub use lfo::{LfoWaveform, LfoFadeMode, PerformanceLfoShape, PerformanceLfo, LfoDestination,
    PerformanceLfoTarget, apply_lfo_modulation, pitch_depth_cents, volume_depth, cutoff_depth,
    delay_to_seconds, lfo_fade_mode_from_index, lfo_offset_from_param, lfo_offset_to_param,
    lfo_waveform_from_index};

// ---------------------------------------------------------------------------
// Master effects (Reverb / Chorus)
// ---------------------------------------------------------------------------

pub mod effects;
pub use effects::{ChorusType, MasterEffects, ReverbType};

pub mod master_output;
pub use master_output::{Measurement, MasterOutput};

pub mod master_section;
pub use master_section::MasterSection;

// ---------------------------------------------------------------------------
// VCO 抽象境界 / 後段プロセッサー境界（フェーズ7の土台）
// ---------------------------------------------------------------------------

pub mod vco;
pub use vco::{AudioProcessor, Vco};

pub mod eg;
pub use eg::{cc76_to_rate_scale, cc_to_time_scale, BipolarFg, Eg, EgParams, GainFg};

pub mod time_eg;
pub use time_eg::{
    apply_loop_drift, bipolar_level, depth_drift_per_cycle, drift_accumulated_after_cycles,
    level_drift_per_cycle, loop_level_range, loop_pivot_level, nearest_sync_note, pan_gains,
    seconds_to_time, sync_note_anchor, sync_note_beats, sync_rate_beats, sync_region_seconds,
    tempo_speed_scale, time_to_seconds, TimeEg, TimeEgParams, TimeStage, BIPOLAR_NEUTRAL_LEVEL,
    BIPOLAR_NEUTRAL_RAW, MAX_STAGES, RETRIGGER_MODE_CONTINUE, RETRIGGER_MODE_RESET,
    SYNC_NOTE_COUNT, TEXTURE_CHAOS, TEXTURE_OFF, TEXTURE_RANDOM, TEXTURE_SAMPLE_HOLD,
};

pub mod vcf;
pub use vcf::{
    cutoff_to_hz, effective_cutoff, effective_cutoff_bipolar_level, FilterType, Svf, Vcf,
    VoiceFilter,
};

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
    /// `log_to_linear(data[i])` decoded once at construction time, so `sample_at()`
    /// (called every sample for every sounding operator) is a plain array read
    /// instead of a `powf()` call. Values are identical to decoding `data` on the fly.
    linear: [f32; WAVE_SIZE],
}

impl WaveTable {
    fn new() -> Self {
        Self { data: [LOG_SILENCE; WAVE_SIZE], linear: [0.0; WAVE_SIZE] }
    }

    /// Encode `value` into the log table and refresh the decoded linear cache.
    fn set(&mut self, idx: usize, value: f32) {
        self.data[idx] = linear_to_log(value);
        self.linear[idx] = log_to_linear(self.data[idx]);
    }

    /// Decode one sample at the given table index to linear [-1.0, 1.0].
    pub fn sample_at(&self, idx: usize) -> f32 {
        self.linear[idx % WAVE_SIZE]
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
        t.set(i, (2.0 * std::f32::consts::PI * phase).sin());
    }
    t
}

pub fn gen_square() -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        t.set(i, if i < WAVE_SIZE / 2 { 1.0 } else { -1.0 });
    }
    t
}

pub fn gen_sawtooth() -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let phase = i as f32 / WAVE_SIZE as f32;
        t.set(i, 2.0 * phase - 1.0);
    }
    t
}

pub fn gen_triangle() -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let p = i as f32 / WAVE_SIZE as f32;
        let s = if p < 0.5 { 4.0 * p - 1.0 } else { 3.0 - 4.0 * p };
        t.set(i, s);
    }
    t
}

/// Build a wave table from an arbitrary waveform function.
/// `f` maps phase (0.0–1.0) to amplitude (-1.0–1.0); out-of-range values are clamped.
pub fn gen_from_fn(f: impl Fn(f32) -> f32) -> WaveTable {
    let mut t = WaveTable::new();
    for i in 0..WAVE_SIZE {
        let phase = i as f32 / WAVE_SIZE as f32;
        t.set(i, f(phase).clamp(-1.0, 1.0));
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
        t.set(i, (a + frac * (b - a)).clamp(-1.0, 1.0));
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
