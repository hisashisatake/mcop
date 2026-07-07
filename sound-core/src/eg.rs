// ---------------------------------------------------------------------------
// パラメーターマッピング関数群（すべて純粋関数）
//
// 数式はすべて初期案（暫定）。CLAUDE.mdのテスト方針に従い、
// 実装後に音を聴いて係数を調整する。
// ---------------------------------------------------------------------------

/// レート値(0〜255)→1サンプルあたりのEG変化量。
/// rate=0は特殊値で「変化なし」（OPM/OPNのAR=0/D1R=0/D2R=0と同じフリーズ状態）。
/// rate=1〜255はt_max（rate=1、最遅）〜t_min（rate=255、最速）の指数マッピング。
fn rate_to_delta(rate: u8, sample_rate: f32, t_min: f32, t_max: f32) -> f32 {
    if rate == 0 {
        return 0.0;
    }
    let t = t_min * (t_max / t_min).powf(1.0 - (rate as f32 - 1.0) / 254.0);
    1.0 / (t * sample_rate)
}

/// AR: 0.68ms〜20.2秒。OPM AR(5bit)のreg=31〜1(eg_rate=62〜2、KSRなし)の理論値が基準。
/// reg=31(eg_rate=62)はキーオン時に瞬時attenuation=0となる特殊仕様だが、
/// 増分テーブルの値自体はreg=30(eg_rate=60)と同一のため0.68msを採用。
/// rate=0はreg=0相当のフリーズ（発音しない）。
pub fn ar_to_delta(rate: u8, sample_rate: f32) -> f32 {
    rate_to_delta(rate, sample_rate, 0.00068, 20.2)
}

/// D1R/D2R: 8.71ms〜284.9秒。OPM D1R/D2R(5bit)のreg=31〜1(eg_rate=62〜2、KSRなし)の理論値が基準。
/// rate=0はD1R/D2R=0相当のフリーズ（サスティンレベルを無限保持）。
pub fn decay_to_delta(rate: u8, sample_rate: f32) -> f32 {
    rate_to_delta(rate, sample_rate, 0.00871, 284.9)
}

/// RR: 8.71ms〜284.9秒。OPM RR(4bit)のreg=15〜0(eg_rate=62〜2、KSRなし)の理論値が基準。
/// [decay_to_delta]と同じeg_rate範囲だが、RRは`eg_rate = reg*4+2`でreg=0でも
/// eg_rate=2となり実機にフリーズが存在しないため、rate=0〜255の全域を指数補間する
/// （rate=0でも284.9秒で減衰し、無限保持の特殊値は持たない）。
pub fn rr_to_delta(rate: u8, sample_rate: f32) -> f32 {
    let t_min: f32 = 0.00871;
    let t_max: f32 = 284.9;
    let t = t_min * (t_max / t_min).powf(1.0 - rate as f32 / 255.0);
    1.0 / (t * sample_rate)
}

// ---------------------------------------------------------------------------
// 5段OPM形式のキーオン連動EG状態機械
// ---------------------------------------------------------------------------

/// 5段OPM形式のキーオン連動エンベロープ状態機械（AR→D1R→D1L→D2R→RR、+Idle）。
/// 発振源に依存しない汎用プリミティブ。ym38x6-core/operator.rsのEnvPhase/tick_envelopeと
/// 同じ状態遷移・同じD1L解釈（線形 d1l/255）を持つ（Vcf/Vca用。振幅EG本体は据え置き、
/// 将来の統合は任意）。
#[derive(Clone, Copy, PartialEq, Debug)]
enum EgPhase {
    Attack,
    Decay1,
    Decay2,
    Release,
    Idle,
}

pub struct Eg {
    phase: EgPhase,
    level: f32,
}

impl Eg {
    pub fn new() -> Self {
        Self { phase: EgPhase::Idle, level: 0.0 }
    }

    pub fn note_on(&mut self) {
        self.phase = EgPhase::Attack;
        self.level = 0.0;
    }

    pub fn note_off(&mut self) {
        if self.phase != EgPhase::Idle {
            self.phase = EgPhase::Release;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.phase == EgPhase::Idle
    }

    /// 1サンプル分エンベロープを進め、現在のレベル(0.0〜1.0)を返す。
    /// ar/d1r/d1l/d2r/rrは0〜255の生パラメーター値（[ar_to_delta]等で内部変換する）。
    pub fn tick(&mut self, sample_rate: f32, ar: u8, d1r: u8, d1l: u8, d2r: u8, rr: u8) -> f32 {
        let sustain_level = d1l as f32 / 255.0;
        match self.phase {
            EgPhase::Attack => {
                self.level += ar_to_delta(ar, sample_rate);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.phase = EgPhase::Decay1;
                }
            }
            EgPhase::Decay1 => {
                self.level -= decay_to_delta(d1r, sample_rate);
                if self.level <= sustain_level {
                    self.level = sustain_level;
                    self.phase = EgPhase::Decay2;
                }
            }
            EgPhase::Decay2 => {
                self.level -= decay_to_delta(d2r, sample_rate);
                if self.level <= 0.0 {
                    self.level = 0.0;
                }
            }
            EgPhase::Release => {
                self.level -= rr_to_delta(rr, sample_rate);
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.phase = EgPhase::Idle;
                }
            }
            EgPhase::Idle => {}
        }
        self.level
    }
}

impl Default for Eg {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ar_to_delta_bounds() {
        let sr = 44100.0;
        // rate=0はフリーズ（変化なし）
        assert_eq!(ar_to_delta(0, sr), 0.0);
        let slowest = ar_to_delta(1, sr);
        let fastest = ar_to_delta(255, sr);
        assert!((slowest - 1.0 / (20.2 * sr)).abs() < 1e-9);
        assert!((fastest - 1.0 / (0.00068 * sr)).abs() < 1e-9);
        assert!(fastest > slowest);
    }

    #[test]
    fn decay_to_delta_bounds() {
        let sr = 44100.0;
        // rate=0はフリーズ（変化なし）
        assert_eq!(decay_to_delta(0, sr), 0.0);
        assert!((decay_to_delta(255, sr) - 1.0 / (0.00871 * sr)).abs() < 1e-9);
        assert!((decay_to_delta(1, sr) - 1.0 / (284.9 * sr)).abs() < 1e-9);
    }

    #[test]
    fn rr_to_delta_bounds() {
        let sr = 44100.0;
        // rr=0はreg=0相当（284.9秒、フリーズではない）
        assert!((rr_to_delta(0, sr) - 1.0 / (284.9 * sr)).abs() < 1e-9);
        // rr=255はreg=15相当（8.71ms）
        assert!((rr_to_delta(255, sr) - 1.0 / (0.00871 * sr)).abs() < 1e-9);
        // 指数カーブ：全域で滑らかに増加する
        assert!(rr_to_delta(0, sr) < rr_to_delta(64, sr));
        assert!(rr_to_delta(64, sr) < rr_to_delta(128, sr));
        assert!(rr_to_delta(128, sr) < rr_to_delta(192, sr));
        assert!(rr_to_delta(192, sr) < rr_to_delta(255, sr));
    }

    #[test]
    fn eg_note_on_enters_attack_and_is_not_idle() {
        let mut eg = Eg::new();
        assert!(eg.is_idle());
        eg.note_on();
        assert!(!eg.is_idle());
    }

    #[test]
    fn eg_reaches_sustain_after_note_on() {
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        let mut level = 0.0;
        // d2r=0（フリーズ）にして、D1Rでsustainに到達した後そこに留まることを確認する
        for _ in 0..3000 {
            level = eg.tick(sr, 255, 255, 128, 0, 255);
        }
        // D1L=128 → sustain_level ≈ 128/255 ≈ 0.502
        assert!((level - 128.0 / 255.0).abs() < 0.05, "expected near sustain level, got {level}");
    }

    #[test]
    fn eg_note_off_releases_to_idle() {
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        for _ in 0..3000 {
            eg.tick(sr, 255, 255, 128, 255, 255);
        }
        eg.note_off();
        assert!(!eg.is_idle());
        for _ in 0..3000 {
            if eg.is_idle() {
                break;
            }
            eg.tick(sr, 255, 255, 128, 255, 255);
        }
        assert!(eg.is_idle());
    }

    #[test]
    fn eg_d1l_255_and_d2r_0_sticks_at_full_sustain() {
        // 旧4段ADSRの退化ケース：d1l=255かつd2r=0でsustainが1.0のまま張り付く
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        let mut level = 0.0;
        for _ in 0..10000 {
            level = eg.tick(sr, 255, 150, 255, 0, 150);
        }
        assert!((level - 1.0).abs() < 1e-6, "expected level stuck at 1.0, got {level}");
    }
}
