// ---------------------------------------------------------------------------
// SVFフィルター + Vcf（spec.md「フィルター」セクション参照）
//
// FM合成出力にかけるボイス単位のVCF相当。Cutoff/Resonance/Filter Type（LP/HP/BP）と、
// Cutoffを変調するキーオン連動EG（5段OPM形式、[crate::eg::Eg]）を実装する。
//
// 数式・マッピングはすべて初期案（暫定）。CLAUDE.mdのテスト方針に従い、
// 実装後に音を聴いて係数を調整する。
// ---------------------------------------------------------------------------

use crate::eg::{Eg, EgParams};

/// Cutoff(0〜255)→Hz（対数、0≒20Hz、255≒20kHz、暫定）。
pub fn cutoff_to_hz(cutoff: u8) -> f32 {
    const F_MIN: f32 = 20.0;
    const F_MAX: f32 = 20000.0;
    F_MIN * (F_MAX / F_MIN).powf(cutoff as f32 / 255.0)
}

/// 実効Cutoff = clamp(Cutoffベース値 + Filter EG出力 × Filter EG Depth, 0, 255)
pub fn effective_cutoff(base_cutoff: u8, eg_output: f32, depth: u8) -> u8 {
    (base_cutoff as f32 + eg_output * depth as f32)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Resonance(0〜255)→SVFのQ値。Self-Oscillation ONなら255でほぼ無減衰（自己発振）、
/// OFFなら255でも発振寸前で安定動作する（暫定）。
fn resonance_to_q(resonance: u8, self_oscillation: bool) -> f32 {
    const Q_MIN: f32 = 0.5;
    let q_max = if self_oscillation { 1000.0 } else { 20.0 };
    let r = resonance as f32 / 255.0;
    Q_MIN + r * (q_max - Q_MIN)
}

// ---------------------------------------------------------------------------
// Filter Type
// ---------------------------------------------------------------------------

/// フィルタータイプ（spec.md準拠、0=LP/1=HP/2=BP）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FilterType {
    #[default]
    Lp,
    Hp,
    Bp,
}

impl FilterType {
    /// 0〜255からの変換。0=LP、1=HP、2以上=BP。
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => FilterType::Lp,
            1 => FilterType::Hp,
            _ => FilterType::Bp,
        }
    }
}

// ---------------------------------------------------------------------------
// SVF（State Variable Filter）
//
// 内部の計算式（ic1eq/ic2eq、g/k/a1/a2/a3、v1/v2/v3）は、Andrew Simper(Cytomic)の
// 技術文書「SvfLinearTrapOptimised2.pdf」（Solving the continuous SVF equations
// using a trapezoidal integrator）に掲載されているTPT(Topology-Preserving Transform)
// 型SVFの参考実装を基にした。
// https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf
// 出典の詳細は THIRD_PARTY_NOTICES.md を参照。
// ---------------------------------------------------------------------------

/// TPT（Topology-Preserving Transform）型SVF（Andrew Simper,通称Cytomic SVF）。
/// LP/HP/BPを同一の内部状態(ic1eq/ic2eq)から同時に導出でき、
/// 高域カットオフ・高Resonanceでも数値的に安定。
pub struct Svf {
    ic1eq: f32,
    ic2eq: f32,
}

impl Svf {
    pub fn new() -> Self {
        Self { ic1eq: 0.0, ic2eq: 0.0 }
    }

    /// 1サンプル処理する。`filter_type`に応じてLP/HP/BPいずれかの出力を返す。
    pub fn process(
        &mut self,
        input: f32,
        sample_rate: f32,
        cutoff_hz: f32,
        resonance: u8,
        self_oscillation: bool,
        filter_type: FilterType,
    ) -> f32 {
        let g = (std::f32::consts::PI * cutoff_hz / sample_rate).tan();
        let k = 1.0 / resonance_to_q(resonance, self_oscillation);

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        match filter_type {
            FilterType::Lp => v2,
            FilterType::Bp => v1,
            FilterType::Hp => input - k * v1 - v2,
        }
    }
}

impl Default for Svf {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Vcf トレイト + 具象実装（VoiceFilter）
// ---------------------------------------------------------------------------

/// ボイス単位のVCFセクションの共通インターフェース。SVF本体 + Cutoffを変調する
/// キーオン連動EGを一体で保持する。発振原理に依存しないプリミティブ型のみで構成される。
pub trait Vcf: Send {
    fn note_on(&mut self);
    fn note_off(&mut self);
    #[allow(clippy::too_many_arguments)]
    fn process(
        &mut self,
        input: f32,
        sample_rate: f32,
        base_cutoff: u8,
        resonance: u8,
        filter_type: FilterType,
        self_oscillation: bool,
        eg_params: EgParams,
        eg_depth: u8,
    ) -> f32;
}

/// ボイス単位のVCFセクション（SVF本体 + Cutoffを変調するキーオン連動EGを一体で保持する）。
pub struct VoiceFilter {
    svf: Svf,
    cutoff_eg: Eg,
}

impl VoiceFilter {
    pub fn new() -> Self {
        Self { svf: Svf::new(), cutoff_eg: Eg::new() }
    }
}

impl Default for VoiceFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Vcf for VoiceFilter {
    fn note_on(&mut self) {
        self.cutoff_eg.note_on();
    }

    fn note_off(&mut self) {
        self.cutoff_eg.note_off();
    }

    fn process(
        &mut self,
        input: f32,
        sample_rate: f32,
        base_cutoff: u8,
        resonance: u8,
        filter_type: FilterType,
        self_oscillation: bool,
        eg_params: EgParams,
        eg_depth: u8,
    ) -> f32 {
        let eg_level = self.cutoff_eg.tick(sample_rate, eg_params, 1.0);
        let cutoff = effective_cutoff(base_cutoff, eg_level, eg_depth);
        let cutoff_hz = cutoff_to_hz(cutoff);
        self.svf.process(input, sample_rate, cutoff_hz, resonance, self_oscillation, filter_type)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_to_hz_bounds() {
        assert!((cutoff_to_hz(0) - 20.0).abs() < 1e-2);
        assert!((cutoff_to_hz(255) - 20000.0).abs() < 1.0);
        assert!(cutoff_to_hz(255) > cutoff_to_hz(0));
    }

    #[test]
    fn effective_cutoff_clamps() {
        assert_eq!(effective_cutoff(200, 1.0, 255), 255);
        assert_eq!(effective_cutoff(0, 0.0, 255), 0);
        assert_eq!(effective_cutoff(100, 1.0, 100), 200);
        assert_eq!(effective_cutoff(0, -1.0, 255), 0);
    }

    #[test]
    fn filter_type_from_u8_mapping() {
        assert_eq!(FilterType::from_u8(0), FilterType::Lp);
        assert_eq!(FilterType::from_u8(1), FilterType::Hp);
        assert_eq!(FilterType::from_u8(2), FilterType::Bp);
        assert_eq!(FilterType::from_u8(255), FilterType::Bp);
    }

    #[test]
    fn self_oscillation_rings_longer_than_without() {
        let sr = 44100.0;
        let cutoff_hz = cutoff_to_hz(128);

        let mut svf_on = Svf::new();
        let mut svf_off = Svf::new();

        // インパルスを入力し、以後は無音入力で余韻(リング)を観察する
        svf_on.process(1.0, sr, cutoff_hz, 255, true, FilterType::Lp);
        svf_off.process(1.0, sr, cutoff_hz, 255, false, FilterType::Lp);

        let total = 2000;
        let tail = 200;
        let mut peak_on = 0.0f32;
        let mut peak_off = 0.0f32;
        for i in 0..total {
            let out_on = svf_on.process(0.0, sr, cutoff_hz, 255, true, FilterType::Lp);
            let out_off = svf_off.process(0.0, sr, cutoff_hz, 255, false, FilterType::Lp);
            if i >= total - tail {
                peak_on = peak_on.max(out_on.abs());
                peak_off = peak_off.max(out_off.abs());
            }
        }

        assert!(
            peak_on > peak_off * 2.0,
            "Self-Oscillation ONの方が余韻が長く残るはず: on={peak_on}, off={peak_off}"
        );
    }

    #[test]
    fn svf_long_run_no_nan_across_full_range() {
        let sr = 44100.0;
        for cutoff in [0u8, 64, 128, 192, 255] {
            for resonance in [0u8, 128, 255] {
                for self_osc in [false, true] {
                    for ft in [FilterType::Lp, FilterType::Hp, FilterType::Bp] {
                        let mut svf = Svf::new();
                        let cutoff_hz = cutoff_to_hz(cutoff);
                        for i in 0..4410 {
                            let input = (i as f32 * 0.1).sin();
                            let out = svf.process(input, sr, cutoff_hz, resonance, self_osc, ft);
                            assert!(
                                out.is_finite(),
                                "non-finite: cutoff={cutoff} resonance={resonance} self_osc={self_osc} ft={ft:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
