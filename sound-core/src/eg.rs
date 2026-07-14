// ---------------------------------------------------------------------------
// パラメーターマッピング関数群（すべて純粋関数）
//
// 数式はすべて初期案（暫定）。CLAUDE.mdのテスト方針に従い、
// 実装後に音を聴いて係数を調整する。
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

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
// EgParams（5段OPM形式のEGパラメーター + Loop/Floor/Curve拡張）
// ---------------------------------------------------------------------------

/// 5段OPM形式EG（AR/D1R/D1L/D2R/RR）に、ループ可能ファンクションジェネレーター用の
/// 3項目（Floor/Loop/Curve）を足したパラメーター一式（spec-sound.md「ファンクションジェネレーター」
/// 節参照）。`loop_enabled=0`かつ`floor=0`かつ`curve=0`（=[EgParams::classic]）では、
/// 従来の5段EGと完全に同一の挙動になる。
///
/// FMオペレーター（`ym38x6-core::operator`）とVCF/VCAのCutoff/Gain FG（`GainFg`型として
/// そのまま使う）の両方がこの型を共有する（新規の別部品を作らない設計、spec-sound.md参照）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EgParams {
    pub ar: u8,
    pub d1r: u8,
    pub d1l: u8,
    pub d2r: u8,
    pub rr: u8,
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。
    #[serde(default)]
    pub floor: u8,
    /// 0=ワンショット（従来のADSR挙動そのまま）／1=ループ。
    #[serde(rename = "loop", default)]
    pub loop_enabled: u8,
    /// 0=線形（角の立つ三角）／1=サイン風（レイズドコサインで角を丸める）。
    #[serde(default)]
    pub curve: u8,
}

impl EgParams {
    /// 従来の5段EG（Loop/Floor/Curveなし）を表すヘルパー。FMオペレーターのように
    /// ループ機能を使わない呼び出し側は、これで`EgParams`を組み立てる。
    pub fn classic(ar: u8, d1r: u8, d1l: u8, d2r: u8, rr: u8) -> Self {
        Self { ar, d1r, d1l, d2r, rr, floor: 0, loop_enabled: 0, curve: 0 }
    }

    fn is_loop(&self) -> bool {
        self.loop_enabled != 0
    }
}

// ---------------------------------------------------------------------------
// FG（ファンクションジェネレーター）スロットの型（spec-sound.md「ファンクションジェネレーター」節）
// ---------------------------------------------------------------------------

/// Gain FG（旧VCA EG後継）のパラメーター一式。音量に負値は無いためDepthを持たず、
/// `EgParams`と完全に同じ形（ar/d1r/d1l/d2r/rr/floor/loop/curve）のため新規の別型を作らず
/// そのままエイリアスする。
pub type GainFg = EgParams;

/// Pitch FG／Cutoff FGのパラメーター一式。共通のループ可能EG（`EgParams`）に、
/// バイポーラDepth（0〜255、中心128＝変調なし）を足したもの。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BipolarFg {
    #[serde(flatten)]
    pub eg: EgParams,
    /// バイポーラDepth（0〜255、中心128＝変調なし、128超＝＋方向、128未満＝−方向）。
    pub depth: u8,
}

impl Default for BipolarFg {
    fn default() -> Self {
        Self { eg: EgParams::default(), depth: 128 }
    }
}

// ---------------------------------------------------------------------------
// 5段OPM形式のキーオン連動EG状態機械（+ Loop/Floor/Curve拡張）
// ---------------------------------------------------------------------------

/// 5段OPM形式のキーオン連動エンベロープ状態機械（AR→D1R→D1L→D2R→RR、+Idle）に、
/// ループモード（`LoopUp`/`LoopDown`でFloor⇄peakを往復）を足した状態機械。
/// 発振源に依存しない汎用プリミティブ（Vcf/Vca/Pitch FG/FMオペレーター共通）。
#[derive(Clone, Copy, PartialEq, Debug)]
enum EgPhase {
    Attack,
    Decay1,
    Decay2,
    /// ループモード専用：Floor→peak(1.0)、AR（開く/戻り）で往復する。
    LoopUp,
    /// ループモード専用：peak(1.0)→Floor、D1R（閉じる/下降）で往復する。
    LoopDown,
    Release,
    Idle,
}

pub struct Eg {
    phase: EgPhase,
    level: f32,
    /// 現在のセグメント（フェーズ）の開始レベル。Curve整形の基準点。
    segment_start: f32,
    /// 現在のセグメントの目標レベル。Curve整形の基準点。
    segment_end: f32,
}

impl Eg {
    pub fn new() -> Self {
        Self { phase: EgPhase::Idle, level: 0.0, segment_start: 0.0, segment_end: 0.0 }
    }

    pub fn note_on(&mut self) {
        self.phase = EgPhase::Attack;
        self.level = 0.0;
        self.segment_start = 0.0;
        self.segment_end = 1.0;
    }

    /// 残響レベルを保持したままAttack相位へ再突入する（実機OPMのKey-On挙動の再現）。
    /// `level`は現在値を維持し、そこから改めてAR区間を開始する（同音連打でのプチノイズを消す）。
    pub fn retrigger(&mut self) {
        self.phase = EgPhase::Attack;
        self.segment_start = self.level;
        self.segment_end = 1.0;
    }

    pub fn note_off(&mut self) {
        if self.phase != EgPhase::Idle {
            self.segment_start = self.level;
            self.segment_end = 0.0;
            self.phase = EgPhase::Release;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.phase == EgPhase::Idle
    }

    /// Curve整形前の生レベル(0.0〜1.0)。フェーズ遷移判定に使う内部値そのもの。
    fn shaped_output(&self, curve: u8) -> f32 {
        if curve == 0 {
            return self.level;
        }
        let span = self.segment_end - self.segment_start;
        if span.abs() < 1e-9 {
            return self.level;
        }
        let progress = ((self.level - self.segment_start) / span).clamp(0.0, 1.0);
        let shaped = 0.5 - 0.5 * (std::f32::consts::PI * progress).cos();
        self.segment_start + shaped * span
    }

    /// 1サンプル分エンベロープを進め、現在のレベル(0.0〜1.0、Curve整形適用後)を返す。
    /// `params`はAR/D1R/D1L/D2R/RRとLoop/Floor/Curveを持つ0〜255の生パラメーター値。
    /// `rate_scale`はAR/D1R/D2R/RRの時定数に掛ける倍率（KSR等、FM以外は1.0を渡す）。
    pub fn tick(&mut self, sample_rate: f32, params: EgParams, rate_scale: f32) -> f32 {
        let sustain_level = params.d1l as f32 / 255.0;
        let floor_level = params.floor as f32 / 255.0;
        match self.phase {
            EgPhase::Attack => {
                self.level += ar_to_delta(params.ar, sample_rate) * rate_scale;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    if params.is_loop() {
                        self.phase = EgPhase::LoopDown;
                        self.segment_start = 1.0;
                        self.segment_end = floor_level;
                    } else {
                        self.phase = EgPhase::Decay1;
                        self.segment_start = 1.0;
                        self.segment_end = sustain_level;
                    }
                }
            }
            EgPhase::LoopUp => {
                self.level += ar_to_delta(params.ar, sample_rate) * rate_scale;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.phase = EgPhase::LoopDown;
                    self.segment_start = 1.0;
                    self.segment_end = floor_level;
                }
            }
            EgPhase::LoopDown => {
                self.level -= decay_to_delta(params.d1r, sample_rate) * rate_scale;
                if self.level <= floor_level {
                    self.level = floor_level;
                    self.phase = EgPhase::LoopUp;
                    self.segment_start = floor_level;
                    self.segment_end = 1.0;
                }
            }
            EgPhase::Decay1 => {
                self.level -= decay_to_delta(params.d1r, sample_rate) * rate_scale;
                if self.level <= sustain_level {
                    self.level = sustain_level;
                    self.phase = EgPhase::Decay2;
                    self.segment_start = sustain_level;
                    self.segment_end = 0.0;
                }
            }
            EgPhase::Decay2 => {
                self.level -= decay_to_delta(params.d2r, sample_rate) * rate_scale;
                if self.level <= 0.0 {
                    self.level = 0.0; // Idleにはせずキーオン継続中は0に張り付く
                }
            }
            EgPhase::Release => {
                self.level -= rr_to_delta(params.rr, sample_rate) * rate_scale;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.phase = EgPhase::Idle;
                }
            }
            EgPhase::Idle => {}
        }
        self.shaped_output(params.curve)
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
        let params = EgParams::classic(255, 255, 128, 0, 255);
        for _ in 0..3000 {
            level = eg.tick(sr, params, 1.0);
        }
        // D1L=128 → sustain_level ≈ 128/255 ≈ 0.502
        assert!((level - 128.0 / 255.0).abs() < 0.05, "expected near sustain level, got {level}");
    }

    #[test]
    fn eg_note_off_releases_to_idle() {
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        let params = EgParams::classic(255, 255, 128, 255, 255);
        for _ in 0..3000 {
            eg.tick(sr, params, 1.0);
        }
        eg.note_off();
        assert!(!eg.is_idle());
        for _ in 0..3000 {
            if eg.is_idle() {
                break;
            }
            eg.tick(sr, params, 1.0);
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
        let params = EgParams::classic(255, 150, 255, 0, 150);
        for _ in 0..10000 {
            level = eg.tick(sr, params, 1.0);
        }
        assert!((level - 1.0).abs() < 1e-6, "expected level stuck at 1.0, got {level}");
    }

    #[test]
    fn eg_loop_oscillates_between_floor_and_peak() {
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        // floor=128(≈0.502)、AR/D1Rとも高速で何度も往復させる
        let params = EgParams { ar: 255, d1r: 255, d1l: 0, d2r: 0, rr: 255, floor: 128, loop_enabled: 1, curve: 0 };
        // 最初のAttackは0→peakの立ち上がり（floor未満を通過する）なので、
        // 一度peakへ到達し「ループに入った」後だけをfloor⇄peak往復の観測対象にする。
        let mut reached_peak_once = false;
        let mut max_seen = 0.0f32;
        let mut min_seen = 1.0f32;
        for _ in 0..20000 {
            let level = eg.tick(sr, params, 1.0);
            if level >= 0.999 {
                reached_peak_once = true;
            }
            if reached_peak_once {
                max_seen = max_seen.max(level);
                min_seen = min_seen.min(level);
            }
        }
        assert!(max_seen > 0.99, "expected to reach peak, got max={max_seen}");
        assert!(
            (min_seen - 128.0 / 255.0).abs() < 0.01,
            "expected to reach floor level, got min={min_seen}"
        );
        assert!(!eg.is_idle(), "loop mode should never become idle on its own");
    }

    #[test]
    fn eg_loop_note_off_releases_from_current_level() {
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        let params = EgParams { ar: 200, d1r: 200, d1l: 0, d2r: 0, rr: 255, floor: 64, loop_enabled: 1, curve: 0 };
        // ループを何周かさせてから離鍵する
        for _ in 0..2000 {
            eg.tick(sr, params, 1.0);
        }
        eg.note_off();
        assert!(!eg.is_idle());
        let mut settled = false;
        for _ in 0..20000 {
            let level = eg.tick(sr, params, 1.0);
            if eg.is_idle() {
                assert_eq!(level, 0.0);
                settled = true;
                break;
            }
        }
        assert!(settled, "loop mode should release to idle after note_off");
    }

    #[test]
    fn eg_curve_does_not_change_phase_transition_timing() {
        let sr = 44100.0;
        let params_linear = EgParams::classic(150, 150, 128, 0, 150);
        let params_curved = EgParams { curve: 1, ..params_linear };

        let ticks_to_sustain = |params: EgParams| {
            let mut eg = Eg::new();
            eg.note_on();
            let mut count = 0;
            loop {
                eg.tick(sr, params, 1.0);
                count += 1;
                // Decay2に入った最初のtick数を数える（sustain到達のタイミング）
                if count > 100_000 {
                    panic!("did not reach sustain in time");
                }
                if (eg.level - (128.0 / 255.0)).abs() < 1e-6 && eg.phase == EgPhase::Decay2 {
                    return count;
                }
            }
        };

        let linear_count = ticks_to_sustain(params_linear);
        let curved_count = ticks_to_sustain(params_curved);
        assert_eq!(linear_count, curved_count, "curve should not affect phase transition timing");
    }

    #[test]
    fn eg_curve_shapes_output_value_but_not_at_segment_endpoints() {
        let sr = 44100.0;
        let mut eg_linear = Eg::new();
        eg_linear.note_on();
        let mut eg_curved = Eg::new();
        eg_curved.note_on();

        let params_linear = EgParams::classic(150, 0, 255, 0, 0);
        let params_curved = EgParams { curve: 1, ..params_linear };

        let mut differed = false;
        for _ in 0..500 {
            let linear = eg_linear.tick(sr, params_linear, 1.0);
            let curved = eg_curved.tick(sr, params_curved, 1.0);
            if (linear - curved).abs() > 1e-4 {
                differed = true;
            }
            // 端点(0.0/1.0付近)以外では値が変わるはず、範囲は保つ
            assert!((0.0..=1.0).contains(&curved));
        }
        assert!(differed, "curve=1 should shape the output away from the linear ramp mid-segment");
    }

    #[test]
    fn eg_retrigger_preserves_level_and_reenters_attack() {
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        let params = EgParams::classic(255, 100, 128, 0, 100);
        // Attack完了、Decay1途中まで進めて残響レベルを作る
        for _ in 0..2000 {
            eg.tick(sr, params, 1.0);
        }
        let level_before = eg.level;
        assert!(level_before > 0.0 && level_before < 1.0);

        eg.retrigger();
        assert!(!eg.is_idle());
        assert_eq!(eg.phase, EgPhase::Attack);
        // retrigger直後のtick結果はlevel_beforeからさらにARで持ち上がった値のはず（0にリセットされない）
        let first_tick = eg.tick(sr, params, 1.0);
        assert!(
            first_tick >= level_before,
            "retrigger should continue upward from the preserved level, not reset to 0: before={level_before}, after={first_tick}"
        );
    }
}
