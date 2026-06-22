// ---------------------------------------------------------------------------
// オペレーター（オシレーター + EGエンベロープ）
// ---------------------------------------------------------------------------

use crate::mapping::*;
use crate::waveform::{is_noise_waveform, noise_clock_rate, noise_color};
use serde::{Deserialize, Serialize};
use sound_core::WaveTable;

/// OPN系5段階エンベロープ（Attack→Decay1→Decay2→Release、+Idle）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EnvPhase {
    Attack,
    Decay1,
    Decay2,
    Release,
    Idle,
}

/// オペレーター単位パラメーター一式（12個）。NRPN/DAWパラメーターから直接コピー可能。
/// 基本は全8bit(0〜255)だが、`mul`は0〜255統一の例外（[mapping::mul_to_ratio](crate::mapping::mul_to_ratio)参照）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperatorParams {
    pub tl: u8,
    pub ar: u8,
    pub d1r: u8,
    pub d2r: u8,
    pub d1l: u8,
    pub rr: u8,
    /// MUL（周波数比、0〜15）。OPM/OPN/OPQ/OPZ共通のMultiple(4bit)に準拠。
    pub mul: u8,
    /// DT1（微細デチューン、0〜255、中心128＝±0、両端±50セント）。OPM/OPN/OPQ系の慣習に合わせた微調整用。
    pub dt1: u8,
    pub ksr: u8,
    pub am_enable: bool,
    pub velocity_sensitivity: u8,
    /// 0〜255（0〜7=ビルトイン波形、8〜255=ユーザー波形スロット）
    pub waveform: u8,
    /// OP単位の追加チューニング（0〜255、中心128＝±0、両端±1200セント＝±1オクターブ）。
    /// DT1(±50セント)で足りない広いデチューンや、インハーモニックなOP周波数比を音色として静的に持たせる拡張。
    /// DT1とはセントで加算される。既存`.38x6`に無い場合は中心128（オフセットなし）。
    #[serde(default = "default_op_fine_tune")]
    pub op_fine_tune: u8,
}

/// `op_fine_tune`の中心値（オフセットなし）。serde欠落時およびDefaultで使う。
pub(crate) fn default_op_fine_tune() -> u8 {
    128
}

impl Default for OperatorParams {
    /// 既存挙動を保つため数値0・bool false（TL=0で無音）。ただし双極性の`op_fine_tune`のみ
    /// 中心128（オフセットなし）を既定とする。
    fn default() -> Self {
        Self {
            tl: 0,
            ar: 0,
            d1r: 0,
            d2r: 0,
            d1l: 0,
            rr: 0,
            mul: 0,
            dt1: 0,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: default_op_fine_tune(),
        }
    }
}

pub struct Operator {
    pub params: OperatorParams,
    /// フェーズ3は全Op同一値（Note-On時に一括設定）。フェーズ4のOP単位上書きの土台。
    frequency: f32,
    phase: f32,
    env_phase: EnvPhase,
    env_level: f32,
    velocity: u8,
    /// 音色LFOによるピッチ変調（セント、全Op共通、3.1.5でChannelが設定）
    tone_lfo_pitch_mod_cents: f32,
    /// 音色LFOによる振幅変調（0.0〜1.0、am_enable時のみ非ゼロ、3.1.5でChannelが設定）
    tone_lfo_amp_mod: f32,
    /// パフォーマンスLFO（ビブラート）によるピッチ変調（セント、全Op共通、Channelが毎サンプル設定）
    perf_lfo_pitch_mod_cents: f32,
    /// OP単位F-Number上書き(NRPN Operator F-Number)による周波数比。1.0=上書きなし（Note-Onでリセット）。
    f_number_ratio: f32,
    /// 現在のアルゴリズムでこのOPがキャリア（出力に直接合算される）かどうか。
    /// Velocity Sensitivity（明るさ専用）はモジュレーターにのみ効かせるため、キャリアでは無視する。
    is_carrier: bool,
    /// ノイズ波形（32〜63）用の17bit LFSR状態（AY-3-8910互換）。初期1。
    noise_lfsr: u32,
    /// サンプル&ホールドの更新蓄積（クロックレート / sample_rate を毎サンプル加算）。
    noise_accum: f32,
    /// サンプル&ホールドの現在保持値（±1）。
    noise_hold: f32,
}

impl Operator {
    pub fn new(params: OperatorParams) -> Self {
        Self {
            params,
            frequency: 440.0,
            phase: 0.0,
            env_phase: EnvPhase::Idle,
            env_level: 0.0,
            velocity: 127,
            tone_lfo_pitch_mod_cents: 0.0,
            tone_lfo_amp_mod: 0.0,
            perf_lfo_pitch_mod_cents: 0.0,
            f_number_ratio: 1.0,
            is_carrier: false,
            noise_lfsr: 1,
            noise_accum: 0.0,
            noise_hold: 0.0,
        }
    }

    /// このOPがキャリアかどうかを設定する（Channelがアルゴリズムから決めて呼ぶ）。
    pub fn set_carrier(&mut self, is_carrier: bool) {
        self.is_carrier = is_carrier;
    }

    pub fn note_on(&mut self, base_frequency: f32, velocity: u8) {
        self.frequency = base_frequency;
        self.velocity = velocity;
        self.phase = 0.0;
        self.env_phase = EnvPhase::Attack;
        self.env_level = 0.0;
        self.f_number_ratio = 1.0;
        self.noise_lfsr = 1;
        self.noise_accum = 0.0;
        self.noise_hold = 0.0;
    }

    /// 残響レベルを保持したまま Attack 相位へ再突入する（実機 OPM の Key-On 挙動の再現）。
    /// 前の音が消えきる前のキーオンで `env_level` を 0（無音）に落とさず、現在のレベルから
    /// アタックを再開する。これにより同音連打でのプチノイズが消え、モジュレーター側の
    /// エンベロープが残響に引きずられることで生じる「FMらしい再アタックの明るさ」も再現される。
    /// 位相は実機同様リセットする。
    pub fn retrigger(&mut self, base_frequency: f32, velocity: u8) {
        self.frequency = base_frequency;
        self.velocity = velocity;
        self.phase = 0.0;
        self.env_phase = EnvPhase::Attack;
        // env_level は保持（残響から再アタック）
        self.f_number_ratio = 1.0;
        self.noise_lfsr = 1;
        self.noise_accum = 0.0;
        self.noise_hold = 0.0;
    }

    pub fn note_off(&mut self) {
        if self.env_phase != EnvPhase::Idle {
            self.env_phase = EnvPhase::Release;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.env_phase == EnvPhase::Idle
    }

    /// 音色LFOによる変調値を設定する（毎サンプル、Channelから呼ばれる）。
    pub fn set_tone_lfo_modulation(&mut self, pitch_cents: f32, amp_mod: f32) {
        self.tone_lfo_pitch_mod_cents = pitch_cents;
        self.tone_lfo_amp_mod = amp_mod;
    }

    /// パフォーマンスLFO（ビブラート）によるピッチ変調を設定する（毎サンプル、Channelから呼ばれる）。
    pub fn set_pitch_modulation(&mut self, cents: f32) {
        self.perf_lfo_pitch_mod_cents = cents;
    }

    /// OP単位F-Number上書き（NRPN Operator F-Number、0〜8191）を設定する。
    pub fn set_f_number_override(&mut self, f_number: u16) {
        self.f_number_ratio = f_number_to_ratio(f_number);
    }

    fn effective_frequency(&self) -> f32 {
        let cents = dt1_to_cents(self.params.dt1)
            + op_fine_tune_to_cents(self.params.op_fine_tune)
            + self.tone_lfo_pitch_mod_cents
            + self.perf_lfo_pitch_mod_cents;
        self.frequency * self.f_number_ratio * mul_to_ratio(self.params.mul) * 2f32.powf(cents / 1200.0)
    }

    fn tick_envelope(&mut self, sample_rate: f32, note: u8) {
        let ksr_mul = ksr_rate_multiplier(self.params.ksr, note);
        // env_level 空間での閾値: d1l=255→1.0（即D2R移行）、d1l=0→0.0（D1Rが全域減衰）
        let sustain_level = self.params.d1l as f32 / 255.0;
        match self.env_phase {
            EnvPhase::Attack => {
                self.env_level += ar_to_delta(self.params.ar, sample_rate) * ksr_mul;
                if self.env_level >= 1.0 {
                    self.env_level = 1.0;
                    self.env_phase = EnvPhase::Decay1;
                }
            }
            EnvPhase::Decay1 => {
                self.env_level -= decay_to_delta(self.params.d1r, sample_rate) * ksr_mul;
                if self.env_level <= sustain_level {
                    self.env_level = sustain_level;
                    self.env_phase = EnvPhase::Decay2;
                }
            }
            EnvPhase::Decay2 => {
                self.env_level -= decay_to_delta(self.params.d2r, sample_rate) * ksr_mul;
                if self.env_level <= 0.0 {
                    self.env_level = 0.0; // Idleにはせずキーオン継続中は0に張り付く
                }
            }
            EnvPhase::Release => {
                self.env_level -= rr_to_delta(self.params.rr, sample_rate) * ksr_mul;
                if self.env_level <= 0.0 {
                    self.env_level = 0.0;
                    self.env_phase = EnvPhase::Idle;
                }
            }
            EnvPhase::Idle => {}
        }
    }

    /// ノイズ波形（32〜63）の1サンプルを生成する。17bit LFSR（AY-3-8910互換）を
    /// 色(=NP)に応じたクロックレートでサンプル&ホールド更新する。NP小=高レート（広帯域/白）、
    /// NP大=低レート（同値が連続=低域寄り）。ピッチレスのため note 周波数には依存しない。
    fn next_noise_sample(&mut self, wf: u8, sample_rate: f32) -> f32 {
        let rate = noise_clock_rate(noise_color(wf));
        self.noise_accum += rate / sample_rate;
        // 1サンプルでrate>sample_rate（NP小）の場合は複数回更新＝広帯域ノイズ
        while self.noise_accum >= 1.0 {
            self.noise_accum -= 1.0;
            // 17bit LFSR: bit = lfsr[0] XOR lfsr[3]、17段右シフト
            let bit = (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1;
            self.noise_lfsr = (self.noise_lfsr >> 1) | (bit << 16);
            self.noise_hold = if bit != 0 { 1.0 } else { -1.0 };
        }
        self.noise_hold
    }

    /// `modulation`: FM変調入力（位相オフセット、0.0〜1.0スケール）
    pub fn tick(&mut self, sample_rate: f32, wave: &WaveTable, modulation: f32, note: u8) -> f32 {
        if self.env_phase == EnvPhase::Idle {
            return 0.0;
        }
        self.tick_envelope(sample_rate, note);

        let freq = self.effective_frequency();
        self.phase = (self.phase + freq / sample_rate).fract();

        // ノイズ波形はテーブル参照せずLFSRでランタイム生成（ピッチレス、modulation無視）。
        let sample = if is_noise_waveform(self.params.waveform) {
            self.next_noise_sample(self.params.waveform, sample_rate)
        } else {
            let modulated_phase = (self.phase + modulation).rem_euclid(1.0);
            let idx = (modulated_phase * wave.len() as f32) as usize;
            wave.sample_at(idx)
        };

        // Velocity Sensitivityは明るさ専用：モジュレーターのTL（=変調量）にのみ効かせる。
        // キャリアでは無視し、音量はチャンネル側のvelocity_to_volume_gainに一本化する。
        let vel_sens = if self.is_carrier { 0 } else { self.params.velocity_sensitivity };
        let eff_tl = effective_tl(self.params.tl, self.velocity, vel_sens);
        let amp_factor = (1.0 - self.tone_lfo_amp_mod).clamp(0.0, 1.0);
        // dBリニアエンベロープ（OPN/OPM互換）: env_level=1.0→0dB, 0.0→-96dB
        // 4.8 = 96.0 / 20.0
        let env_amp = 10f32.powf(-(1.0 - self.env_level) * 4.8);
        sample * env_amp * tl_to_gain(eff_tl) * amp_factor
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::gen_op_sine;

    fn fast_params() -> OperatorParams {
        OperatorParams {
            tl: 255,
            ar: 255,
            d1r: 255,
            d2r: 255,
            d1l: 128,
            rr: 255,
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
        }
    }

    /// サスティンを満レベルで保持するノイズ用パラメーター（音量を一定にして解析する）。
    fn noise_params(wf: u8) -> OperatorParams {
        OperatorParams { d1l: 255, d2r: 0, rr: 0, waveform: wf, ..fast_params() }
    }

    /// アタック整定後の `n` サンプルを収集する（ノイズはテーブル参照しないので wave はダミー）。
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
        let out = collect_noise(32, 2000); // color=NP=0 ≒ 最広帯域（白）
        let zc = out.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        // 白は高レート更新で頻繁に符号反転（ピッチレス）。32サンプルループのような周期性は出ない。
        assert!(zc > 400, "white zero-crossings too low: {zc}");
        // 正負両方に振れる
        assert!(out.iter().cloned().fold(f32::MAX, f32::min) < 0.0);
        assert!(out.iter().cloned().fold(f32::MIN, f32::max) > 0.0);
    }

    #[test]
    fn darker_color_reduces_high_frequency_content() {
        // 隣接サンプル差の平均: 白(NP=0) > 低域(NP=31)。NP大は低レート更新で同値が連続し滑らか。
        let white = collect_noise(32, 4000);
        let dark = collect_noise(63, 4000);
        let mean_abs_diff = |v: &[f32]| {
            v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (v.len() - 1) as f32
        };
        let dw = mean_abs_diff(&white);
        let dd = mean_abs_diff(&dark);
        assert!(dw > dd * 2.0, "white diff {dw} should far exceed dark diff {dd}");
    }

    #[test]
    fn envelope_transitions_through_phases() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let mut op = Operator::new(fast_params());
        op.note_on(440.0, 127);
        assert_eq!(op.env_phase, EnvPhase::Attack);

        // Attack → Decay1
        for _ in 0..200 {
            if op.env_phase != EnvPhase::Attack {
                break;
            }
            op.tick(sr, &wave, 0.0, 69);
        }
        assert_eq!(op.env_phase, EnvPhase::Decay1);

        // Decay1 → Decay2
        for _ in 0..400 {
            if op.env_phase != EnvPhase::Decay1 {
                break;
            }
            op.tick(sr, &wave, 0.0, 69);
        }
        assert_eq!(op.env_phase, EnvPhase::Decay2);

        // Decay2: env_levelが0に張り付き、Idleにはならない
        for _ in 0..200 {
            op.tick(sr, &wave, 0.0, 69);
        }
        assert_eq!(op.env_phase, EnvPhase::Decay2);
        assert_eq!(op.env_level, 0.0);

        // note_off → Release → Idle
        op.note_off();
        assert_eq!(op.env_phase, EnvPhase::Release);
        for _ in 0..200 {
            if op.is_idle() {
                break;
            }
            op.tick(sr, &wave, 0.0, 69);
        }
        assert!(op.is_idle());
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
        params.mul = 1; // ratio = 1.0
        params.dt1 = 128; // 0 cents
        let mut op = Operator::new(params);
        op.note_on(440.0, 127);
        let base = op.effective_frequency();
        assert!((base - 440.0).abs() < 1e-3);

        // MULを変えると周波数が変わる（mul=2 → ratio=2.0）
        op.params.mul = 2;
        let doubled = op.effective_frequency();
        assert!((doubled - 880.0).abs() < 1e-3);

        // DT1を変えると周波数が変わる（dt1=128に戻し、dt1=0で-50セント）
        op.params.mul = 1;
        op.params.dt1 = 0;
        let detuned = op.effective_frequency();
        assert!(detuned < base, "detune downward should lower frequency: {detuned} vs {base}");
    }

    #[test]
    fn op_fine_tune_shifts_effective_frequency_about_one_octave() {
        let mut params = fast_params();
        params.mul = 1; // ratio = 1.0
        params.dt1 = 128; // DT1は中立
        params.op_fine_tune = 255; // ほぼ+1オクターブ(+1190.625セント)
        let mut op = Operator::new(params);
        op.note_on(440.0, 127);
        let expected = 440.0 * 2f32.powf(1190.625 / 1200.0); // ≒ ×1.988
        assert!(
            (op.effective_frequency() - expected).abs() < 0.5,
            "op_fine_tune up should raise ~1 octave: {} vs {}",
            op.effective_frequency(),
            expected
        );

        // 中心128ではDT1のみ作用し、追加チューニングは0
        op.params.op_fine_tune = 128;
        assert!((op.effective_frequency() - 440.0).abs() < 1e-3);
    }

    #[test]
    fn f_number_override_changes_effective_frequency_and_resets_on_note_on() {
        let mut op = Operator::new(fast_params());
        op.note_on(440.0, 127);
        let base = op.effective_frequency();
        assert!((base - 440.0).abs() < 1e-3);

        // F_NUMBER_CENTERの半分（2048）→比率0.5倍
        op.set_f_number_override(F_NUMBER_CENTER / 2);
        let halved = op.effective_frequency();
        assert!((halved - 220.0).abs() < 1e-3);

        // note_onで比率1.0にリセットされる
        op.note_on(440.0, 127);
        let reset = op.effective_frequency();
        assert!((reset - 440.0).abs() < 1e-3);
    }

    #[test]
    fn tone_lfo_modulation_affects_frequency_and_amplitude() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let mut op = Operator::new(fast_params());
        op.note_on(440.0, 127);

        let base = op.effective_frequency();
        op.set_tone_lfo_modulation(100.0, 0.0);
        let pitched = op.effective_frequency();
        assert!(pitched > base, "positive pitch mod should raise frequency");

        op.set_tone_lfo_modulation(0.0, 1.0);
        // 振幅変調1.0でamp_factor=0 → 出力は常に0
        for _ in 0..10 {
            assert_eq!(op.tick(sr, &wave, 0.0, 69), 0.0);
        }
    }
}
