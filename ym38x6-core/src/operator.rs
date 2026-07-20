// ---------------------------------------------------------------------------
// オペレーター（オシレーター + EGエンベロープ）
// ---------------------------------------------------------------------------

use crate::mapping::*;
use crate::waveform::{is_noise_waveform, noise_clock_rate, noise_color};
use serde::{Deserialize, Serialize};
use sound_core::{Eg, EgParams, WaveTable};

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
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。`sound_core::EgParams::floor`に対応。
    #[serde(default)]
    pub floor: u8,
    /// 0=ワンショット（従来のADSR挙動そのまま）/1=ループ。`sound_core::EgParams::loop_enabled`に対応。
    #[serde(rename = "loop", default)]
    pub loop_enabled: u8,
    /// 0=線形（角の立つ三角）/1=サイン風（レイズドコサインで角を丸める）。`sound_core::EgParams::curve`に対応。
    #[serde(default)]
    pub curve: u8,
    /// EGSFT（TX81Z EG Shift）。EGの減衰レンジ(dB)を圧縮する（0〜255、既定0＝圧縮なし＝96dBフルレンジ）。
    /// チップの `env_attenuation >> shift`（TX81Z egsft 0/1/2/3 = 96/48/24/12dB）を連続化したもので、
    /// 値が大きいほどEGの振れ幅が上方（大音量側）へ圧縮される（打鍵時に-96dBでなく高い床から立ち上がる）。
    /// TLはこの圧縮を受けない（チップと同じくEG分のみ圧縮）。[mapping::eg_shift_to_db_range](crate::mapping::eg_shift_to_db_range)参照。
    #[serde(default)]
    pub eg_shift: u8,
    /// Level Scaling（ノート依存の出力レベル減衰、OPL系KSL相当）。0〜255、既定0＝スケーリングなし。
    /// 値が大きいほど高音域で出力(TL)を強く絞る。キャリア・モジュレーター双方に効く（実機TX81Z LS準拠）。
    /// ノート依存の減衰カーブは[mapping::level_scale_atten](crate::mapping::level_scale_atten)が持つ。
    #[serde(default)]
    pub level_scale: u8,
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
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            eg_shift: 0,
            level_scale: 0,
        }
    }
}

pub struct Operator {
    pub params: OperatorParams,
    /// フェーズ3は全Op同一値（Note-On時に一括設定）。フェーズ4のOP単位上書きの土台。
    frequency: f32,
    phase: f32,
    /// キーオン連動エンベロープ（`sound_core::Eg`へ統一、KSRはrate_scale経由で適用）。
    eg: Eg,
    velocity: u8,
    /// チップ内LFOによるピッチ変調（セント、全Op共通、Channelが設定）
    chip_lfo_pitch_mod_cents: f32,
    /// チップ内LFOによる振幅変調（0.0〜1.0、am_enable時のみ非ゼロ、Channelが設定）
    chip_lfo_amp_mod: f32,
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
    /// KSRレート倍率(`ksr_rate_multiplier`、`powf()`を含む)のキャッシュ。(ksr, note)が
    /// 前回と同じならtick()内で再計算しない（ノート中は両方とも不変のため、毎サンプル
    /// powf()を呼んでいた無駄を避ける）。
    cached_rate_scale: f32,
    cached_rate_scale_key: Option<(u8, u8)>,
}

impl Operator {
    pub fn new(params: OperatorParams) -> Self {
        Self {
            params,
            frequency: 440.0,
            phase: 0.0,
            eg: Eg::new(),
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
        self.eg.note_on();
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
        self.eg.retrigger(); // レベルは保持（残響から再アタック）
        self.f_number_ratio = 1.0;
        self.noise_lfsr = 1;
        self.noise_accum = 0.0;
        self.noise_hold = 0.0;
    }

    pub fn note_off(&mut self) {
        self.eg.note_off();
    }

    pub fn is_idle(&self) -> bool {
        self.eg.is_idle()
    }

    /// 現在のEGレベル(0.0〜1.0)。ボイススチールの「最も静かなボイス」判定に使う。
    pub fn env_level(&self) -> f32 {
        self.eg.level()
    }

    /// チップ内LFOによる変調値を設定する（毎サンプル、Channelから呼ばれる）。
    pub fn set_chip_lfo_modulation(&mut self, pitch_cents: f32, amp_mod: f32) {
        self.chip_lfo_pitch_mod_cents = pitch_cents;
        self.chip_lfo_amp_mod = amp_mod;
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
            + self.chip_lfo_pitch_mod_cents
            + self.perf_lfo_pitch_mod_cents;
        self.frequency * self.f_number_ratio * mul_to_ratio(self.params.mul) * 2f32.powf(cents / 1200.0)
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
        let env_level = self.eg.tick(
            sample_rate,
            EgParams {
                ar: self.params.ar,
                d1r: self.params.d1r,
                d1l: self.params.d1l,
                d2r: self.params.d2r,
                rr: self.params.rr,
                floor: self.params.floor,
                loop_enabled: self.params.loop_enabled,
                curve: self.params.curve,
                // OP単位Delayは公開しない（Pitch/Cutoff/Gain FGのみのスコープ、
                // ステップ7プラン参照）。常に0=遅延なしで既存挙動を保つ。
                delay: 0,
            },
            ksr_mul,
        );

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
        let eff_tl = eff_tl.saturating_sub(level_scale_atten(self.params.level_scale, note));
        let amp_factor = (1.0 - self.chip_lfo_amp_mod).clamp(0.0, 1.0);
        // dBリニアエンベロープ（OPN/OPM互換）: env_level=1.0→0dB, 0.0→-db_range。
        // db_rangeは通常96dB（eg_shift=0）。EGSFTはこのEG減衰レンジのみを圧縮し（TLは別掛けで不変）、
        // チップの `env_attenuation >> eg_shift` と厳密一致する。
        let db_range = eg_shift_to_db_range(self.params.eg_shift);
        let env_amp = 10f32.powf(-(1.0 - env_level) * (db_range / 20.0));
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
            floor: 0,
            loop_enabled: 0,
            curve: 0,
            eg_shift: 0,
            level_scale: 0,
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

    /// EG状態機械は`sound_core::Eg`へ統一済み（内部フェーズは公開しない）。
    /// ここではブラックボックスの出力振幅から、Attack→Decay1→Decay2（0に張り付き、
    /// Idleにはならない）→note_off→Release→Idleという一連の遷移が保たれていることを検証する
    /// （fast_paramsはAR/D1R/D2R/RR=255で全区間が数十〜数百サンプルと高速に完了する）。
    /// EGSFT（eg_shift）はEGの減衰レンジを圧縮し、サステインの床を持ち上げる。
    /// d1l=128（env_level≈0.5）でホールドさせ、eg_shift=0（-48dB付近）と
    /// eg_shift=255（12dBレンジ＝床が大幅に持ち上がる）でサステインのピーク振幅を比較する。
    #[test]
    fn eg_shift_raises_sustained_amplitude() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        // Decay2レート0でd1l到達後に張り付かせる
        let base = OperatorParams { d1l: 128, d2r: 0, rr: 255, ..fast_params() };
        let settle = |eg_shift: u8| -> f32 {
            let mut op = Operator::new(OperatorParams { eg_shift, ..base });
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
        let wave = gen_op_sine();
        let mut op = Operator::new(fast_params());
        op.note_on(440.0, 127);
        assert!(!op.is_idle());

        // Attackで一度ピーク付近まで振幅が立ち上がることを確認する。
        let mut peak = 0.0f32;
        for _ in 0..50 {
            peak = peak.max(op.tick(sr, &wave, 0.0, 69).abs());
        }
        assert!(peak > 0.9, "expected attack to reach near-peak amplitude, got {peak}");

        // 十分な時間が経過すると、d1l=128のDecay1を経てd2r=255のDecay2で0近くまで
        // 減衰し（Idleにはならず）張り付く。
        let mut settled = 0.0f32;
        for _ in 0..2000 {
            settled = op.tick(sr, &wave, 0.0, 69).abs();
        }
        assert!(!op.is_idle(), "should still be sounding (Decay2 sticks at 0, not Idle)");
        assert!(settled < 1e-3, "expected amplitude to have decayed near 0, got {settled}");

        // note_off → Release → Idle
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
        // 振幅変調1.0でamp_factor=0 → 出力は常に0
        for _ in 0..10 {
            assert_eq!(op.tick(sr, &wave, 0.0, 69), 0.0);
        }
    }

    /// floor/loop/curveフィールドを持たない旧`.38x6`のJSONを読み込むと、`#[serde(default)]`により
    /// 3項目とも0（loop=0＝従来のADSR挙動）で復元されることを確認する（後方互換）。
    #[test]
    fn deserializes_legacy_json_without_loop_fields_as_disabled() {
        let legacy_json = r#"{
            "tl": 200, "ar": 255, "d1r": 100, "d2r": 80, "d1l": 180, "rr": 150,
            "mul": 1, "dt1": 128, "ksr": 64, "am_enable": false,
            "velocity_sensitivity": 0, "waveform": 0
        }"#;
        let op: OperatorParams = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(op.floor, 0);
        assert_eq!(op.loop_enabled, 0);
        assert_eq!(op.curve, 0);
        assert_eq!(op.op_fine_tune, 128, "op_fine_tuneも既存の後方互換規則どおり中心128");
    }

    /// loop=0(既定)の場合、新規追加したfloor/loop/curveフィールドが0でも従来の5段ADSRと
    /// 完全に同一の出力になることを確認する（既存`.38x6`・コンバーター出力の後方互換担保）。
    #[test]
    fn loop_disabled_matches_classic_envelope_exactly() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let mut classic = fast_params();
        classic.floor = 0;
        classic.loop_enabled = 0;
        classic.curve = 0;
        let mut op_classic = Operator::new(classic);
        let mut op_new = Operator::new(classic); // 同一パラメーターの複製（新フィールドは既定と一致）
        op_classic.note_on(440.0, 127);
        op_new.note_on(440.0, 127);
        for _ in 0..5000 {
            let a = op_classic.tick(sr, &wave, 0.0, 69);
            let b = op_new.tick(sr, &wave, 0.0, 69);
            assert_eq!(a, b, "floor=0/loop=0/curve=0 must match the classic ADSR bit-for-bit");
        }
    }

    /// OP単位ループEG: loop=1で、そのOPのEG出力（＝モジュレーターなら変調指数）が
    /// floorとpeakの間を周期的に往復することを確認する（FM特有のうねりの土台）。
    /// `tick()`は音声波形(サイン)込みの出力を返すため、1音声周期(≈100サンプル@440Hz/44.1kHz)
    /// より十分長いウィンドウ内の最大振幅を追跡し、サインのゼロクロスとEGループを区別する。
    #[test]
    fn loop_enabled_operator_oscillates_between_floor_and_peak() {
        let sr = 44100.0;
        let wave = gen_op_sine();
        let mut params = fast_params();
        params.ar = 200;
        params.d1r = 200;
        // EG出力はdBリニア（-96dB〜0dB）のため、floor=0(完全開閉)ならピーク(≒0dB)⇄
        // ほぼ無音(≒-96dB)を往復する。中間のfloor値は指数変換で振幅の判別が難しくなるため、
        // 「往復しているか」を最も明確に検出できる両端(0/1)を使う。
        params.floor = 0;
        params.loop_enabled = 1;
        let mut op = Operator::new(params);
        op.note_on(440.0, 127);

        // 初回Attack(0→peak)はループの「floor⇄peak往復」ではないため、一度peak付近に
        // 達するまでをウォームアップとして読み捨ててから、往復のウィンドウ観測を始める。
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
