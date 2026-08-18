//! op505-core: ym38x6のEG（レート方式5段）をN点Time/Level方式（`sound_core::TimeEg`）に
//! 全面移行した新チップのコアクレート。
//!
//! EG非依存の安定部分（アルゴリズム結線・波形・チップ内LFO・パラメーターマッピングテーブル・
//! 質感LFO）は`sound-fm`（`ym38x6-core`と共有する兄弟クレート）へ直接依存する
//! （fork-on-write方針。将来op505独自に進化させたくなったら該当モジュールだけコピーして依存を切る）。
//! `ym38x6-core`への依存は`[dev-dependencies]`のみ（`examples/op505_probe.rs`が既存`.38x6`
//! からの変換に使う）。`src/`は一切依存しない（op505/デフォーク計画Phase 4）。
//! EG関連（オペレーターEG・Pitch/Cutoff/Gain FG）は全面的にTimeEg化するため、
//! `operator.rs`とChannel/Engine部のみ複製・改変する。

pub mod eg_convert;
pub mod operator;
pub mod preset;
pub use operator::Op505OperatorParams;
pub use preset::{op505_presets_dir, Op505Preset, Op505PresetBank, Op505PresetEntry, Op505PresetFile};

use std::collections::BTreeMap;

use sound_fm::algorithm::ALGORITHMS;
use sound_fm::chip_lfo::{ams_to_depth, pms_to_cents_range, ChipLfo};
use sound_fm::mapping::{
    carrier_velocity_gain, feedback_to_scale_with_max, frequency_to_note, velocity_to_volume_gain,
    FM_MODULATION_INDEX_SCALE,
};
use sound_fm::waveform::{self, gen_builtin_waveform};
use sound_fm::{texture_lfo_to_shape, FmLfoDestination, TextureLfo};
use operator::Operator;
use serde::{Deserialize, Serialize};
use sound_core::{
    apply_lfo_modulation, convert_wave_32, cutoff_to_hz, effective_cutoff, pitch_depth_cents,
    tempo_speed_scale, volume_depth, cutoff_depth, FilterType, LfoDestination, PerformanceLfo,
    PerformanceLfoTarget, Svf, TimeEg, TimeEgParams, TimeStage, Vco, WaveTable,
    RETRIGGER_MODE_RESET,
};

// ---------------------------------------------------------------------------
// パッチ（チャンネル + オペレーター4個分のパラメーター一式）
// ---------------------------------------------------------------------------

/// Pitch/Cutoff FG：バイポーラDepth(中心128)＋TimeEgParams。ym38x6の`BipolarFg`のTimeEg版。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op505BipolarFg {
    pub eg: TimeEgParams,
    pub depth: u8,
}

impl Default for Op505BipolarFg {
    /// `#[derive(Default)]`だと`depth`が0（バイポーラ中心128からもっとも外れた値）になり、
    /// 新規パッチのP.DEP±/F.DEP±ノブが最大逆方向で始まってしまうため明示する。
    fn default() -> Self {
        Self { eg: TimeEgParams::default(), depth: 128 }
    }
}

/// Gain FG：Depthなし、TimeEgParamsそのもの（ym38x6の`GainFg`のTimeEg版）。
pub type Op505GainFg = TimeEgParams;

/// `Op505ChannelParams`の一部（`chip_lfo_*`等）を除く、チャンネル単位パラメーター一式。
/// ym38x6の`ChannelParams`から、旧`ChannelParamsWire`後方互換層（フィールドリネーム・
/// 旧filter_eg_*/vca_eg_*からの移行）を取り除いた素直な新フォーマット。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op505ChannelParams {
    pub algorithm: u8,
    pub feedback: u8,
    pub chip_lfo_freq: u8,
    pub chip_lfo_pmd: u8,
    pub chip_lfo_amd: u8,
    pub chip_lfo_delay: u8,
    pub pms: u8,
    pub ams: u8,
    pub filter_cutoff: u8,
    pub filter_resonance: u8,
    pub filter_type: u8,
    pub filter_self_oscillation: bool,
    /// Pitch FG：ピッチ変調の一次源。バイポーラDepth(中心128)でキーオン一発のピッチ
    /// 下降/上昇とループ時のビブラートの両方を作れる。
    pub pitch_fg: Op505BipolarFg,
    /// Cutoff FG：バイポーラDepth化されたカットオフ変調。
    pub cutoff_fg: Op505BipolarFg,
    /// Gain FG：静止を挟んだ2値スイッチ（トレモロ/ゲート）等、TimeEgの本命ユースケース。
    pub gain_fg: Op505GainFg,
    /// 質感LFO（旧チャンネルLFOを5波形に絞って再編、焼き込み専用）。型は`ym38x6-core`を再利用する
    /// （EG非依存のため、fork-on-write方針で独自進化させたくなるまでは複製しない）。
    pub texture_lfo: TextureLfo,
}

/// `Gain FG`の透過既定：stage0(time=0,level=255)を即座に到達しそのまま静止し、
/// リリース区間を持たない（`release_point`が最終段＝note-offが何もしない）＝ゲートを一切閉じない。
/// 発音終了は各オペレーターのidle判定のみで行う（ym38x6の`default_gain_fg`の設計意図を踏襲）。
///
/// この1段構成はGain FG専用。OP1〜4 EGとPitch/Cutoff FGはエディタ側でSTAGE>=2と最終段level=0を
/// 強制するため必ずリリース区間を持つ（`ui_core::TimeEgProfile`参照）。Gain FGだけ例外なのは、
/// 出力への乗算でボイス解放に関与せず、level 0が「無音」を意味してしまうため。
fn default_gain_fg() -> Op505GainFg {
    TimeEgParams {
        stages: {
            let mut stages = [TimeStage::default(); sound_core::MAX_STAGES];
            stages[0] = TimeStage { time: 0, level: 255, curve: 0 };
            stages
        },
        stage_count: 1,
        loop_enabled: 0,
        loop_start: 0,
        release_point: 0,
     ..Default::default()}
}

impl Default for Op505ChannelParams {
    fn default() -> Self {
        Self {
            algorithm: 0,
            feedback: 0,
            chip_lfo_freq: 0,
            chip_lfo_pmd: 0,
            chip_lfo_amd: 0,
            chip_lfo_delay: 0,
            pms: 0,
            ams: 0,
            filter_cutoff: 255,
            filter_resonance: 0,
            filter_type: 0,
            filter_self_oscillation: true,
            pitch_fg: Op505BipolarFg::default(),
            cutoff_fg: Op505BipolarFg::default(),
            gain_fg: default_gain_fg(),
            texture_lfo: TextureLfo::default(),
        }
    }
}

/// 4op分のオペレーターパラメーター + チャンネルパラメーターの一式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Op505Patch {
    pub operators: [Op505OperatorParams; 4],
    pub channel: Op505ChannelParams,
}

// ---------------------------------------------------------------------------
// チャンネル（4オペレーター + アルゴリズム結線）
// ---------------------------------------------------------------------------

/// フィードバック帰還スケールの上限。ym38x6の`set_feedback_scale_max`のような実行時
/// 上書きは複製せず固定値にする（A/B比較用の実験フラグはym38x6側に残す。fork-on-write）。
const FEEDBACK_SCALE_MAX: f32 = 1.8;

struct Channel {
    operators: [Operator; 4],
    channel_params: Op505ChannelParams,
    /// フィードバックオペレーターの直前の出力（自己変調に使う）。
    feedback_buffer: f32,
    /// 2サンプル平均帰還用の、さらに1つ前の出力out[n-2]（実機OPM/OPN/OPZ準拠、常時有効）。
    feedback_buffer2: f32,
    note: u8,
    base_frequency: f32,
    velocity: u8,
    perf_lfo: PerformanceLfo,
    /// Pitch FGのキーオン連動エンベロープ（TimeEg）。
    pitch_fg_eg: TimeEg,
    /// CC76（Vibrato Rate）由来のPitch FG速さスケール（1.0=無補正）。
    pitch_fg_rate_scale: f32,
    pitch_mod_cents: f32,
    bend_cents: f32,
    volume_mod_delta: f32,
    tl_carrier_mod_delta: f32,
    cutoff_mod_delta: f32,
    channel_gain: f32,
    chip_lfo: ChipLfo,
    /// 4op合成後に適用するSVF本体。CutoffのEG変調は`cutoff_fg_eg`が別途計算し、
    /// `effective_cutoff`で合成してから渡す（sound-coreの`VoiceFilter`のような一体型ではなく
    /// 分解結線。sound-core側にEG方式ごとの並行実装を増やさないための設計、plan参照）。
    svf: Svf,
    /// Cutoff FGのキーオン連動エンベロープ（TimeEg）。
    cutoff_fg_eg: TimeEg,
    /// Gain FGのキーオン連動エンベロープ（TimeEg）。`sound_core::VoiceAmp`は使わず、
    /// tick結果をそのままゲイン乗算する（実体は乗算1行のため分解結線で十分）。
    gain_fg_eg: TimeEg,
    note_is_off: bool,
}

impl Channel {
    fn new(frequency: f32, velocity: u8, patch: Op505Patch) -> Self {
        let note = frequency_to_note(frequency);
        let algo = &ALGORITHMS[(patch.channel.algorithm as usize).min(7)];
        let operators = std::array::from_fn(|i| {
            let mut op = Operator::new(patch.operators[i]);
            op.set_carrier(algo.carriers.contains(&i));
            op.note_on(frequency, velocity);
            op
        });
        let mut pitch_fg_eg = TimeEg::new();
        pitch_fg_eg.note_on();
        let mut cutoff_fg_eg = TimeEg::new();
        cutoff_fg_eg.note_on();
        let mut gain_fg_eg = TimeEg::new();
        gain_fg_eg.note_on();
        let mut perf_lfo = PerformanceLfo::new();
        perf_lfo.set_rate(patch.channel.texture_lfo.rate);
        perf_lfo.set_delay(patch.channel.texture_lfo.delay);
        perf_lfo.set_shape(texture_lfo_to_shape(patch.channel.texture_lfo));
        perf_lfo.note_on();
        Self {
            operators,
            channel_params: patch.channel,
            feedback_buffer: 0.0,
            feedback_buffer2: 0.0,
            note,
            base_frequency: frequency,
            velocity,
            perf_lfo,
            pitch_fg_eg,
            pitch_fg_rate_scale: 1.0,
            pitch_mod_cents: 0.0,
            bend_cents: 0.0,
            volume_mod_delta: 0.0,
            tl_carrier_mod_delta: 0.0,
            cutoff_mod_delta: 0.0,
            channel_gain: 1.0,
            chip_lfo: ChipLfo::new(),
            svf: Svf::new(),
            cutoff_fg_eg,
            gain_fg_eg,
            note_is_off: false,
        }
    }

    /// 各FGの`eg.retrigger_mode`がRESETなら`note_on()`と同じく0からクリーンに再スタートし、
    /// 既定のCONTINUEなら現在レベルを保ったまま段0へ向かう（実機OPMのKey-On挙動、
    /// オペレーターの`retrigger()`と同じ使い分け）。
    fn retrigger(&mut self, frequency: f32, velocity: u8, patch: Op505Patch) {
        let note = frequency_to_note(frequency);
        let algo = &ALGORITHMS[(patch.channel.algorithm as usize).min(7)];
        for (i, op) in self.operators.iter_mut().enumerate() {
            op.params = patch.operators[i];
            op.set_carrier(algo.carriers.contains(&i));
            op.retrigger(frequency, velocity);
        }
        self.channel_params = patch.channel;
        self.note = note;
        self.base_frequency = frequency;
        self.velocity = velocity;
        retrigger_time_eg(&mut self.cutoff_fg_eg, &patch.channel.cutoff_fg.eg);
        retrigger_time_eg(&mut self.gain_fg_eg, &patch.channel.gain_fg);
        retrigger_time_eg(&mut self.pitch_fg_eg, &patch.channel.pitch_fg.eg);
        self.perf_lfo.set_rate(patch.channel.texture_lfo.rate);
        self.perf_lfo.set_delay(patch.channel.texture_lfo.delay);
        self.perf_lfo.set_shape(texture_lfo_to_shape(patch.channel.texture_lfo));
        self.perf_lfo.note_on();
        self.note_is_off = false;
    }

    fn note_off(&mut self) {
        for op in self.operators.iter_mut() {
            op.note_off();
        }
        self.cutoff_fg_eg.note_off();
        self.gain_fg_eg.note_off();
        self.pitch_fg_eg.note_off();
        self.perf_lfo.note_off();
        self.note_is_off = true;
    }

    /// ボイススチールの優先度スコア。値が小さいほど先に奪われる。
    fn steal_score(&self) -> f32 {
        let algo = &ALGORITHMS[(self.channel_params.algorithm as usize).min(7)];
        let carrier_level = algo
            .carriers
            .iter()
            .map(|&i| self.operators[i].env_level())
            .fold(0.0f32, f32::max);
        if self.note_is_off {
            carrier_level
        } else {
            carrier_level + 10.0
        }
    }

    fn note_on_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_on(self.base_frequency, self.velocity);
    }

    fn note_off_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_off();
    }

    fn is_idle(&self) -> bool {
        self.operators.iter().all(|op| op.is_idle())
    }

    fn tick(&mut self, sample_rate: f32, wave_tables: &[Option<WaveTable>], tempo_bpm: f32) -> f32 {
        if self.is_idle() {
            return 0.0;
        }

        let texture_lfo_value = self.perf_lfo.tick(sample_rate);
        let texture_lfo = self.channel_params.texture_lfo;
        self.pitch_mod_cents = 0.0;
        self.volume_mod_delta = 0.0;
        self.tl_carrier_mod_delta = 0.0;
        self.cutoff_mod_delta = 0.0;
        match FmLfoDestination::from_u8(texture_lfo.destination) {
            FmLfoDestination::Pitch => {
                let cents = pitch_depth_cents(texture_lfo.depth, 0, 0);
                apply_lfo_modulation(texture_lfo_value, LfoDestination::Pitch, cents, self);
            }
            FmLfoDestination::Volume => {
                let depth = volume_depth(texture_lfo.depth, 0);
                apply_lfo_modulation(texture_lfo_value, LfoDestination::Volume, depth, self);
            }
            FmLfoDestination::TlCarrier => {
                self.tl_carrier_mod_delta = texture_lfo_value * volume_depth(texture_lfo.depth, 0);
            }
            FmLfoDestination::Cutoff => {
                self.cutoff_mod_delta = texture_lfo_value * cutoff_depth(texture_lfo.depth, 0);
            }
            FmLfoDestination::Unplugged => {}
        }

        // Pitch FG：ループ可能TimeEgでビブラート/シンセタムを作る一次源。
        // CC76由来の速度補正(pitch_fg_rate_scale)とテンポ同期(tempo_speed_scale)を乗算で共存させる。
        let pitch_fg = self.channel_params.pitch_fg;
        let pitch_fg_speed = self.pitch_fg_rate_scale * tempo_speed_scale(&pitch_fg.eg, tempo_bpm);
        let pitch_fg_out = self.pitch_fg_eg.tick(sample_rate, pitch_fg.eg, pitch_fg_speed);
        let pitch_fg_cents = pitch_fg_out * (pitch_fg.depth as f32 - 128.0) / 128.0 * 1200.0;
        for op in self.operators.iter_mut() {
            op.set_pitch_modulation(self.pitch_mod_cents + self.bend_cents + pitch_fg_cents);
        }

        // チップ内LFO（プリセット・NRPNで設定する音作り用、PMS/AMS×PMD/AMD）
        let chip_lfo_value = self.chip_lfo.tick(
            sample_rate,
            self.channel_params.chip_lfo_freq,
            self.channel_params.chip_lfo_delay,
        );
        let chip_pitch_mod_cents = chip_lfo_value
            * pms_to_cents_range(self.channel_params.pms)
            * (self.channel_params.chip_lfo_pmd as f32 / 255.0);
        let chip_amp_mod = chip_lfo_value
            * ams_to_depth(self.channel_params.ams)
            * (self.channel_params.chip_lfo_amd as f32 / 255.0);
        for op in self.operators.iter_mut() {
            let am = if op.params.am_enable { chip_amp_mod } else { 0.0 };
            op.set_chip_lfo_modulation(chip_pitch_mod_cents, am);
        }

        // アルゴリズム結線に基づく4op合成
        let algo = &ALGORITHMS[(self.channel_params.algorithm as usize).min(7)];
        let mut op_outputs = [0.0f32; 4];
        for &op_idx in algo.eval_order.iter() {
            let mut modulation = 0.0;
            for &(from, to) in algo.routes {
                if to == op_idx {
                    modulation += op_outputs[from] * FM_MODULATION_INDEX_SCALE;
                }
            }
            if op_idx == algo.feedback_op {
                let fb_source = 0.5 * (self.feedback_buffer + self.feedback_buffer2);
                let scale = feedback_to_scale_with_max(self.channel_params.feedback, FEEDBACK_SCALE_MAX);
                modulation += fb_source * scale;
            }
            let wave = wave_table_for(wave_tables, self.operators[op_idx].params.waveform);
            let out = self.operators[op_idx].tick(sample_rate, wave, modulation, self.note, tempo_bpm);
            op_outputs[op_idx] = out;
            if op_idx == algo.feedback_op {
                self.feedback_buffer2 = self.feedback_buffer;
                self.feedback_buffer = out;
            }
        }

        let all_full_velocity_gain =
            algo.carriers.iter().all(|&i| self.operators[i].params.velocity_gain == 255);
        let carrier_sum: f32 = if all_full_velocity_gain {
            let sum: f32 = algo.carriers.iter().map(|&i| op_outputs[i]).sum();
            sum * velocity_to_volume_gain(self.velocity)
        } else {
            algo.carriers
                .iter()
                .map(|&i| {
                    op_outputs[i]
                        * carrier_velocity_gain(self.operators[i].params.velocity_gain, self.velocity)
                })
                .sum()
        };
        let tl_carrier_gain = (1.0 + self.tl_carrier_mod_delta).max(0.0);
        let volume_gain = (1.0 + self.volume_mod_delta).max(0.0);
        let dry = carrier_sum * tl_carrier_gain * volume_gain * self.channel_gain;

        // VCF：Cutoff FG（TimeEg）を先にtickし、effective_cutoffで基準Cutoffと合成してからSvfへ。
        let cp = &self.channel_params;
        let modulated_cutoff =
            (cp.filter_cutoff as f32 + self.cutoff_mod_delta).round().clamp(0.0, 255.0) as u8;
        let cutoff_fg_speed = tempo_speed_scale(&cp.cutoff_fg.eg, tempo_bpm);
        let cutoff_level = self.cutoff_fg_eg.tick(sample_rate, cp.cutoff_fg.eg, cutoff_fg_speed);
        let cutoff = effective_cutoff(modulated_cutoff, cutoff_level, cp.cutoff_fg.depth);
        let cutoff_hz = cutoff_to_hz(cutoff);
        let filtered = self.svf.process(
            dry,
            sample_rate,
            cutoff_hz,
            cp.filter_resonance,
            cp.filter_self_oscillation,
            FilterType::from_u8(cp.filter_type),
        );

        // VCA：Gain FG（TimeEg）をtickしてゲイン乗算するだけ（VoiceAmpの実体と同じ1行）。
        let gain_fg_speed = tempo_speed_scale(&cp.gain_fg, tempo_bpm);
        let gain = self.gain_fg_eg.tick(sample_rate, cp.gain_fg, gain_fg_speed);
        filtered * gain
    }
}

/// FGの`eg.retrigger_mode`に応じてretrigger()/note_on()を使い分ける
/// （`Operator::retrigger()`と同じ分岐をChannel側の3FGへ揃える）。
fn retrigger_time_eg(eg: &mut TimeEg, params: &TimeEgParams) {
    if params.retrigger_mode == RETRIGGER_MODE_RESET {
        eg.note_on();
    } else {
        eg.retrigger();
    }
}

impl PerformanceLfoTarget for Channel {
    fn apply_pitch_modulation(&mut self, cents: f32) {
        self.pitch_mod_cents = cents;
    }

    fn apply_volume_modulation(&mut self, delta: f32) {
        self.volume_mod_delta = delta;
    }
}

/// 指定スロットの波形テーブルを返す。未割り当てスロットはスロット0（サイン波）にフォールバックする。
fn wave_table_for(wave_tables: &[Option<WaveTable>], slot: u8) -> &WaveTable {
    wave_tables[slot as usize]
        .as_ref()
        .unwrap_or_else(|| wave_tables[0].as_ref().unwrap())
}

// ---------------------------------------------------------------------------
// op505 エンジン
// ---------------------------------------------------------------------------

const TOTAL_SLOTS: usize = 256;

/// 同時発音数の既定上限。ym38x6-coreの実測（[[project_voice_steal_and_eg_lut_optimization]]）に
/// 基づく値をそのまま引き継ぐ（通常演奏でヘッドルームを持ちつつストレス時の負荷増大を抑える）。
const DEFAULT_MAX_VOICES: usize = 64;

pub struct Op505Engine {
    sample_rate: f32,
    /// ボイスID→チャンネル。`BTreeMap`でID昇順の決定論的イテレーション順を保証する
    /// （浮動小数点加算順序を固定し、同一入力で出力WAVがビット一致するようにする）。
    channels: BTreeMap<usize, Channel>,
    wave_tables: Vec<Option<WaveTable>>,
    current_patch: Op505Patch,
    max_voices: usize,
    mix_buf: Vec<f32>,
    /// TimeEgのテンポ同期（`sync_enabled`）に使うBPM。ホストDAWのTransport（VST）や
    /// タップテンポ（gesture-app）から`set_tempo()`経由で設定される。既定120。
    tempo_bpm: f32,
}

impl Op505Engine {
    pub fn new(sample_rate: f32) -> Self {
        let mut wave_tables: Vec<Option<WaveTable>> = (0..TOTAL_SLOTS).map(|_| None).collect();
        for i in 0..waveform::BUILTIN_WAVEFORM_COUNT {
            wave_tables[i as usize] = Some(gen_builtin_waveform(i));
        }
        Self {
            sample_rate,
            channels: BTreeMap::new(),
            wave_tables,
            current_patch: Op505Patch::default(),
            max_voices: DEFAULT_MAX_VOICES,
            mix_buf: Vec::new(),
            tempo_bpm: 120.0,
        }
    }

    pub fn set_max_voices(&mut self, max_voices: usize) {
        self.max_voices = max_voices.max(1);
    }

    fn steal_one_voice(&mut self) {
        if let Some(steal_id) = self
            .channels
            .iter()
            .min_by(|a, b| a.1.steal_score().partial_cmp(&b.1.steal_score()).unwrap())
            .map(|(&id, _)| id)
        {
            self.channels.remove(&steal_id);
        }
    }

    pub fn set_patch(&mut self, patch: Op505Patch) {
        self.current_patch = patch;
    }

    /// `set_patch`と同様に`current_patch`を更新した上で、現在発音中の全チャンネルにも
    /// 反映する（GUI/DAWノブ変更相当）。
    pub fn set_patch_live(&mut self, patch: Op505Patch) {
        self.current_patch = patch;
        let channel_ids: Vec<usize> = self.channels.keys().copied().collect();
        for channel in channel_ids {
            self.set_channel_params(channel, patch.channel);
            for (op_index, op) in patch.operators.iter().enumerate() {
                self.set_operator_params(channel, op_index, *op);
            }
        }
    }

    pub fn current_patch(&self) -> Op505Patch {
        self.current_patch
    }

    pub fn set_channel_params(&mut self, channel: usize, params: Op505ChannelParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.perf_lfo.set_rate(params.texture_lfo.rate);
            ch.perf_lfo.set_delay(params.texture_lfo.delay);
            ch.perf_lfo.set_shape(texture_lfo_to_shape(params.texture_lfo));
            ch.channel_params = params;
        }
    }

    pub fn set_operator_params(&mut self, channel: usize, op_index: usize, params: Op505OperatorParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].params = params;
        }
    }

    pub fn set_operator_f_number(&mut self, channel: usize, op_index: usize, f_number: u16) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].set_f_number_override(f_number);
        }
    }

    pub fn set_pitch_fg_rate_scale(&mut self, channel: usize, scale: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.pitch_fg_rate_scale = scale;
        }
    }

    pub fn note_on_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_on_operator(op_index);
        }
    }

    pub fn note_off_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off_operator(op_index);
        }
    }

    pub fn set_user_wave(&mut self, slot: u8, input: &[i8; 32]) {
        assert!(slot >= 8, "slots 0-7 are reserved for builtin waves");
        self.wave_tables[slot as usize] = Some(convert_wave_32(input));
    }

    pub fn silence_group(&mut self, group: usize) {
        self.channels.retain(|id, _| id >> 7 != group);
    }

    pub fn active_voice_count(&self) -> usize {
        self.channels.len()
    }

    /// 発音中のボイスIDを`out`へ書き出す（`out`は呼び出し側が事前に確保する。
    /// オーディオスレッドからの毎ブロック呼び出しでアロケーションが走らないようにするため）。
    pub fn collect_active_channels(&self, out: &mut Vec<usize>) {
        out.clear();
        out.extend(self.channels.keys().copied());
    }
}

impl Vco for Op505Engine {
    fn note_on(&mut self, channel: usize, frequency: f32, velocity: u8) {
        let patch = self.current_patch;
        if let Some(ch) = self.channels.get_mut(&channel) {
            if !ch.is_idle() {
                ch.retrigger(frequency, velocity, patch);
                return;
            }
        } else if self.channels.len() >= self.max_voices {
            self.steal_one_voice();
        }
        self.channels.insert(channel, Channel::new(frequency, velocity, patch));
    }

    fn note_off(&mut self, channel: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off();
        }
    }

    fn set_pitch_bend(&mut self, channel: usize, cents: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.bend_cents = cents;
        }
    }

    fn set_pitch_bend_group(&mut self, group: usize, cents: f32) {
        for (id, ch) in self.channels.iter_mut() {
            if id >> 7 == group {
                ch.bend_cents = cents;
            }
        }
    }

    fn set_channel_volume(&mut self, channel: usize, gain: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.channel_gain = gain.max(0.0);
        }
    }

    fn set_channel_volume_group(&mut self, group: usize, gain: f32) {
        for (id, ch) in self.channels.iter_mut() {
            if id >> 7 == group {
                ch.channel_gain = gain.max(0.0);
            }
        }
    }

    /// TimeEgのテンポ同期（`sync_enabled`）に使うBPMを更新する。0以下は無視する
    /// （ホストDAWのTransportが未再生等で`tempo`を返さない場合の防御、既存値を保持する）。
    fn set_tempo(&mut self, bpm: f32) {
        if bpm > 0.0 {
            self.tempo_bpm = bpm;
        }
    }

    fn render(&mut self, output: &mut [f32], num_channels: usize) {
        let num_channels = num_channels.max(1);
        let sample_rate = self.sample_rate;
        let wave_tables = &self.wave_tables;
        let tempo_bpm = self.tempo_bpm;
        let frames = output.len().div_ceil(num_channels);
        self.mix_buf.clear();
        self.mix_buf.resize(frames, 0.0);
        for ch in self.channels.values_mut() {
            for mix in self.mix_buf.iter_mut() {
                if ch.is_idle() {
                    break;
                }
                *mix += ch.tick(sample_rate, wave_tables, tempo_bpm);
            }
        }
        for (frame, &mix) in output.chunks_mut(num_channels).zip(self.mix_buf.iter()) {
            for s in frame.iter_mut() {
                *s += mix;
            }
        }
        self.channels.retain(|_, ch| !ch.is_idle());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn stages_with(entries: &[(u8, u8, u8)]) -> [TimeStage; sound_core::MAX_STAGES] {
        let mut stages = [TimeStage::default(); sound_core::MAX_STAGES];
        for (i, &(time, level, curve)) in entries.iter().enumerate() {
            stages[i] = TimeStage { time, level, curve };
        }
        stages
    }

    /// 瞬時に満レベルへ到達しそのまま無限サスティンするEG（ym38x6版loud_patchのAR=255/D1L=255相当）。
    /// 段1はリリース用（OP EGは必ずレベル0へ着地させる。`ui_core::TimeEgProfile`参照）。
    fn instant_sustain_eg() -> TimeEgParams {
        TimeEgParams {
            stages: stages_with(&[(0, 255, 0), (0, 0, 0)]),
            stage_count: 2,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 0,
         ..Default::default()}
    }

    /// 全Opがアルゴリズム7（全並列）で即音量最大・サスティン無限のテスト用パッチ。
    fn loud_patch(velocity_sensitivity: u8) -> Op505Patch {
        let op_params = Op505OperatorParams {
            tl: 255,
            eg: instant_sustain_eg(),
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity,
            waveform: 0,
            op_fine_tune: 128,
            eg_shift: 0,
            level_scale: 0,
            velocity_gain: 255,
        };
        let mut patch = Op505Patch::default();
        patch.operators = [op_params; 4];
        patch.channel.algorithm = 7;
        patch
    }

    #[test]
    fn voice_steal_caps_channel_count() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(4);
        for ch in 0..8 {
            engine.note_on(ch, 440.0, 100);
        }
        assert_eq!(engine.channels.len(), 4, "上限を超えて積み上がってはいけない");
    }

    #[test]
    fn voice_steal_prefers_released_over_held() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(2);
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_off(0);

        engine.note_on(2, 440.0, 100);

        assert!(!engine.channels.contains_key(&0), "release中のボイスが優先的に奪われるはず");
        assert!(engine.channels.contains_key(&1), "発音中のボイスは残るはず");
        assert!(engine.channels.contains_key(&2), "新規ボイスは確保されるはず");
        assert_eq!(engine.channels.len(), 2);
    }

    #[test]
    fn collect_active_channels_reports_only_sounding_voices() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_on(2, 440.0, 100);

        let mut ids = Vec::new();
        engine.collect_active_channels(&mut ids);
        assert_eq!(ids, vec![0, 1, 2], "BTreeMapのキー昇順で決定論的に列挙されるはず");

        engine.silence_group(0);
        engine.collect_active_channels(&mut ids);
        assert!(ids.is_empty(), "silence_groupで消えたチャンネルは含まれないはず");
    }

    #[test]
    fn retrigger_does_not_trigger_steal() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.set_max_voices(2);
        engine.note_on(0, 440.0, 100);
        engine.note_on(1, 440.0, 100);
        engine.note_on(0, 880.0, 100);

        assert!(engine.channels.contains_key(&0));
        assert!(engine.channels.contains_key(&1), "retriggerが他ボイスを巻き込んで奪ってはいけない");
        assert_eq!(engine.channels.len(), 2);
    }

    /// ブロック一括レンダリングと1サンプルずつのレンダリングがビット単位で一致することを確認する
    /// （render()のチャンネル外側×サンプル内側ループが数値的に完全等価であることの回帰テスト）。
    /// フィードバック・フィルター・リリース途中のidle化まで通る条件で比較する。
    #[test]
    fn block_render_matches_sample_by_sample_render() {
        let mut patch = loud_patch(0);
        patch.channel.feedback = 100;
        patch.channel.filter_cutoff = 200;
        patch.channel.filter_resonance = 80;
        // 瞬時attack→decay(40)で静止、リリース段(2)から短時間でidleへ落ちる形。
        let eg = TimeEgParams {
            stages: stages_with(&[(0, 255, 0), (60, 40, 0), (60, 0, 0)]),
            stage_count: 3,
            loop_enabled: 0,
            loop_start: 0,
            release_point: 1,
         ..Default::default()};
        for op in patch.operators.iter_mut() {
            op.eg = eg;
        }

        let mut block = Op505Engine::new(44100.0);
        let mut single = Op505Engine::new(44100.0);
        for engine in [&mut block, &mut single] {
            engine.set_patch(patch);
            engine.note_on(0, 220.0, 100);
            engine.note_on(1, 330.0, 90);
            engine.note_on(2, 440.0, 127);
        }

        const HALF: usize = 2048;
        let mut out_block = vec![0.0f32; HALF * 2];
        let mut out_single = vec![0.0f32; HALF * 2];

        block.render(&mut out_block[..HALF], 1);
        for i in 0..HALF {
            single.render(&mut out_single[i..i + 1], 1);
        }
        for engine in [&mut block, &mut single] {
            engine.note_off(0);
            engine.note_off(1);
            engine.note_off(2);
        }
        block.render(&mut out_block[HALF..], 1);
        for i in HALF..HALF * 2 {
            single.render(&mut out_single[i..i + 1], 1);
        }

        assert!(out_block.iter().any(|&s| s != 0.0), "expected non-silent output");
        for (i, (a, b)) in out_block.iter().zip(out_single.iter()).enumerate() {
            assert_eq!(a, b, "sample {i} differs: block={a}, single={b}");
        }
    }

    #[test]
    fn note_on_produces_non_silent_output() {
        let mut engine = Op505Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.note_on(0, 440.0, 100);
        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        assert!(buf.iter().any(|&s| s != 0.0), "expected non-silent output");
    }

    /// 全8アルゴリズム×フィルター自己発振ONで、長時間レンダリングしてもNaN/Infが出ないことを確認する。
    #[test]
    fn all_algorithms_long_run_no_nan() {
        let sr = 44100.0;
        for algo in 0u8..8 {
            let mut patch = loud_patch(0);
            patch.channel.algorithm = algo;
            patch.channel.feedback = 150;
            patch.channel.filter_cutoff = 180;
            patch.channel.filter_resonance = 255;
            patch.channel.filter_self_oscillation = true;
            let mut engine = Op505Engine::new(sr);
            engine.set_patch(patch);
            engine.note_on(0, 440.0, 100);
            let mut buf = vec![0.0f32; 4096];
            engine.render(&mut buf, 1);
            assert!(buf.iter().all(|s| s.is_finite()), "algo {algo}: expected finite output");
        }
    }

    #[test]
    fn op505_patch_serde_json_round_trips() {
        let mut patch = loud_patch(64);
        patch.channel.gain_fg = TimeEgParams {
            stages: stages_with(&[(15, 230, 0), (40, 230, 0), (15, 40, 0), (40, 40, 0)]),
            stage_count: 4,
            loop_enabled: 1,
            loop_start: 0,
            release_point: 3,
         ..Default::default()};
        let json = serde_json::to_string(&patch).expect("serialize");
        let restored: Op505Patch = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(patch, restored, "Op505Patch should round-trip through JSON unchanged");
    }
}
