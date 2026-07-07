// ---------------------------------------------------------------------------
// Vca（ボイス単位のTVAオーバーレイ）
// ---------------------------------------------------------------------------

use crate::eg::Eg;

/// ボイス単位のTVA（Total Voice Amplitude）オーバーレイの共通インターフェース。
/// キーオン連動EG（5段OPM形式）でゲインを乗算する。
pub trait Vca: Send {
    fn note_on(&mut self);
    fn note_off(&mut self);
    fn process(&mut self, input: f32, sample_rate: f32, ar: u8, d1r: u8, d1l: u8, d2r: u8, rr: u8) -> f32;
}

/// ボイス単位のTVAオーバーレイ（キーオン連動EGでゲインを乗算する）。
/// 既定パラメーター（ar=255,d1r=0,d1l=255,d2r=0,rr=255）ではアタック・リリースとも
/// 数サンプルで完了しほぼ常時ゲイン1.0となり、FM本来のキャリアEG（operator.rs）に対して
/// 実質無音の透過的マクロ・オーバーレイとして働く（二重EG化を避ける既定設計）。
pub struct VoiceAmp {
    eg: Eg,
}

impl VoiceAmp {
    pub fn new() -> Self {
        Self { eg: Eg::new() }
    }
}

impl Default for VoiceAmp {
    fn default() -> Self {
        Self::new()
    }
}

impl Vca for VoiceAmp {
    fn note_on(&mut self) {
        self.eg.note_on();
    }

    fn note_off(&mut self) {
        self.eg.note_off();
    }

    fn process(&mut self, input: f32, sample_rate: f32, ar: u8, d1r: u8, d1l: u8, d2r: u8, rr: u8) -> f32 {
        let gain = self.eg.tick(sample_rate, ar, d1r, d1l, d2r, rr);
        input * gain
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_reach_near_full_gain_quickly() {
        let sr = 44100.0;
        let mut vca = VoiceAmp::new();
        vca.note_on();
        // ar=255（最速）、d1l=255（完全サステイン）：数百サンプルでほぼ1.0に到達する
        let mut out = 0.0;
        for _ in 0..500 {
            out = vca.process(1.0, sr, 255, 0, 255, 0, 255);
        }
        assert!((out - 1.0).abs() < 1e-3, "expected near-unity gain, got {out}");
    }
}
