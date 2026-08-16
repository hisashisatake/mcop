// ---------------------------------------------------------------------------
// オペレーター（オシレーター + TimeEgエンベロープ）
//
// ym38x6-core/src/operator.rsの複製・改変版。EG状態機械のみ`sound_core::Eg`から
// `sound_core::TimeEg`（N点Time/Level方式）へ置き換える。EG非依存のロジック
// （周波数計算・ノイズ生成・KSR/TLキャッシュ）は元実装をそのまま踏襲する。
// ---------------------------------------------------------------------------

use sound_fm::mapping::*;
use sound_fm::waveform::{is_noise_waveform, noise_clock_rate, noise_color};
use serde::{Deserialize, Serialize};
use sound_core::{TimeEg, TimeEgParams, WaveTable};

/// オペレーター単位パラメーター一式。ym38x6の`ar/d1r/d1l/d2r/rr/floor/loop_enabled/curve`
/// （8フィールド）を`eg: TimeEgParams`（N点折れ線＋ループ範囲＋多段リリース）1つに統合する。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op505OperatorParams {
    pub tl: u8,
    pub eg: TimeEgParams,
    /// MUL（周波数比、0〜15）。OPM/OPN/OPQ/OPZ共通のMultiple(4bit)に準拠。
    pub mul: u8,
    /// DT1（微細デチューン、0〜255、中心128＝±0、両端±50セント）。
    pub dt1: u8,
    /// キースケーリングレート。`ksr_rate_multiplier(ksr, note)`を`TimeEg::tick`の
    /// `speed_scale`へそのまま渡す（`Eg::tick`の`rate_scale`と意味論一致：大きいほど速い）。
    pub ksr: u8,
    pub am_enable: bool,
    pub velocity_sensitivity: u8,
    /// 0〜255（0〜7=ビルトイン波形、8〜255=ユーザー波形スロット）
    pub waveform: u8,
    /// OP単位の追加チューニング（0〜255、中心128＝±0、両端±1200セント）。
    pub op_fine_tune: u8,
    /// EGSFT（TX81Z EG Shift）。EGの減衰レンジ(dB)を圧縮する（0〜255、既定0＝圧縮なし）。
    pub eg_shift: u8,
    /// Level Scaling（ノート依存の出力レベル減衰、OPL系KSL相当）。0〜255、既定0＝スケーリングなし。
    pub level_scale: u8,
    /// キャリア出力へのベロシティ音量ゲイン深さ（0〜255、既定255）。モジュレーターでは無視される。
    pub velocity_gain: u8,
}

fn default_op_fine_tune() -> u8 {
    128
}

fn default_velocity_gain() -> u8 {
    255
}

impl Default for Op505OperatorParams {
    fn default() -> Self {
        Self {
            tl: 0,
            eg: TimeEgParams::default(),
            mul: 0,
            dt1: 0,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: default_op_fine_tune(),
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: default_velocity_gain(),
        }
    }
}

pub struct Operator {
    pub params: Op505OperatorParams,
    frequency: f32,
    phase: f32,
    eg: TimeEg,
    velocity: u8,
    chip_lfo_pitch_mod_cents: f32,
    chip_lfo_amp_mod: f32,
    perf_lfo_pitch_mod_cents: f32,
    f_number_ratio: f32,
    is_carrier: bool,
    noise_lfsr: u32,
    noise_accum: f32,
    noise_hold: f32,
    cached_rate_scale: f32,
    cached_rate_scale_key: Option<(u8, u8)>,
    cached_pitch_ratio: f32,
    cached_pitch_ratio_key: Option<f32>,
    /// `env_amp`の等比数列キャッシュ。TimeEgは段ごとにcurveを持つため（ym38x6版のような
    /// グローバル`params.curve`フラグでの早期バイパスは使えない）、`delta`（前サンプルからの
    /// env_level変化量）が前回と一致するかだけで有効性を判定する。線形区間ではdeltaが
    /// 毎サンプル一定になり自然にキャッシュが効き、curve!=0の区間はdeltaが揺れて
    /// 自然にフォールスルー（直接powf()）する。
    cached_env_amp: f32,
    cached_env_level: f32,
    cached_env_delta: f32,
    cached_env_ratio: f32,
    env_amp_cache_valid: bool,
    env_amp_resync_counter: u32,
    cached_tl_gain: f32,
    cached_tl_gain_key: Option<(u8, u8, u8, u8, u8)>,
}

const ENV_AMP_RESYNC_INTERVAL: u32 = 4096;

impl Operator {
    pub fn new(params: Op505OperatorParams) -> Self {
        Self {
            params,
            frequency: 440.0,
            phase: 0.0,
            eg: TimeEg::new(),
            velocity: 127,
            chip_lfo_pitch_mod_cents: 0.0,
            chip_lfo_amp_mod: 0.0,
            perf_lfo_pitch_mod_cents: 0.0,
            f_number_ratio: 1.0,
            is_carrier: false,
            noise_lfsr: 1,
            noise_accum: 0.0,
            noise_hold: 0.0,
            cached_rate_scale: 1.0,
            cached_rate_scale_key: None,
            cached_pitch_ratio: 1.0,
            cached_pitch_ratio_key: None,
            cached_env_amp: 0.0,
            cached_env_level: 0.0,
            cached_env_delta: 0.0,
            cached_env_ratio: 1.0,
            env_amp_cache_valid: false,
            env_amp_resync_counter: 0,
            cached_tl_gain: 0.0,
            cached_tl_gain_key: None,
        }
    }

    pub fn set_carrier(&mut self, is_carrier: bool) {
        self.is_carrier = is_carrier;
    }

    pub fn note_on(&mut self, base_frequency: f32, velocity: u8) {
        self.frequency = base_frequency;
        self.velocity = velocity;
        self.phase = 0.0;
        self.eg.note_on();
        self.f_number_ratio = 1.0;
        self.noise_lfsr = 1;
        self.noise_accum = 0.0;
        self.noise_hold = 0.0;
        self.env_amp_cache_valid = false;
    }

    pub fn retrigger(&mut self, base_frequency: f32, velocity: u8) {
        self.frequency = base_frequency;
        self.velocity = velocity;
        self.phase = 0.0;
        self.eg.retrigger();
        self.f_number_ratio = 1.0;
        self.noise_lfsr = 1;
        self.noise_accum = 0.0;
        self.noise_hold = 0.0;
        self.env_amp_cache_valid = false;
    }

    pub fn note_off(&mut self) {
        self.eg.note_off();
    }

    pub fn is_idle(&self) -> bool {
        self.eg.is_idle()
    }

    /// 現在のEGレベル(0.0〜1.0、curve整形前の生値)。ボイススチールの判定に使う。
    pub fn env_level(&self) -> f32 {
        self.eg.level()
    }

    pub fn set_chip_lfo_modulation(&mut self, pitch_cents: f32, amp_mod: f32) {
        self.chip_lfo_pitch_mod_cents = pitch_cents;
        self.chip_lfo_amp_mod = amp_mod;
    }

    pub fn set_pitch_modulation(&mut self, cents: f32) {
        self.perf_lfo_pitch_mod_cents = cents;
    }

    pub fn set_f_number_override(&mut self, f_number: u16) {
        self.f_number_ratio = f_number_to_ratio(f_number);
    }

    fn effective_frequency(&mut self) -> f32 {
        let cents = dt1_to_cents(self.params.dt1)
            + op_fine_tune_to_cents(self.params.op_fine_tune)
            + self.chip_lfo_pitch_mod_cents
            + self.perf_lfo_pitch_mod_cents;
        let pitch_ratio = if self.cached_pitch_ratio_key == Some(cents) {
            self.cached_pitch_ratio
        } else {
            let v = 2f32.powf(cents / 1200.0);
            self.cached_pitch_ratio = v;
            self.cached_pitch_ratio_key = Some(cents);
            v
        };
        self.frequency * self.f_number_ratio * mul_to_ratio(self.params.mul) * pitch_ratio
    }

    fn compute_env_amp(&mut self, env_level: f32, db_range: f32) -> f32 {
        let k = db_range / 20.0;
        let delta = env_level - self.cached_env_level;
        self.env_amp_resync_counter += 1;
        let use_cache = self.env_amp_cache_valid
            && delta == self.cached_env_delta
            && self.env_amp_resync_counter < ENV_AMP_RESYNC_INTERVAL;
        let env_amp = if use_cache {
            self.cached_env_amp * self.cached_env_ratio
        } else {
            let amp = 10f32.powf(-(1.0 - env_level) * k);
            self.cached_env_ratio = 10f32.powf(delta * k);
            self.cached_env_delta = delta;
            self.env_amp_cache_valid = true;
            self.env_amp_resync_counter = 0;
            amp
        };
        self.cached_env_amp = env_amp;
        self.cached_env_level = env_level;
        env_amp
    }

    fn next_noise_sample(&mut self, wf: u8, sample_rate: f32) -> f32 {
        let rate = noise_clock_rate(noise_color(wf));
        self.noise_accum += rate / sample_rate;
        while self.noise_accum >= 1.0 {
            self.noise_accum -= 1.0;
            let bit = (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1;
            self.noise_lfsr = (self.noise_lfsr >> 1) | (bit << 16);
            self.noise_hold = if bit != 0 { 1.0 } else { -1.0 };
        }
        self.noise_hold
    }

    /// `modulation`: FM変調入力（位相オフセット、0.0〜1.0スケール）
    pub fn tick(&mut self, sample_rate: f32, wave: &WaveTable, modulation: f32, note: u8) -> f32 {
        if self.eg.is_idle() {
            return 0.0;
        }
        let rate_scale_key = (self.params.ksr, note);
        let ksr_mul = if self.cached_rate_scale_key == Some(rate_scale_key) {
            self.cached_rate_scale
        } else {
            let v = ksr_rate_multiplier(self.params.ksr, note);
            self.cached_rate_scale = v;
            self.cached_rate_scale_key = Some(rate_scale_key);
            v
        };
        let env_level = self.eg.tick(sample_rate, self.params.eg, ksr_mul);

        let freq = self.effective_frequency();
        self.phase = (self.phase + freq / sample_rate).fract();

        let sample = if is_noise_waveform(self.params.waveform) {
            self.next_noise_sample(self.params.waveform, sample_rate)
        } else {
            let modulated_phase = (self.phase + modulation).rem_euclid(1.0);
            let idx = (modulated_phase * wave.len() as f32) as usize;
            wave.sample_at(idx)
        };

        let vel_sens = if self.is_carrier { 0 } else { self.params.velocity_sensitivity };
        let tl_gain_key = (self.params.tl, self.velocity, vel_sens, self.params.level_scale, note);
        let tl_gain = if self.cached_tl_gain_key == Some(tl_gain_key) {
            self.cached_tl_gain
        } else {
            let eff_tl = effective_tl(self.params.tl, self.velocity, vel_sens);
            let eff_tl = eff_tl.saturating_sub(level_scale_atten(self.params.level_scale, note));
            let v = tl_to_gain(eff_tl);
            self.cached_tl_gain = v;
            self.cached_tl_gain_key = Some(tl_gain_key);
            v
        };
        let amp_factor = (1.0 - self.chip_lfo_amp_mod).clamp(0.0, 1.0);
        let db_range = eg_shift_to_db_range(self.params.eg_shift);
        let env_amp = self.compute_env_amp(env_level, db_range);
        sample * env_amp * tl_gain * amp_factor
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sound_core::{TimeStage, MAX_STAGES};
    use sound_fm::waveform::gen_op_sine;

    fn stages_with(entries: &[(u8, u8, u8)]) -> [TimeStage; MAX_STAGES] {
        let mut stages = [TimeStage::default(); MAX_STAGES];
        for (i, &(time, level, curve)) in entries.iter().enumerate() {
            stages[i] = TimeStage { time, level, curve };
        }
        stages
    }

    /// Attack→Decay1(d1l相当)→Decay2(0へ張り付き、Idleにはならない)＋リリース段の4段。
    /// 保持区間`0..=release_point(2)`で「Decay2の0張り付き」を表現し、
    /// リリース区間は段3の1本（0→0の即時遷移で次段が無いのでIdleへ）。
    fn fast_params() -> Op505OperatorParams {
        Op505OperatorParams {
            tl: 255,
            eg: TimeEgParams {
                stages: stages_with(&[(1, 255, 0), (1, 128, 0), (1, 0, 0), (1, 0, 0)]),
                stage_count: 4,
                loop_enabled: 0,
                loop_start: 2,
                release_point: 2,
            },
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        }
    }

    /// サスティンを満レベルで即座に保持するノイズ用パラメーター（time=0＝瞬時ジャンプ）。
    fn noise_params(wf: u8) -> Op505OperatorParams {
        Op505OperatorParams {
            eg: TimeEgParams {
                stages: stages_with(&[(0, 255, 0), (0, 0, 0)]),
                stage_count: 2,
                loop_enabled: 0,
                loop_start: 0,
                release_point: 0,
            },
            waveform: wf,
            ..fast_params()
        }
    }

    fn collect_noise(wf: u8, n: usize) -> Vec<f32> {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let mut op = Operator::new(noise_params(wf));
        op.note_on(440.0, 127);
        for _ in 0..16 {
            op.tick(sr, &wave, 0.0, 69);
        }
        (0..n).map(|_| op.tick(sr, &wave, 0.0, 69)).collect()
    }

    #[test]
    fn white_noise_is_aperiodic_and_bipolar() {
        let out = collect_noise(32, 2000);
        let zc = out.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        assert!(zc > 400, "white zero-crossings too low: {zc}");
        assert!(out.iter().cloned().fold(f32::MAX, f32::min) < 0.0);
        assert!(out.iter().cloned().fold(f32::MIN, f32::max) > 0.0);
    }

    #[test]
    fn darker_color_reduces_high_frequency_content() {
        let white = collect_noise(32, 4000);
        let dark = collect_noise(63, 4000);
        let mean_abs_diff = |v: &[f32]| {
            v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (v.len() - 1) as f32
        };
        let dw = mean_abs_diff(&white);
        let dd = mean_abs_diff(&dark);
        assert!(dw > dd * 2.0, "white diff {dw} should far exceed dark diff {dd}");
    }

    /// EGSFT（eg_shift）はEGの減衰レンジを圧縮し、サステインの床を持ち上げる。
    /// stage1(level=128)をリリース点にして張り付かせ、
    /// eg_shift=0と255でサステインのピーク振幅を比較する（段2はリリース用）。
    #[test]
    fn eg_shift_raises_sustained_amplitude() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let base = Op505OperatorParams {
            eg: TimeEgParams {
                stages: stages_with(&[(1, 255, 0), (1, 128, 0), (1, 0, 0)]),
                stage_count: 3,
                loop_enabled: 0,
                loop_start: 1,
                release_point: 1,
            },
            ..fast_params()
        };
        let settle = |eg_shift: u8| -> f32 {
            let mut op = Operator::new(Op505OperatorParams { eg_shift, ..base });
            op.note_on(440.0, 127);
            for _ in 0..2000 {
                op.tick(sr, &wave, 0.0, 69);
            }
            (0..512)
                .map(|_| op.tick(sr, &wave, 0.0, 69).abs())
                .fold(0.0f32, f32::max)
        };
        let off = settle(0);
        let on = settle(255);
        assert!(off < 0.02, "eg_shift=0 sustain should stay near -48dB, got {off}");
        assert!(on > off * 10.0, "eg_shift should raise sustain floor: off={off} on={on}");
    }

    #[test]
    fn envelope_transitions_through_phases() {
        let sr = 44100.0;
        // frequency=0＋矩形波で位相を固定し、出力振幅が包絡線を直接反映するようにする
        // （TimeEgの最小段長は約44サンプル=1ms単位のため、実周波数のサイン波だと
        // ピーク検出ウィンドウ内でサイン波の位相とEGピークが偶然一致しないと失敗しうる）。
        let wave = sound_core::gen_square();
        let mut op = Operator::new(fast_params());
        op.note_on(0.0, 127);
        assert!(!op.is_idle());

        let mut peak = 0.0f32;
        for _ in 0..50 {
            peak = peak.max(op.tick(sr, &wave, 0.0, 69).abs());
        }
        assert!(peak > 0.9, "expected attack to reach near-peak amplitude, got {peak}");

        let mut settled = 0.0f32;
        for _ in 0..2000 {
            settled = op.tick(sr, &wave, 0.0, 69).abs();
        }
        assert!(!op.is_idle(), "should still be sounding (stage2 sticks at 0, not Idle)");
        assert!(settled < 1e-3, "expected amplitude to have decayed near 0, got {settled}");

        op.note_off();
        assert!(!op.is_idle(), "should not be idle immediately after note_off (Release just entered)");
        let mut became_idle = false;
        for _ in 0..200 {
            op.tick(sr, &wave, 0.0, 69);
            if op.is_idle() {
                became_idle = true;
                break;
            }
        }
        assert!(became_idle, "expected to reach Idle after Release completes");
    }

    #[test]
    fn idle_operator_is_silent() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let mut op = Operator::new(fast_params());
        assert!(op.is_idle());
        assert_eq!(op.tick(sr, &wave, 0.0, 69), 0.0);
    }

    #[test]
    fn mul_and_dt1_change_effective_frequency() {
        let mut params = fast_params();
        params.mul = 1;
        params.dt1 = 128;
        let mut op = Operator::new(params);
        op.note_on(440.0, 127);
        let base = op.effective_frequency();
        assert!((base - 440.0).abs() < 1e-3);

        op.params.mul = 2;
        let doubled = op.effective_frequency();
        assert!((doubled - 880.0).abs() < 1e-3);

        op.params.mul = 1;
        op.params.dt1 = 0;
        let detuned = op.effective_frequency();
        assert!(detuned < base, "detune downward should lower frequency: {detuned} vs {base}");
    }

    #[test]
    fn op_fine_tune_shifts_effective_frequency_about_one_octave() {
        let mut params = fast_params();
        params.mul = 1;
        params.dt1 = 128;
        params.op_fine_tune = 255;
        let mut op = Operator::new(params);
        op.note_on(440.0, 127);
        let expected = 440.0 * 2f32.powf(1190.625 / 1200.0);
        assert!(
            (op.effective_frequency() - expected).abs() < 0.5,
            "op_fine_tune up should raise ~1 octave: {} vs {}",
            op.effective_frequency(),
            expected
        );

        op.params.op_fine_tune = 128;
        assert!((op.effective_frequency() - 440.0).abs() < 1e-3);
    }

    #[test]
    fn f_number_override_changes_effective_frequency_and_resets_on_note_on() {
        let mut op = Operator::new(fast_params());
        op.note_on(440.0, 127);
        let base = op.effective_frequency();
        assert!((base - 440.0).abs() < 1e-3);

        op.set_f_number_override(F_NUMBER_CENTER / 2);
        let halved = op.effective_frequency();
        assert!((halved - 220.0).abs() < 1e-3);

        op.note_on(440.0, 127);
        let reset = op.effective_frequency();
        assert!((reset - 440.0).abs() < 1e-3);
    }

    #[test]
    fn chip_lfo_modulation_affects_frequency_and_amplitude() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let mut op = Operator::new(fast_params());
        op.note_on(440.0, 127);

        let base = op.effective_frequency();
        op.set_chip_lfo_modulation(100.0, 0.0);
        let pitched = op.effective_frequency();
        assert!(pitched > base, "positive pitch mod should raise frequency");

        op.set_chip_lfo_modulation(0.0, 1.0);
        for _ in 0..10 {
            assert_eq!(op.tick(sr, &wave, 0.0, 69), 0.0);
        }
    }

    /// compute_env_ampの等比数列キャッシュが、毎回powf()で直接計算した値からどれだけ
    /// 乖離するかを実際のnote_on〜note_offのライフサイクルで検証する回帰テスト。
    #[test]
    fn env_amp_cache_stays_close_to_direct_computation() {
        let sr = 44100.0;
        let wave = sound_core::gen_square();
        let params = Op505OperatorParams {
            tl: 255,
            eg: TimeEgParams {
                stages: stages_with(&[(60, 255, 0), (90, 180, 0), (70, 40, 0), (80, 0, 0)]),
                stage_count: 4,
                loop_enabled: 0,
                loop_start: 2,
                release_point: 2,
            },
            velocity_sensitivity: 0,
            eg_shift: 0,
            ..fast_params()
        };
        let mut op = Operator::new(params);
        op.note_on(0.0, 127); // frequency=0 → phaseが進まずsample_at(0)固定
        let db_range = sound_fm::mapping::eg_shift_to_db_range(0);
        let k = db_range / 20.0;
        let mut max_rel_diff = 0.0f32;
        for i in 0..20000 {
            if i == 8000 {
                op.note_off();
            }
            let out = op.tick(sr, &wave, 0.0, 69);
            if op.is_idle() {
                break;
            }
            let level = op.env_level();
            let reference = 10f32.powf(-(1.0 - level) * k);
            let diff = (out - reference).abs();
            let rel = diff / reference.max(1e-9);
            max_rel_diff = max_rel_diff.max(rel);
        }
        eprintln!("max_rel_diff={max_rel_diff}");
        assert!(max_rel_diff < 0.01, "cached env_amp diverges too much from direct computation: rel={max_rel_diff}");
    }

    /// OP単位ループEG: loop=1で、そのOPのEG出力（＝モジュレーターなら変調指数）が
    /// floor(0)とpeak(1.0)の間を周期的に往復することを確認する。
    #[test]
    fn loop_enabled_operator_oscillates_between_floor_and_peak() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let params = Op505OperatorParams {
            eg: TimeEgParams {
                // 段3はリリース用（ループ区間は1..=2のまま）。
                stages: stages_with(&[(100, 255, 0), (90, 0, 0), (90, 255, 0), (90, 0, 0)]),
                stage_count: 4,
                loop_enabled: 1,
                loop_start: 1,
                release_point: 2,
            },
            ..fast_params()
        };
        let mut op = Operator::new(params);
        op.note_on(440.0, 127);

        for _ in 0..40000 {
            if op.tick(sr, &wave, 0.0, 69).abs() >= 0.99 {
                break;
            }
        }

        const WINDOW: usize = 256;
        let mut chunk_max = Vec::new();
        let mut cur_max = 0.0f32;
        for i in 0..40000 {
            let sample = op.tick(sr, &wave, 0.0, 69).abs();
            cur_max = cur_max.max(sample);
            if (i + 1) % WINDOW == 0 {
                chunk_max.push(cur_max);
                cur_max = 0.0;
            }
        }

        let overall_max = chunk_max.iter().cloned().fold(0.0f32, f32::max);
        let overall_min = chunk_max.iter().cloned().fold(1.0f32, f32::min);
        assert!(!op.is_idle(), "loop mode should never become idle on its own");
        assert!(overall_max > 0.9, "expected windowed peak near 1.0, got {overall_max}");
        assert!(overall_min < 0.05, "expected windowed trough near silence(floor=0), got {overall_min}");
    }
}
