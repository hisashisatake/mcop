use std::f32::consts::TAU;

// ---------------------------------------------------------------------------
// パラメーターマッピング
// ---------------------------------------------------------------------------

/// mod_rate=0 → 0.05Hz, mod_rate=255 → 5Hz（指数マッピング）
fn mod_rate_to_hz(mod_rate: u8) -> f32 {
    const F_MIN: f32 = 0.05;
    const F_MAX: f32 = 5.0;
    F_MIN * (F_MAX / F_MIN).powf(mod_rate as f32 / 255.0)
}

/// mod_depth=0 → 0ms, mod_depth=255 → 6ms（線形マッピング）
fn mod_depth_to_ms(mod_depth: u8) -> f32 {
    const D_MAX: f32 = 6.0;
    mod_depth as f32 / 255.0 * D_MAX
}

// ---------------------------------------------------------------------------
// Chorus Type
// ---------------------------------------------------------------------------

/// GM2/GS準拠のChorusタイプ（spec.md マスターエフェクトセクション参照）。
/// 宣言順 = NRPN値（0〜7）= タイプごとのチューニング表インデックス。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ChorusType {
    #[default]
    Chorus1,
    Chorus2,
    Chorus3,
    Chorus4,
    FeedbackChorus,
    Flanger,
    ShortDelay,
    ShortDelayFb,
}

impl ChorusType {
    /// NRPN値（0〜7）からの変換。範囲外はShortDelayFb（最大値）にclampする。
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => ChorusType::Chorus1,
            1 => ChorusType::Chorus2,
            2 => ChorusType::Chorus3,
            3 => ChorusType::Chorus4,
            4 => ChorusType::FeedbackChorus,
            5 => ChorusType::Flanger,
            6 => ChorusType::ShortDelay,
            _ => ChorusType::ShortDelayFb,
        }
    }
}

struct ChorusTuning {
    base_delay_ms: f32,
    depth_scale: f32,
    rate_scale: f32,
    feedback: f32,
}

const CHORUS_TUNINGS: [ChorusTuning; 8] = [
    // Chorus1〜4: ディレイ・深さ・レートが段階的に大きくなる定番コーラス
    ChorusTuning { base_delay_ms: 15.0, depth_scale: 1.0, rate_scale: 1.0, feedback: 0.0 },
    ChorusTuning { base_delay_ms: 20.0, depth_scale: 1.3, rate_scale: 1.0, feedback: 0.0 },
    ChorusTuning { base_delay_ms: 25.0, depth_scale: 1.6, rate_scale: 1.2, feedback: 0.0 },
    ChorusTuning { base_delay_ms: 30.0, depth_scale: 2.0, rate_scale: 1.4, feedback: 0.0 },
    // Feedback Chorus: Chorus2相当 + フィードバック
    ChorusTuning { base_delay_ms: 20.0, depth_scale: 1.3, rate_scale: 1.0, feedback: 0.5 },
    // Flanger: 短いディレイ + フィードバック
    ChorusTuning { base_delay_ms: 5.0, depth_scale: 1.0, rate_scale: 1.0, feedback: 0.3 },
    // Short Delay / Short Delay (FB): 変調なしの固定短ディレイ
    ChorusTuning { base_delay_ms: 8.0, depth_scale: 0.0, rate_scale: 0.0, feedback: 0.0 },
    ChorusTuning { base_delay_ms: 8.0, depth_scale: 0.0, rate_scale: 0.0, feedback: 0.4 },
];

/// バッファ長算出用の最大ディレイ時間（base_delay_ms + depth_scale×D_MAXの最大値より大きい値）。
const MAX_DELAY_MS: f32 = 50.0;

// ---------------------------------------------------------------------------
// Chorus
// ---------------------------------------------------------------------------

/// LFO変調ディレイラインによるコーラス/フランジャー。
/// `sound-core`の他コンポーネントに依存しない、エンジン非依存のDSPモジュール。
pub struct Chorus {
    sample_rate: f32,
    chorus_type: ChorusType,
    mod_rate: u8,
    mod_depth: u8,
    feedback_param: u8,
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    lfo_phase: f32,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        let len = ((MAX_DELAY_MS / 1000.0) * sample_rate) as usize + 1;
        Self {
            sample_rate,
            chorus_type: ChorusType::default(),
            mod_rate: 128,
            mod_depth: 128,
            feedback_param: 0,
            buffer_l: vec![0.0; len],
            buffer_r: vec![0.0; len],
            write_pos: 0,
            lfo_phase: 0.0,
        }
    }

    pub fn set_type(&mut self, chorus_type: ChorusType) {
        self.chorus_type = chorus_type;
    }

    pub fn set_mod_rate(&mut self, value: u8) {
        self.mod_rate = value;
    }

    pub fn set_mod_depth(&mut self, value: u8) {
        self.mod_depth = value;
    }

    pub fn set_feedback(&mut self, value: u8) {
        self.feedback_param = value;
    }

    /// 1サンプル処理する。
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let tuning = &CHORUS_TUNINGS[self.chorus_type as usize];

        let rate_hz = mod_rate_to_hz(self.mod_rate) * tuning.rate_scale;
        self.lfo_phase = (self.lfo_phase + rate_hz / self.sample_rate).fract();
        let lfo = (self.lfo_phase * TAU).sin();

        let depth_ms = mod_depth_to_ms(self.mod_depth) * tuning.depth_scale;
        let delay_ms = tuning.base_delay_ms + lfo * depth_ms;
        let delay_samples = (delay_ms / 1000.0 * self.sample_rate).max(0.0);

        let feedback = (tuning.feedback + self.feedback_param as f32 / 255.0 * 0.5).min(0.95);

        let len = self.buffer_l.len();
        let read_pos = (self.write_pos as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = read_pos as usize % len;
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();

        let out_l = self.buffer_l[idx0] * (1.0 - frac) + self.buffer_l[idx1] * frac;
        let out_r = self.buffer_r[idx0] * (1.0 - frac) + self.buffer_r[idx1] * frac;

        self.buffer_l[self.write_pos] = in_l + out_l * feedback;
        self.buffer_r[self.write_pos] = in_r + out_r * feedback;
        self.write_pos = (self.write_pos + 1) % len;

        (out_l, out_r)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_type_is_chorus1() {
        assert_eq!(ChorusType::default(), ChorusType::Chorus1);
    }

    #[test]
    fn from_u8_mapping() {
        assert_eq!(ChorusType::from_u8(0), ChorusType::Chorus1);
        assert_eq!(ChorusType::from_u8(4), ChorusType::FeedbackChorus);
        assert_eq!(ChorusType::from_u8(7), ChorusType::ShortDelayFb);
        assert_eq!(ChorusType::from_u8(255), ChorusType::ShortDelayFb);
    }

    #[test]
    fn modulated_chorus_varies_output() {
        let mut chorus = Chorus::new(44100.0);
        chorus.set_type(ChorusType::Chorus1);
        chorus.set_mod_rate(200); // 速めのLFOで短時間でも変化を観測できるようにする
        chorus.set_mod_depth(255);

        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for _ in 0..4410 {
            let (out_l, _) = chorus.process(1.0, 1.0);
            min = min.min(out_l);
            max = max.max(out_l);
        }
        assert!(max - min > 0.01, "LFO変調により出力が変化するはず: min={min}, max={max}");
    }

    #[test]
    fn short_delay_has_no_modulation() {
        let mut chorus = Chorus::new(44100.0);
        chorus.set_type(ChorusType::ShortDelay);
        chorus.set_mod_rate(255);
        chorus.set_mod_depth(255);

        // 立ち上がり後は固定ディレイのため、定常入力に対して出力も一定値に収束する
        for _ in 0..1000 {
            chorus.process(1.0, 1.0);
        }
        let (a, _) = chorus.process(1.0, 1.0);
        let (b, _) = chorus.process(1.0, 1.0);
        assert!((a - b).abs() < 1e-6, "Short Delayは変調なしのため出力が一定のはず: a={a}, b={b}");
    }

    #[test]
    fn feedback_chorus_stays_bounded() {
        let mut chorus = Chorus::new(44100.0);
        chorus.set_type(ChorusType::FeedbackChorus);
        chorus.set_feedback(255);

        for _ in 0..(44100 * 2) {
            let (out_l, out_r) = chorus.process(1.0, -1.0);
            assert!(out_l.is_finite() && out_r.is_finite());
            assert!(out_l.abs() < 100.0 && out_r.abs() < 100.0, "発散している: {out_l}, {out_r}");
        }
    }
}
