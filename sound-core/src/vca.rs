// ---------------------------------------------------------------------------
// Vca（ボイス単位のTVAオーバーレイ）
// ---------------------------------------------------------------------------

use crate::eg::{Eg, EgParams};

/// ボイス単位のTVA（Total Voice Amplitude）オーバーレイの共通インターフェース。
/// キーオン連動EG（5段OPM形式）でゲインを乗算する。
pub trait Vca: Send {
    fn note_on(&mut self);
    fn note_off(&mut self);
    fn process(&mut self, input: f32, sample_rate: f32, params: EgParams) -> f32;
}

/// ボイス単位のTVAオーバーレイ（キーオン連動EGでゲインを乗算する）。
/// 透過的な既定（ar=255,d1r=0,d1l=255,d2r=0,rr=0）ではアタックは数サンプルで完了しゲイン1.0に
/// 張り付き、リリースは最遅（実質ゲートを閉じない）ため、FM本来のキャリアEG（operator.rs）の
/// アタックもリリースも打ち消さない透過的マクロ・オーバーレイとして働く（二重EG化を避ける既定設計）。
/// 注意: rrを速く（例:255=8.71ms）すると離鍵時に全チャンネルを短時間でゼロへ閉じ、
/// 各オペレーター本来のリリース尾を打ち消す（リリース瞬断）。透過用途では必ずrrを最遅に保つ。
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

    fn process(&mut self, input: f32, sample_rate: f32, params: EgParams) -> f32 {
        let gain = self.eg.tick(sample_rate, params, 1.0);
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
        let params = EgParams::classic(255, 0, 255, 0, 255);
        let mut out = 0.0;
        for _ in 0..500 {
            out = vca.process(1.0, sr, params);
        }
        assert!((out - 1.0).abs() < 1e-3, "expected near-unity gain, got {out}");
    }
}
