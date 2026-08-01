// ---------------------------------------------------------------------------
// パラメーターマッピング関数群（すべて純粋関数）
//
// 数式はすべて初期案（暫定）。CLAUDE.mdのテスト方針に従い、
// 実装後に音を聴いて係数を調整する。
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::lfo::delay_to_seconds;

// ---------------------------------------------------------------------------
// レートテーブル（AR/D1R/D2R/RR）
//
// rate(0〜255)は1ノート中不変のパッチ値だが、旧実装は`powf()`（超越関数）を
// `Eg::tick()`から毎サンプル呼んでいた。AR/D1R/D2R/RRそれぞれについて
// rate→秒数のテーブルを初回アクセス時に1回だけ構築し（`OnceLock`、全チャンネル共有）、
// 以降は配列参照のみで済ませる。数式は変更していないため出力は従来と数学的に同一。
// ---------------------------------------------------------------------------

/// rate(1〜255)→秒数の256要素テーブルを構築する（index 0は未使用）。
fn build_rate_seconds_table(t_min: f32, t_max: f32) -> [f32; 256] {
    let mut table = [0.0f32; 256];
    for (rate, slot) in table.iter_mut().enumerate().skip(1) {
        *slot = t_min * (t_max / t_min).powf(1.0 - (rate as f32 - 1.0) / 254.0);
    }
    table
}

fn ar_seconds_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| build_rate_seconds_table(0.00068, 20.2))
}

fn decay_seconds_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| build_rate_seconds_table(0.00871, 284.9))
}

fn rr_seconds_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0f32; 256];
        let t_min: f32 = 0.00871;
        let t_max: f32 = 284.9;
        for (rate, slot) in table.iter_mut().enumerate() {
            *slot = t_min * (t_max / t_min).powf(1.0 - rate as f32 / 255.0);
        }
        table
    })
}

/// AR: 0.68ms〜20.2秒。OPM AR(5bit)のreg=31〜1(eg_rate=62〜2、KSRなし)の理論値が基準。
/// reg=31(eg_rate=62)はキーオン時に瞬時attenuation=0となる特殊仕様だが、
/// 増分テーブルの値自体はreg=30(eg_rate=60)と同一のため0.68msを採用。
/// rate=0はreg=0相当のフリーズ（発音しない）。
pub fn ar_to_delta(rate: u8, sample_rate: f32) -> f32 {
    if rate == 0 {
        return 0.0;
    }
    1.0 / (ar_seconds_table()[rate as usize] * sample_rate)
}

/// D1R/D2R: 8.71ms〜284.9秒。OPM D1R/D2R(5bit)のreg=31〜1(eg_rate=62〜2、KSRなし)の理論値が基準。
/// rate=0はD1R/D2R=0相当のフリーズ（サスティンレベルを無限保持）。
pub fn decay_to_delta(rate: u8, sample_rate: f32) -> f32 {
    if rate == 0 {
        return 0.0;
    }
    1.0 / (decay_seconds_table()[rate as usize] * sample_rate)
}

/// RR: 8.71ms〜284.9秒。OPM RR(4bit)のreg=15〜0(eg_rate=62〜2、KSRなし)の理論値が基準。
/// [decay_to_delta]と同じeg_rate範囲だが、RRは`eg_rate = reg*4+2`でreg=0でも
/// eg_rate=2となり実機にフリーズが存在しないため、rate=0〜255の全域を指数補間する
/// （rate=0でも284.9秒で減衰し、無限保持の特殊値は持たない）。
pub fn rr_to_delta(rate: u8, sample_rate: f32) -> f32 {
    1.0 / (rr_seconds_table()[rate as usize] * sample_rate)
}

/// u8(0〜255)→レベル(0.0〜1.0)。`v as f32 / 255.0`と同値の256要素テーブル
/// （D1L/Floorのレベル換算は`Eg::tick`から毎サンプル×EG基数分呼ばれるため、
/// 除算を配列参照に置き換える。値はテーブル構築時に同じ除算で求めるためビット単位で同一）。
fn u8_to_level(v: u8) -> f32 {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0.0f32; 256];
        for (v, slot) in table.iter_mut().enumerate() {
            *slot = v as f32 / 255.0;
        }
        table
    });
    table[v as usize]
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
    /// キーオンからAR開始までの遅延(0〜255、既定0＝遅延なし)。0〜10秒（線形、
    /// `crate::lfo::delay_to_seconds`と同じマッピング）。
    #[serde(default)]
    pub delay: u8,
}

impl EgParams {
    /// 従来の5段EG（Loop/Floor/Curve/Delayなし）を表すヘルパー。FMオペレーターのように
    /// ループ機能を使わない呼び出し側は、これで`EgParams`を組み立てる。
    pub fn classic(ar: u8, d1r: u8, d1l: u8, d2r: u8, rr: u8) -> Self {
        Self { ar, d1r, d1l, d2r, rr, floor: 0, loop_enabled: 0, curve: 0, delay: 0 }
    }

    fn is_loop(&self) -> bool {
        self.loop_enabled != 0
    }
}

/// CC76（Vibrato Rate）の生値(0〜127、64=無補正)から、Pitch FGのAR/D1Rへ一括で掛ける
/// 乗算スケール係数を返す（spec-sound.md「演奏層による補正」節）。
/// AR/D1Rは指数マッピング（[ar_to_delta]/[decay_to_delta]）のため、生コードへの加算では
/// 基準値によって体感速度が大きく変わってしまう。`Eg::tick`の`rate_scale`引数（時間軸への
/// 乗算）を経由することで、パッチのベース速度によらず一律「s倍速く/遅く」という
/// 「スケール」の語義通りの挙動になる。
/// 64→1.0（無補正）、0→0.25倍（4倍遅く）、127→4.0倍（4倍速く）の指数カーブ。
pub fn cc76_to_rate_scale(cc76: u8) -> f32 {
    const SCALE_MIN: f32 = 0.25;
    const SCALE_MAX: f32 = 4.0;
    let cc76 = cc76.min(127) as f32;
    if cc76 <= 64.0 {
        // 0〜64 → SCALE_MIN〜1.0
        SCALE_MIN * (1.0 / SCALE_MIN).powf(cc76 / 64.0)
    } else {
        // 64〜127 → 1.0〜SCALE_MAX
        (SCALE_MAX).powf((cc76 - 64.0) / 63.0)
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
    /// キーオンからAR開始までの遅延（`EgParams::delay`）。レベルは0に固定。
    Delay,
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
    /// Delayフェーズに入ってからの経過秒数（Delayフェーズ専用、他フェーズでは未使用）。
    elapsed: f32,
}

impl Eg {
    pub fn new() -> Self {
        Self { phase: EgPhase::Idle, level: 0.0, segment_start: 0.0, segment_end: 0.0, elapsed: 0.0 }
    }

    /// Delayフェーズへ入る（`delay=0`の場合は`tick`の冒頭で同一tick内にAttackへ
    /// フォールスルーするため、既存パッチの挙動は1サンプルもズレない）。
    pub fn note_on(&mut self) {
        self.phase = EgPhase::Delay;
        self.level = 0.0;
        self.segment_start = 0.0;
        self.segment_end = 0.0;
        self.elapsed = 0.0;
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

    /// 現在のエンベロープレベル(0.0〜1.0、Curve整形前の生値)。ボイススチールで
    /// 「どのボイスが最も静かか」を比較する用途（`shaped_output`はCurve由来の
    /// 一時的な上下動があるため使わない）。
    pub fn level(&self) -> f32 {
        self.level
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
        let sustain_level = u8_to_level(params.d1l);
        let floor_level = u8_to_level(params.floor);
        if self.phase == EgPhase::Delay {
            let delay_seconds = delay_to_seconds(params.delay);
            if self.elapsed < delay_seconds {
                self.elapsed += 1.0 / sample_rate;
                return self.shaped_output(params.curve);
            }
            // delay=0の場合はelapsed(0.0) < delay_seconds(0.0)が偽になり、
            // ここへ直行して同一tick内でAttack処理へフォールスルーする
            // （既存パッチは1サンプルもズレない）。
            self.phase = EgPhase::Attack;
            self.segment_start = 0.0;
            self.segment_end = 1.0;
        }
        match self.phase {
            EgPhase::Delay => {}
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
        let params = EgParams { ar: 255, d1r: 255, d1l: 0, d2r: 0, rr: 255, floor: 128, loop_enabled: 1, curve: 0, delay: 0 };
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
        let params = EgParams { ar: 200, d1r: 200, d1l: 0, d2r: 0, rr: 255, floor: 64, loop_enabled: 1, curve: 0, delay: 0 };
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

    #[test]
    fn eg_delay_zero_matches_immediate_attack() {
        // delay=0の場合、1回目のtickでAttackへフォールスルーし、旧実装（note_on直後に
        // Attackへ入る）と1サンプルもズレないことを確認する。
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        let params = EgParams::classic(200, 150, 128, 0, 150);
        let first_tick = eg.tick(sr, params, 1.0);
        let expected_first_tick = ar_to_delta(200, sr);
        assert!(
            (first_tick - expected_first_tick).abs() < 1e-9,
            "delay=0 should ramp on the very first tick like the old immediate-Attack behavior: got {first_tick}, expected {expected_first_tick}"
        );
    }

    #[test]
    fn eg_delay_holds_at_zero_then_attacks() {
        let sr = 44100.0;
        let mut eg = Eg::new();
        eg.note_on();
        // delay=13（短めの遅延、≈0.51秒）。elapsedはサンプルごとの浮動小数点加算（sound_core::lfo/
        // chip_lfoと同じ方式）のため、数十万サンプル級の長い遅延で境界ぎりぎりを厳密比較すると
        // 累積誤差で早期にドリフトしうる。安全マージンとして90%地点までのみ「0に張り付く」ことを検証する。
        let params = EgParams { ar: 255, d1r: 0, d1l: 255, d2r: 0, rr: 255, floor: 0, loop_enabled: 0, curve: 0, delay: 13 };
        let delay_seconds = delay_to_seconds(13);
        let delay_samples = (delay_seconds * sr) as usize;
        let hold_check_samples = delay_samples * 9 / 10;

        // 遅延中（90%地点まで）はレベル0に張り付く
        for _ in 0..hold_check_samples {
            let level = eg.tick(sr, params, 1.0);
            assert_eq!(level, 0.0, "should hold at 0 during the delay window");
        }

        // 遅延経過後はAttackへ移行し、レベルが立ち上がる（残り10%分+余裕を見て回す）。
        let mut reached_nonzero = false;
        for _ in 0..(delay_samples / 5) {
            if eg.tick(sr, params, 1.0) > 0.0 {
                reached_nonzero = true;
                break;
            }
        }
        assert!(reached_nonzero, "should start ramping up after the delay elapses");
    }

    #[test]
    fn cc76_to_rate_scale_neutral_at_64() {
        assert!((cc76_to_rate_scale(64) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cc76_to_rate_scale_monotonic_and_bounded() {
        let min = cc76_to_rate_scale(0);
        let mid = cc76_to_rate_scale(64);
        let max = cc76_to_rate_scale(127);
        assert!(min < mid);
        assert!(mid < max);
        assert!((min - 0.25).abs() < 1e-6);
        assert!((max - 4.0).abs() < 1e-6);
    }
}
