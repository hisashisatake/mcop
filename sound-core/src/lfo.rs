use std::f32::consts::TAU;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// パラメーターマッピング
// ---------------------------------------------------------------------------

/// rate=0 → 0.01Hz, rate=255 → 20Hz（指数マッピング）。
///
/// rateは1ノート中不変のパッチ値だが、以前は`PerformanceLfo::tick()`から毎サンプル
/// `powf()`を呼んでいた。`tl_to_gain`等と同じ256要素テーブルパターンで初回アクセス時に
/// 1回だけ構築し（`OnceLock`、全チャンネル共有）、以降は配列参照のみで済ませる。
/// 数式は変更していないため出力は従来とビット単位で同一。
fn rate_to_hz(rate: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const F_MIN: f32 = 0.01;
        const F_MAX: f32 = 20.0;
        let mut table = [0.0f32; 256];
        for (rate, slot) in table.iter_mut().enumerate() {
            *slot = F_MIN * (F_MAX / F_MIN).powf(rate as f32 / 255.0);
        }
        table
    });
    table[rate as usize]
}

/// delay=0 → 0秒, delay=255 → 10秒（線形マッピング）。
/// `sound_core::eg`のFG Delayフェーズ、`op505-core`のAdapter（delay→プラトー段変換）からも
/// 再利用するため`pub`。
/// `PerformanceLfo::tick`から毎サンプル呼ばれる（delay+fade_timeの2回）ため、
/// `rate_to_hz`と同じ256要素テーブル化で除算を配列参照に置き換える（値はビット単位で同一）。
pub fn delay_to_seconds(delay: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const D_MAX: f32 = 10.0;
        let mut table = [0.0f32; 256];
        for (delay, slot) in table.iter_mut().enumerate() {
            *slot = delay as f32 / 255.0 * D_MAX;
        }
        table
    });
    table[delay as usize]
}

/// 三角波: phase=0→-1, 0.25→0, 0.5→1, 0.75→0
fn triangle(phase: f32) -> f32 {
    2.0 * (2.0 * (phase - (phase + 0.5).floor())).abs() - 1.0
}

/// xorshift32による簡易疑似乱数（S&H波形用、外部crateに依存しない）
fn next_random(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

// ---------------------------------------------------------------------------
// パフォーマンスLFO（ビブラート/トレモロ）
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum LfoWaveform {
    #[default]
    Triangle,
    Sine,
    Square,
    SampleHold,
    /// 位相0→-1、1直前→+1のランプ（のこぎり波）。
    Saw,
    /// 三角波を上下でクリップした台形。
    Trapezoid,
    /// 周期ごとに新しい目標値を抽選し、位相で前回値→次回値を線形補間する
    /// 滑らかなランダムウォーク（階段状のSampleHoldとの違いはここ）。
    Random,
    /// ロジスティック写像(`x = 3.9 * x * (1 - x)`)を周期ごとに反復する決定論的カオス。
    /// 周期内はSampleHold同様にホールドする。
    Chaos,
}

/// パフォーマンスLFOのFadeモード（4種）。
///
/// ON-IN/ON-OUTはnote_on起点、OFF-IN/OFF-OUTはnote_off起点でfade_time秒かけて
/// 振幅ゲインをランプする。既存のDelay（`delay`フィールド、ハードカットオーバー方式）
/// とは独立したパラメーターで、Delayが経過した後にFadeが効き始める。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum LfoFadeMode {
    /// note_on後、Delay経過を起点に振幅ゲインが0→1にフェードイン。
    #[default]
    OnIn,
    /// note_on後、Delay経過を起点に振幅ゲインが1→0にフェードアウト。
    OnOut,
    /// note_off前は振幅ゲイン0、note_off後にfade_time秒かけて0→1にフェードイン。
    OffIn,
    /// note_off前は振幅ゲイン1、note_off後にfade_time秒かけて1→0にフェードアウト。
    OffOut,
}

/// パフォーマンスLFOの「波形の形」に関する設定値のまとめ。
///
/// `rate`/`delay`とは別に、音色パッチ側（`ym38x6-core`の`ChannelParams`等）が
/// 1フィールドとして保持できるように独立させてある。LFOの意味論（enum定義・
/// 波形計算・Fadeロジック）は`sound-core`に閉じ、呼び出し側はこの構造体を
/// 持ち回るだけでよい。
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct PerformanceLfoShape {
    pub waveform: LfoWaveform,
    pub fade_mode: LfoFadeMode,
    /// Fadeにかかる時間。`delay_to_seconds`と同じ0〜10秒レンジ（0=フェード無効・
    /// 常時フルゲインとなり、旧来のハードエッジ挙動と等価になる）。
    pub fade_time: u8,
    /// 波形の中心シフト(-100〜100)。生値(-1.0〜1.0)に`offset/100.0`を加算してから
    /// クランプする。
    pub offset: i8,
}

/// Rate/Delay/Waveformを持つ、エンジン非依存のパフォーマンスLFO。
///
/// Depth（変調の深さ）の適用方法はDestination（Pitch/Volumeなど）ごとに
/// モデルが異なるため、ここでは持たない。`tick`は-1.0〜1.0の変調値のみを
/// 返し、Depthの適用は呼び出し側（PerformanceLfoTarget実装側）が行う。
pub struct PerformanceLfo {
    rate: u8,
    delay: u8,
    waveform: LfoWaveform,
    fade_mode: LfoFadeMode,
    fade_time: u8,
    offset: i8,
    phase: f32,
    elapsed: f32,
    rng_state: u32,
    sh_value: f32,
    /// Random波形の補間元・補間先（周期境界で`random_prev = random_next`にスライドする）。
    random_prev: f32,
    random_next: f32,
    /// Chaos波形のロジスティック写像の状態(0.0〜1.0)。
    chaos_state: f32,
    /// note_on〜note_offの間はtrue（Fade OFF系の起点判定に使う）。
    gate_open: bool,
    /// note_off後の経過秒（Fade OFF系のランプに使う）。
    released_elapsed: f32,
}

impl PerformanceLfo {
    pub fn new() -> Self {
        Self {
            rate: 0,
            delay: 0,
            waveform: LfoWaveform::default(),
            fade_mode: LfoFadeMode::default(),
            fade_time: 0,
            offset: 0,
            phase: 0.0,
            elapsed: 0.0,
            rng_state: 0x1234_5678,
            sh_value: 0.0,
            random_prev: 0.0,
            random_next: 0.0,
            chaos_state: 0.5,
            gate_open: false,
            released_elapsed: 0.0,
        }
    }

    pub fn set_rate(&mut self, rate: u8) {
        self.rate = rate;
    }

    pub fn set_delay(&mut self, delay: u8) {
        self.delay = delay;
    }

    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
    }

    /// `waveform`/`fade_mode`/`fade_time`/`offset`を一括設定する。
    pub fn set_shape(&mut self, shape: PerformanceLfoShape) {
        self.waveform = shape.waveform;
        self.fade_mode = shape.fade_mode;
        self.fade_time = shape.fade_time;
        self.offset = shape.offset;
    }

    /// キーオン時に呼び出し、位相とディレイ経過時間をリセットする
    pub fn note_on(&mut self) {
        self.phase = 0.0;
        self.elapsed = 0.0;
        self.gate_open = true;
        self.released_elapsed = 0.0;
        self.chaos_state = 0.5;
    }

    /// キーオフ時に呼び出す。Fade OFF-IN/OFF-OUTの起点をリセットする。
    pub fn note_off(&mut self) {
        self.gate_open = false;
        self.released_elapsed = 0.0;
    }

    /// 1サンプル進めて変調値（-1.0〜1.0）を返す。ディレイ経過前は常に0.0。
    pub fn tick(&mut self, sample_rate: f32) -> f32 {
        let freq = rate_to_hz(self.rate);
        let prev_phase = self.phase;
        self.phase = (self.phase + freq / sample_rate).fract();
        self.elapsed += 1.0 / sample_rate;
        if !self.gate_open {
            self.released_elapsed += 1.0 / sample_rate;
        }

        let period_wrapped = self.phase < prev_phase;
        if period_wrapped {
            match self.waveform {
                LfoWaveform::SampleHold => self.sh_value = next_random(&mut self.rng_state),
                LfoWaveform::Random => {
                    self.random_prev = self.random_next;
                    self.random_next = next_random(&mut self.rng_state);
                }
                LfoWaveform::Chaos => {
                    self.chaos_state = 3.9 * self.chaos_state * (1.0 - self.chaos_state);
                }
                _ => {}
            }
        }

        let delay_seconds = delay_to_seconds(self.delay);
        if self.elapsed < delay_seconds {
            return 0.0;
        }

        let raw = match self.waveform {
            LfoWaveform::Triangle => triangle(self.phase),
            LfoWaveform::Sine => (self.phase * TAU).sin(),
            LfoWaveform::Square => if self.phase < 0.5 { 1.0 } else { -1.0 },
            LfoWaveform::SampleHold => self.sh_value,
            LfoWaveform::Saw => 2.0 * self.phase - 1.0,
            LfoWaveform::Trapezoid => (triangle(self.phase) * 2.0).clamp(-1.0, 1.0),
            LfoWaveform::Random => self.random_prev + self.phase * (self.random_next - self.random_prev),
            LfoWaveform::Chaos => 2.0 * self.chaos_state - 1.0,
        };

        let shifted = (raw + self.offset as f32 / 100.0).clamp(-1.0, 1.0);
        shifted * self.fade_gain(delay_seconds)
    }

    /// Fadeモードに応じた振幅ゲイン（0.0〜1.0）を計算する。
    /// `fade_time=0`は常時フルゲイン（フェード無効、旧来のハードエッジ挙動と等価）。
    fn fade_gain(&self, delay_seconds: f32) -> f32 {
        let fade_seconds = delay_to_seconds(self.fade_time);
        if fade_seconds <= 0.0 {
            return 1.0;
        }

        match self.fade_mode {
            LfoFadeMode::OnIn => {
                let t = (self.elapsed - delay_seconds).max(0.0);
                (t / fade_seconds).min(1.0)
            }
            LfoFadeMode::OnOut => {
                let t = (self.elapsed - delay_seconds).max(0.0);
                1.0 - (t / fade_seconds).min(1.0)
            }
            LfoFadeMode::OffIn => {
                if self.gate_open {
                    0.0
                } else {
                    (self.released_elapsed / fade_seconds).min(1.0)
                }
            }
            LfoFadeMode::OffOut => {
                if self.gate_open {
                    1.0
                } else {
                    1.0 - (self.released_elapsed / fade_seconds).min(1.0)
                }
            }
        }
    }
}

impl Default for PerformanceLfo {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// パフォーマンスLFOの適用先（Destination）
// ---------------------------------------------------------------------------

/// パフォーマンスLFOの共通Destination。
/// 拡張Destination（38x6のTLキャリア一括など）は各エンジン側で個別に扱う。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LfoDestination {
    /// ビブラート。実効Depthはセント単位。
    #[default]
    Pitch,
    /// トレモロ。実効Depthは実効音量への加算量（-1.0〜1.0）。
    Volume,
}

/// パフォーマンスLFOの適用先。38x6の各ボイスが実装する。
pub trait PerformanceLfoTarget {
    /// Destination=Pitch（ビブラート）。`cents`はピッチオフセット（セント単位）。
    fn apply_pitch_modulation(&mut self, cents: f32);

    /// Destination=Volume（トレモロ）。`delta`は実効音量への加算量。
    fn apply_volume_modulation(&mut self, delta: f32);
}

/// LFO出力値（-1.0〜1.0）と実効DepthからDestinationに応じた変調をtargetへ適用する。
pub fn apply_lfo_modulation(
    lfo_value: f32,
    destination: LfoDestination,
    depth: f32,
    target: &mut impl PerformanceLfoTarget,
) {
    match destination {
        LfoDestination::Pitch => target.apply_pitch_modulation(lfo_value * depth),
        LfoDestination::Volume => target.apply_volume_modulation(lfo_value * depth),
    }
}

// ---------------------------------------------------------------------------
// ホスト側（VST NRPN/DAWパラメーター・Tauri DTO等）向けの整数表現との相互変換
// ---------------------------------------------------------------------------

/// `LfoWaveform`の宣言順インデックス(0〜7)から復元する。範囲外は`Triangle`にフォールバックする。
/// NRPN/DAWパラメーター等、0〜7の整数表現からWaveformを選択するホスト側の橋渡しに使う
/// （逆方向はフィールドレスenumのため`waveform as u8`で直接変換できる）。
pub fn lfo_waveform_from_index(index: u8) -> LfoWaveform {
    match index {
        1 => LfoWaveform::Sine,
        2 => LfoWaveform::Square,
        3 => LfoWaveform::SampleHold,
        4 => LfoWaveform::Saw,
        5 => LfoWaveform::Trapezoid,
        6 => LfoWaveform::Random,
        7 => LfoWaveform::Chaos,
        _ => LfoWaveform::Triangle,
    }
}

/// `LfoFadeMode`の宣言順インデックス(0〜3)から復元する。範囲外は`OnIn`にフォールバックする
/// （逆方向は`fade_mode as u8`で直接変換できる）。
pub fn lfo_fade_mode_from_index(index: u8) -> LfoFadeMode {
    match index {
        1 => LfoFadeMode::OnOut,
        2 => LfoFadeMode::OffIn,
        3 => LfoFadeMode::OffOut,
        _ => LfoFadeMode::OnIn,
    }
}

/// `PerformanceLfoShape::offset`（i8、-100〜100、中心0）を、ホスト側の0〜255（中心128）表現へ変換する。
pub fn lfo_offset_to_param(offset: i8) -> u8 {
    (128 + (offset as i32 * 128) / 100).clamp(0, 255) as u8
}

/// ホスト側の0〜255（中心128）表現を、`PerformanceLfoShape::offset`（i8、-100〜100）へ変換する。
pub fn lfo_offset_from_param(value: u8) -> i8 {
    (((value as i32 - 128) * 100) / 128).clamp(-100, 100) as i8
}

// ---------------------------------------------------------------------------
// 実効Depthの計算（CC1/CC77/RPN0,5）
// ---------------------------------------------------------------------------

/// Destination=Pitchの実効Depth（セント）。
/// `実効Depth(セント) = CC77値 + CC1値 × RPN0,5値 / 127`
///
/// CC77（パフォーマンスLFO Depthベース値）/ CC1（加算分）は本プロジェクトの
/// 内部表現（0〜255）。RPN0,5（Modulation Depth Range）はGM2準拠の0〜127スケール。
pub fn pitch_depth_cents(cc77: u8, cc1: u8, rpn0_5: u8) -> f32 {
    cc77 as f32 + cc1 as f32 * rpn0_5 as f32 / 127.0
}

/// Destination=Volumeの実効Depth（0.0〜1.0に正規化）。
/// `実効Depth = clamp(CC77値 + CC1値, 0, 255)` を、`PerformanceLfoTarget`が
/// 期待する0.0〜1.0の比率に正規化したもの。
///
/// CC77/CC1は本プロジェクトの内部表現（0〜255）。
pub fn volume_depth(cc77: u8, cc1: u8) -> f32 {
    (cc77 as u16 + cc1 as u16).min(255) as f32 / 255.0
}

/// Destination=Cutoff（38x6拡張、オートワウ）の実効Depth。
/// `実効Depth = clamp(CC77値 + CC1値, 0, 255)`を、Filter EG Depthと同じ0〜255の
/// カットオフ単位系のままデルタ最大値として返す（Volumeと異なり0.0〜1.0への正規化はしない）。
///
/// CC77/CC1は本プロジェクトの内部表現（0〜255）。
pub fn cutoff_depth(cc77: u8, cc1: u8) -> f32 {
    (cc77 as u16 + cc1 as u16).min(255) as f32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_to_hz_bounds() {
        assert!((rate_to_hz(0) - 0.01).abs() < 1e-6, "{}", rate_to_hz(0));
        assert!((rate_to_hz(255) - 20.0).abs() < 1e-4, "{}", rate_to_hz(255));
        assert!(rate_to_hz(255) > rate_to_hz(0));
    }

    #[test]
    fn delay_to_seconds_bounds() {
        assert_eq!(delay_to_seconds(0), 0.0);
        assert!((delay_to_seconds(255) - 10.0).abs() < 1e-6);
    }

    #[derive(Default)]
    struct MockTarget {
        pitch_cents: f32,
        volume_delta: f32,
    }

    impl PerformanceLfoTarget for MockTarget {
        fn apply_pitch_modulation(&mut self, cents: f32) {
            self.pitch_cents = cents;
        }

        fn apply_volume_modulation(&mut self, delta: f32) {
            self.volume_delta = delta;
        }
    }

    #[test]
    fn apply_lfo_modulation_dispatches_to_pitch() {
        let mut target = MockTarget::default();
        apply_lfo_modulation(0.5, LfoDestination::Pitch, 100.0, &mut target);
        assert_eq!(target.pitch_cents, 50.0);
        assert_eq!(target.volume_delta, 0.0);
    }

    #[test]
    fn apply_lfo_modulation_dispatches_to_volume() {
        let mut target = MockTarget::default();
        apply_lfo_modulation(-0.5, LfoDestination::Volume, 0.4, &mut target);
        assert_eq!(target.volume_delta, -0.2);
        assert_eq!(target.pitch_cents, 0.0);
    }

    #[test]
    fn pitch_depth_cents_formula() {
        // CC1=0なら、CC77値（ベース値）のみがそのままセント値になる
        assert_eq!(pitch_depth_cents(100, 0, 64), 100.0);
        // CC77=0、RPN0,5=127（最大）なら、CC1値がそのままセント値になる
        assert!((pitch_depth_cents(0, 127, 127) - 127.0).abs() < 1e-5);
        // RPN0,5=64（デフォルト、約50セント相当）はCC1の影響を約半分に縮小する
        assert!((pitch_depth_cents(0, 127, 64) - 64.0).abs() < 1e-2);
        // 両方最大なら加算される
        assert!((pitch_depth_cents(255, 255, 127) - 510.0).abs() < 1e-2);
    }

    #[test]
    fn lfo_waveform_from_index_round_trips() {
        let waveforms = [
            LfoWaveform::Triangle,
            LfoWaveform::Sine,
            LfoWaveform::Square,
            LfoWaveform::SampleHold,
            LfoWaveform::Saw,
            LfoWaveform::Trapezoid,
            LfoWaveform::Random,
            LfoWaveform::Chaos,
        ];
        for (i, &w) in waveforms.iter().enumerate() {
            assert_eq!(lfo_waveform_from_index(i as u8), w);
        }
        // 範囲外はTriangleにフォールバック
        assert_eq!(lfo_waveform_from_index(255), LfoWaveform::Triangle);
    }

    #[test]
    fn lfo_fade_mode_from_index_round_trips() {
        let modes = [LfoFadeMode::OnIn, LfoFadeMode::OnOut, LfoFadeMode::OffIn, LfoFadeMode::OffOut];
        for (i, &m) in modes.iter().enumerate() {
            assert_eq!(lfo_fade_mode_from_index(i as u8), m);
        }
        assert_eq!(lfo_fade_mode_from_index(255), LfoFadeMode::OnIn);
    }

    #[test]
    fn lfo_offset_param_round_trip_near_center() {
        assert_eq!(lfo_offset_to_param(0), 128);
        assert_eq!(lfo_offset_from_param(128), 0);
        // DT1と同じ中心128・除数128の慣習（CLAUDE.md「DT1」参照）。負側は理論値ぴったり(-100)に
        // 達するが、正側は128分の127までしか伸びないため99止まり（このプロジェクト共通の性質）。
        assert_eq!(lfo_offset_from_param(255), 99);
        assert_eq!(lfo_offset_from_param(0), -100);
    }

    #[test]
    fn volume_depth_clamps_and_normalizes() {
        assert_eq!(volume_depth(0, 0), 0.0);
        assert!((volume_depth(100, 50) - 150.0 / 255.0).abs() < 1e-6);
        // 合計が255を超える場合は255でclampされ、1.0に正規化される
        assert_eq!(volume_depth(255, 255), 1.0);
        assert_eq!(volume_depth(200, 200), 1.0);
    }

    #[test]
    fn cutoff_depth_clamps_without_normalizing() {
        assert_eq!(cutoff_depth(0, 0), 0.0);
        assert_eq!(cutoff_depth(100, 50), 150.0);
        // 合計が255を超える場合は255でclampされる（Volumeと異なり0〜1.0への正規化はしない）
        assert_eq!(cutoff_depth(255, 255), 255.0);
        assert_eq!(cutoff_depth(200, 200), 255.0);
    }
}
