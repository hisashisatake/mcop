pub mod algorithm;
pub mod filter;
pub mod mapping;
pub mod operator;
pub mod preset;
pub mod tone_lfo;
pub mod waveform;

use std::collections::HashMap;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use algorithm::ALGORITHMS;
use filter::{cutoff_to_hz, effective_cutoff, FilterEnvelope, FilterType, Svf};
use mapping::{
    feedback_to_scale_with_max, frequency_to_note, velocity_to_volume_gain,
    FM_MODULATION_INDEX_SCALE,
};
use operator::Operator;

// ---------------------------------------------------------------------------
// EXPERIMENT(fb-2sample): フィードバック帰還方式の実験スイッチ（デフォルト=現状の1サンプル帰還）
//
// 実機OPM/OPN/OPZは feedback 経路を (out[n-1]+out[n-2])/2 の2サンプル平均で帰還する。
// 38x6は従来 out[n-1] のみの1サンプル帰還で、その弱さを feedback_to_scale の max=1.8 で
// 補正してきた。OPZ音色の倍音構造（H3ディップ/H4-6棚）を実機に近づけられるか測定するため、
// 2サンプル平均経路と feedback_to_scale 最大値の override をプロセスグローバルで切替可能にする。
// production 既定は false / max=1.8 のままで挙動は不変。
//
// 測定結果(2026-06-20): 2サンプル平均は音色帯(基音415Hz)で1サンプルとほぼ完全一致し、OPZ
// 忠実度を上げる効果は無かった（高周波しか効かない）。本足場は高周波/将来の feedback 調査用に
// 維持する。一括除去する場合は "EXPERIMENT(fb-2sample)" で grep すること。
// ---------------------------------------------------------------------------

/// true で feedback 経路を (out[n-1]+out[n-2])/2 の2サンプル平均にする（既定 false）。
static FEEDBACK_TWO_SAMPLE: AtomicBool = AtomicBool::new(false);
/// feedback_to_scale の最大値 override。f32 を to_bits で格納し 0 を「未設定（=1.8）」とする。
static FEEDBACK_SCALE_MAX_BITS: AtomicU32 = AtomicU32::new(0);

/// 実験用: feedback 帰還方式を 2サンプル平均にするか設定する。
pub fn set_feedback_two_sample(enabled: bool) {
    FEEDBACK_TWO_SAMPLE.store(enabled, Ordering::Relaxed);
}

/// 実験用: feedback_to_scale の最大値を override する。None で既定(1.8)へ戻す。
pub fn set_feedback_scale_max(max: Option<f32>) {
    let bits = match max {
        Some(m) if m > 0.0 => m.to_bits(),
        _ => 0,
    };
    FEEDBACK_SCALE_MAX_BITS.store(bits, Ordering::Relaxed);
}

fn current_feedback_scale_max() -> f32 {
    let bits = FEEDBACK_SCALE_MAX_BITS.load(Ordering::Relaxed);
    if bits == 0 {
        1.8
    } else {
        f32::from_bits(bits)
    }
}
// Ym38x6Patch::operators / set_operator_paramsの型として外部に公開する
pub use operator::OperatorParams;
pub use preset::{
    gm2_bank0_patch, placeholder_patch, presets_dir, waveform_memory_patch, Preset, PresetBank,
    PresetEntry, PresetFile, WAVEFORM_MEMORY_BANK,
};
use serde::{Deserialize, Serialize};
use sound_core::{
    apply_lfo_modulation, convert_wave_32, PerformanceLfo, PerformanceLfoTarget, WaveTable,
};
use tone_lfo::{ams_to_depth, pms_to_cents_range, ToneLfo};
use waveform::gen_builtin_waveform;

// 呼び出し側がsound-coreに直接依存しなくて済むようre-export
pub use sound_core::{
    pitch_depth_cents, volume_depth, AdsrParams, ChorusType, LfoDestination, LfoWaveform,
    MasterEffects, ReverbType, SoundEngine,
};

// ---------------------------------------------------------------------------
// パッチ（チャンネル + オペレーター4個分のパラメーター一式）
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelParams {
    /// アルゴリズム番号(0〜7)。
    pub algorithm: u8,
    /// フィードバック深さ(0〜255)。
    pub feedback: u8,
    /// 音色LFO周波数。
    pub tone_lfo_freq: u8,
    /// 音色LFO ピッチ変調深さ。
    pub tone_lfo_pmd: u8,
    /// 音色LFO 振幅変調深さ。
    pub tone_lfo_amd: u8,
    /// 音色LFO Delay。
    pub tone_lfo_delay: u8,
    /// PM感度（音色LFOのピッチ変調感度）。
    pub pms: u8,
    /// AM感度（音色LFOの振幅変調感度）。
    pub ams: u8,
    /// フィルターCutoff(0〜255、spec.md「フィルター」セクション参照)。
    pub filter_cutoff: u8,
    /// フィルターResonance(0〜255)。
    pub filter_resonance: u8,
    /// フィルタータイプ(0=LP/1=HP/2=BP)。
    pub filter_type: u8,
    /// Self-Oscillation有効フラグ。
    pub filter_self_oscillation: bool,
    /// Filter EG Attack(0〜255)。
    pub filter_eg_attack: u8,
    /// Filter EG Decay(0〜255)。
    pub filter_eg_decay: u8,
    /// Filter EG Sustain(0〜255)。
    pub filter_eg_sustain: u8,
    /// Filter EG Release(0〜255)。
    pub filter_eg_release: u8,
    /// Filter EGがCutoffに与える変調量(0〜255)。
    pub filter_eg_depth: u8,
}

impl Default for ChannelParams {
    /// filter_cutoff=255（フィルター全開）/ filter_self_oscillation=true（spec.md準拠）以外は
    /// すべて0（音色LFO・Filter EGとも無効、アルゴリズム0）。
    fn default() -> Self {
        Self {
            algorithm: 0,
            feedback: 0,
            tone_lfo_freq: 0,
            tone_lfo_pmd: 0,
            tone_lfo_amd: 0,
            tone_lfo_delay: 0,
            pms: 0,
            ams: 0,
            filter_cutoff: 255,
            filter_resonance: 0,
            filter_type: 0,
            filter_self_oscillation: true,
            filter_eg_attack: 0,
            filter_eg_decay: 0,
            filter_eg_sustain: 0,
            filter_eg_release: 0,
            filter_eg_depth: 0,
        }
    }
}

/// 4op分のオペレーターパラメーター + チャンネルパラメーターの一式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ym38x6Patch {
    pub operators: [OperatorParams; 4],
    pub channel: ChannelParams,
}

// ---------------------------------------------------------------------------
// パフォーマンスLFOの適用先（38x6拡張Destination）
// ---------------------------------------------------------------------------

/// パフォーマンスLFOの適用先。共通Destination（Pitch/Volume）に加え、
/// 38x6固有の拡張Destination（TLキャリア一括）を持つ。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Ym38x6LfoDestination {
    #[default]
    Pitch,
    Volume,
    TlCarrier,
}

// ---------------------------------------------------------------------------
// チャンネル（4オペレーター + アルゴリズム結線）
// ---------------------------------------------------------------------------

struct Channel {
    operators: [Operator; 4],
    channel_params: ChannelParams,
    /// フィードバックオペレーターの直前の出力（自己変調に使う）。
    feedback_buffer: f32,
    /// EXPERIMENT(fb-2sample): 2サンプル平均帰還用の、さらに1つ前の出力 out[n-2]。
    feedback_buffer2: f32,
    /// KSR計算用のノート番号（Note-On時の周波数から近似）。
    note: u8,
    /// OP単位キーオン/オフ（CC102〜105）でのリトリガー用に保持するNote-On時の周波数。
    base_frequency: f32,
    /// OP単位キーオン/オフでのリトリガー用に保持するNote-On時のベロシティ。
    velocity: u8,
    perf_lfo: PerformanceLfo,
    lfo_destination: Ym38x6LfoDestination,
    lfo_depth: f32,
    pitch_mod_cents: f32,
    /// MIDIピッチベンド/RPN0によるピッチオフセット（セント、毎tickでpitch_mod_centsに加算）。
    /// パフォーマンスLFO（pitch_mod_cents）が毎tick上書きされるのと独立に保持する。
    bend_cents: f32,
    volume_mod_delta: f32,
    /// 拡張Destination=TlCarrier用：キャリア出力にかかる乗算ゲインのオフセット。
    tl_carrier_mod_delta: f32,
    /// 音色LFO本体（PMS/AMS×PMD/AMD、spec.md「音色LFO」セクション参照）。
    tone_lfo: ToneLfo,
    /// 4op合成後に適用するSVFフィルター（spec.md「フィルター」セクション参照）。
    svf: Svf,
    /// フィルターCutoffを変調するFilter EG。
    filter_eg: FilterEnvelope,
    /// チョークで破棄される直前に仕込む線形フェードアウトの総サンプル数（0=通常状態）。
    declick_total: u32,
    /// デクリックフェードの残りサンプル数。declick_totalから1ずつ減り、0で完全に消える。
    declick_remaining: u32,
}

impl Channel {
    fn new(frequency: f32, velocity: u8, patch: Ym38x6Patch) -> Self {
        let note = frequency_to_note(frequency);
        // アルゴリズムからキャリア/モジュレーターを判定し、各OPに伝える
        // （Velocity Sensitivity=明るさをモジュレーターにのみ効かせるため）。
        let algo = &ALGORITHMS[(patch.channel.algorithm as usize).min(7)];
        let operators = std::array::from_fn(|i| {
            let mut op = Operator::new(patch.operators[i]);
            op.set_carrier(algo.carriers.contains(&i));
            op.note_on(frequency, velocity);
            op
        });
        let mut filter_eg = FilterEnvelope::new();
        filter_eg.note_on();
        Self {
            operators,
            channel_params: patch.channel,
            feedback_buffer: 0.0,
            feedback_buffer2: 0.0,
            note,
            base_frequency: frequency,
            velocity,
            perf_lfo: PerformanceLfo::new(),
            lfo_destination: Ym38x6LfoDestination::default(),
            lfo_depth: 0.0,
            pitch_mod_cents: 0.0,
            bend_cents: 0.0,
            volume_mod_delta: 0.0,
            tl_carrier_mod_delta: 0.0,
            tone_lfo: ToneLfo::new(),
            svf: Svf::new(),
            filter_eg,
            declick_total: 0,
            declick_remaining: 0,
        }
    }

    /// チョークで破棄される直前に呼ぶ。出力を`samples`サンプルかけて線形に0へ落とし、
    /// 旧ボイスが非ゼロ振幅で瞬間消滅することによるクリックノイズを防ぐ。
    fn start_declick(&mut self, samples: u32) {
        self.declick_total = samples.max(1);
        self.declick_remaining = self.declick_total;
    }

    /// 発音/リリース中のチャンネルを、残響レベルを保持したまま再キーオンする
    /// （実機 OPM の Key-On 挙動）。env_level を 0 に落とさず現在値からアタックを
    /// 再開するため、同音連打でもプチノイズが出ず、残響ドラッグが再現される。
    /// 新しいパッチ・周波数・ベロシティは適用し直す。ピッチベンド量は呼び出し側が
    /// 別途設定するため、ここでは触らない（既存値を維持）。
    fn retrigger(&mut self, frequency: f32, velocity: u8, patch: Ym38x6Patch) {
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
        self.filter_eg.note_on();
        // チョーク経由でデクリック中だった場合は解除して通常再生に戻す。
        self.declick_total = 0;
        self.declick_remaining = 0;
    }

    fn set_performance_lfo(
        &mut self,
        rate: u8,
        delay: u8,
        waveform: LfoWaveform,
        destination: Ym38x6LfoDestination,
        depth: f32,
    ) {
        self.perf_lfo.set_rate(rate);
        self.perf_lfo.set_delay(delay);
        self.perf_lfo.set_waveform(waveform);
        self.lfo_destination = destination;
        self.lfo_depth = depth;
    }

    fn note_off(&mut self) {
        for op in self.operators.iter_mut() {
            op.note_off();
        }
        self.filter_eg.note_off();
    }

    /// CC103〜106（≧64）：指定オペレーター(0〜3)をNote-On時の周波数/ベロシティでキーオンする。
    fn note_on_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_on(self.base_frequency, self.velocity);
    }

    /// CC103〜106（<64）：指定オペレーター(0〜3)をキーオフする（全OP独立。Op3も特別扱いしない）。
    fn note_off_operator(&mut self, op_index: usize) {
        self.operators[op_index].note_off();
    }

    fn is_idle(&self) -> bool {
        // デクリックフェード中はランプが終わるまで生存させる（途中で消すと逆にクリックになる）。
        if self.declick_total > 0 {
            return self.declick_remaining == 0;
        }
        self.operators.iter().all(|op| op.is_idle())
    }

    fn tick(&mut self, sample_rate: f32, wave_tables: &[Option<WaveTable>]) -> f32 {
        if self.is_idle() {
            return 0.0;
        }

        // パフォーマンスLFO（ビブラート/トレモロ/TLキャリア一括）
        let lfo_value = self.perf_lfo.tick(sample_rate);
        match self.lfo_destination {
            Ym38x6LfoDestination::Pitch => {
                apply_lfo_modulation(lfo_value, LfoDestination::Pitch, self.lfo_depth, self);
            }
            Ym38x6LfoDestination::Volume => {
                apply_lfo_modulation(lfo_value, LfoDestination::Volume, self.lfo_depth, self);
            }
            Ym38x6LfoDestination::TlCarrier => {
                self.tl_carrier_mod_delta = lfo_value * self.lfo_depth;
            }
        }
        for op in self.operators.iter_mut() {
            op.set_pitch_modulation(self.pitch_mod_cents + self.bend_cents);
        }

        // 音色LFO（プリセット・NRPNで設定する音作り用、PMS/AMS×PMD/AMD）
        let tone_lfo_value = self.tone_lfo.tick(
            sample_rate,
            self.channel_params.tone_lfo_freq,
            self.channel_params.tone_lfo_delay,
        );
        let tone_pitch_mod_cents = tone_lfo_value
            * pms_to_cents_range(self.channel_params.pms)
            * (self.channel_params.tone_lfo_pmd as f32 / 255.0);
        let tone_amp_mod = tone_lfo_value
            * ams_to_depth(self.channel_params.ams)
            * (self.channel_params.tone_lfo_amd as f32 / 255.0);
        for op in self.operators.iter_mut() {
            let am = if op.params.am_enable { tone_amp_mod } else { 0.0 };
            op.set_tone_lfo_modulation(tone_pitch_mod_cents, am);
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
                // EXPERIMENT(fb-2sample): 2サンプル平均帰還の切替（既定は1サンプル）。
                let fb_source = if FEEDBACK_TWO_SAMPLE.load(Ordering::Relaxed) {
                    // 実機OPM/OPN/OPZ準拠: 直近2サンプルの平均で帰還（帰還経路の1次ローパス）
                    0.5 * (self.feedback_buffer + self.feedback_buffer2)
                } else {
                    self.feedback_buffer
                };
                let scale =
                    feedback_to_scale_with_max(self.channel_params.feedback, current_feedback_scale_max());
                modulation += fb_source * scale;
            }
            let wave = wave_table_for(wave_tables, self.operators[op_idx].params.waveform);
            let out = self.operators[op_idx].tick(sample_rate, wave, modulation, self.note);
            op_outputs[op_idx] = out;
            if op_idx == algo.feedback_op {
                self.feedback_buffer2 = self.feedback_buffer;
                self.feedback_buffer = out;
            }
        }

        let carrier_sum: f32 = algo.carriers.iter().map(|&i| op_outputs[i]).sum();
        let tl_carrier_gain = (1.0 + self.tl_carrier_mod_delta).max(0.0);
        let volume_gain = (1.0 + self.volume_mod_delta).max(0.0);
        // ベロシティは音量のみに作用（通常のMIDI楽器と同じ常時ONの挙動）。音色は変えない。
        let velocity_gain = velocity_to_volume_gain(self.velocity);
        let dry = carrier_sum * tl_carrier_gain * volume_gain * velocity_gain;

        // SVFフィルター + Filter EG（4op合成後）
        let filter_eg_level = self.filter_eg.tick(
            sample_rate,
            self.channel_params.filter_eg_attack,
            self.channel_params.filter_eg_decay,
            self.channel_params.filter_eg_sustain,
            self.channel_params.filter_eg_release,
        );
        let cutoff = effective_cutoff(
            self.channel_params.filter_cutoff,
            filter_eg_level,
            self.channel_params.filter_eg_depth,
        );
        let cutoff_hz = cutoff_to_hz(cutoff);
        let filter_type = FilterType::from_u8(self.channel_params.filter_type);
        let out = self.svf.process(
            dry,
            sample_rate,
            cutoff_hz,
            self.channel_params.filter_resonance,
            self.channel_params.filter_self_oscillation,
            filter_type,
        );

        // デクリックフェード：チョークされた旧ボイスを線形に0へ落とす。
        // 最初のサンプルはgain=1.0（直前と連続）、declick_totalサンプルかけて0に向かう。
        if self.declick_total > 0 {
            let gain = self.declick_remaining as f32 / self.declick_total as f32;
            self.declick_remaining = self.declick_remaining.saturating_sub(1);
            out * gain
        } else {
            out
        }
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

/// 指定スロットの波形テーブルを返す。未割り当てスロット（ユーザー波形未設定）の場合は
/// 常に存在するスロット0（サイン波）にフォールバックする。
fn wave_table_for(wave_tables: &[Option<WaveTable>], slot: u8) -> &WaveTable {
    wave_tables[slot as usize]
        .as_ref()
        .unwrap_or_else(|| wave_tables[0].as_ref().unwrap())
}

// ---------------------------------------------------------------------------
// 38x6 エンジン
// ---------------------------------------------------------------------------

const TOTAL_SLOTS: usize = 256;

/// チョークで破棄されるボイスを線形フェードアウトさせる時間（秒）。クリック緩和用。
/// 新ボイスの発音タイミングは遅らせず、消えゆく旧ボイスにのみ適用するためレイテンシーは増えない。
const DECLICK_SECONDS: f32 = 0.004;

pub struct Ym38x6Engine {
    sample_rate: f32,
    channels: HashMap<usize, Channel>,
    /// チョークされ、デクリックフェード中の旧ボイス。フェード完了で破棄される。
    fading_voices: Vec<Channel>,
    wave_tables: Vec<Option<WaveTable>>,
    current_patch: Ym38x6Patch,
}

impl Ym38x6Engine {
    pub fn new(sample_rate: f32) -> Self {
        let mut wave_tables: Vec<Option<WaveTable>> = (0..TOTAL_SLOTS).map(|_| None).collect();
        for i in 0..waveform::BUILTIN_WAVEFORM_COUNT {
            wave_tables[i as usize] = Some(gen_builtin_waveform(i));
        }
        Self {
            sample_rate,
            channels: HashMap::new(),
            fading_voices: Vec::new(),
            wave_tables,
            current_patch: Ym38x6Patch::default(),
        }
    }

    /// 以降の`SoundEngine::note_on`はこのパッチで発音する。
    pub fn set_patch(&mut self, patch: Ym38x6Patch) {
        self.current_patch = patch;
    }

    /// 指定チャンネルIDへベロシティ・パッチを明示指定してNote-Onする（VST/Tauriから使用）。
    /// 既に同じIDで発音中/リリース中のチャンネルがあれば、env_levelを保持したまま
    /// 残響から再アタックする（実機OPMのKey-On挙動。プチノイズが出ず、モジュレーターの
    /// 残響ドラッグによるFMらしい明るさも再現される）。
    pub fn note_on_with_velocity(&mut self, channel: usize, frequency: f32, velocity: u8, patch: Ym38x6Patch) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            if !ch.is_idle() {
                ch.retrigger(frequency, velocity, patch);
                return;
            }
        }
        self.channels.insert(channel, Channel::new(frequency, velocity, patch));
    }

    /// 同一IDの旧ボイスをデクリックフェードでチョーク（数msで消音）してから新ボイスを発音する。
    /// 残響再アタックではなく明示的に切り替えたい場合に使う（サステインペダルのスウェル等、
    /// 将来のボイス管理用）。`note_on_with_velocity`がデフォルトの残響再アタック。
    pub fn choke_and_note_on(&mut self, channel: usize, frequency: f32, velocity: u8, patch: Ym38x6Patch) {
        if let Some(mut old) = self.channels.remove(&channel) {
            if !old.is_idle() {
                old.start_declick((self.sample_rate * DECLICK_SECONDS) as u32);
                self.fading_voices.push(old);
            }
        }
        self.channels.insert(channel, Channel::new(frequency, velocity, patch));
    }

    /// 発音中チャンネルのピッチベンド量（セント）を設定する（MIDI Pitch Bend / RPN0）。
    pub fn set_pitch_bend(&mut self, channel: usize, cents: f32) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.bend_cents = cents;
        }
    }

    /// 指定グループ（`channel >> 7`が一致する全チャンネル）のピッチベンド量を一括設定する。
    /// VST側のID符号化 `midi_ch*128 + note` において、MIDIチャンネル単位でベンドを
    /// かけるために使う（和音の全ノートが一緒に滑らかに上下する）。
    pub fn set_pitch_bend_group(&mut self, group: usize, cents: f32) {
        for (id, ch) in self.channels.iter_mut() {
            if id >> 7 == group {
                ch.bend_cents = cents;
            }
        }
    }

    /// 発音中チャンネルのチャンネルパラメーターを更新する（DAWオートメーション/NRPN用）。
    pub fn set_channel_params(&mut self, channel: usize, params: ChannelParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.channel_params = params;
        }
    }

    /// 発音中チャンネルの指定オペレーターのパラメーターを更新する。
    pub fn set_operator_params(&mut self, channel: usize, op_index: usize, params: OperatorParams) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].params = params;
        }
    }

    /// 発音中チャンネルの指定オペレーターのF-Numberを上書きする（NRPN Operator F-Number、0〜8191）。
    pub fn set_operator_f_number(&mut self, channel: usize, op_index: usize, f_number: u16) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.operators[op_index].set_f_number_override(f_number);
        }
    }

    /// CC103〜106（≧64）：指定チャンネルの指定オペレーター(0〜3)をキーオンする。
    pub fn note_on_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_on_operator(op_index);
        }
    }

    /// CC103〜106（<64）：指定チャンネルの指定オペレーター(0〜3)をキーオフする（全OP独立）。
    pub fn note_off_operator(&mut self, channel: usize, op_index: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off_operator(op_index);
        }
    }

    /// スロット8〜255にユーザー定義波形をロードする。
    pub fn set_user_wave(&mut self, slot: u8, input: &[i8; 32]) {
        assert!(slot >= 8, "slots 0-7 are reserved for builtin waves");
        self.wave_tables[slot as usize] = Some(convert_wave_32(input));
    }

    /// 指定チャンネルのパフォーマンスLFO（Rate/Delay/Waveform/Destination/Depth）を設定する。
    pub fn set_performance_lfo(
        &mut self,
        channel: usize,
        rate: u8,
        delay: u8,
        waveform: LfoWaveform,
        destination: Ym38x6LfoDestination,
        depth: f32,
    ) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.set_performance_lfo(rate, delay, waveform, destination, depth);
        }
    }
}

impl SoundEngine for Ym38x6Engine {
    /// wave_slot/adsrはトレイト互換のため残すが未使用。velocity=127固定でカレントパッチを使う。
    fn note_on(&mut self, channel: usize, _wave_slot: u8, frequency: f32, _adsr: AdsrParams) {
        let patch = self.current_patch;
        self.note_on_with_velocity(channel, frequency, 127, patch);
    }

    fn note_off(&mut self, channel: usize) {
        if let Some(ch) = self.channels.get_mut(&channel) {
            ch.note_off();
        }
    }

    fn render(&mut self, output: &mut [f32], num_channels: usize) {
        let num_channels = num_channels.max(1);
        let sample_rate = self.sample_rate;
        let wave_tables = &self.wave_tables;
        let fading_voices = &mut self.fading_voices;
        for frame in output.chunks_mut(num_channels) {
            let mut mix = 0.0f32;
            for ch in self.channels.values_mut() {
                if ch.is_idle() {
                    continue;
                }
                mix += ch.tick(sample_rate, wave_tables);
            }
            // チョークされフェードアウト中の旧ボイスも一緒にミックスする
            for ch in fading_voices.iter_mut() {
                if ch.is_idle() {
                    continue;
                }
                mix += ch.tick(sample_rate, wave_tables);
            }
            for s in frame.iter_mut() {
                *s += mix;
            }
        }
        self.channels.retain(|_, ch| !ch.is_idle());
        self.fading_voices.retain(|ch| !ch.is_idle());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 全Opがアルゴリズム7（全並列）で即音量最大・サスティン無限のテスト用パッチ。
    fn loud_patch(velocity_sensitivity: u8) -> Ym38x6Patch {
        let op_params = OperatorParams {
            tl: 255,
            ar: 255,
            d1r: 0,
            d2r: 0,
            d1l: 255,
            rr: 255,
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity,
            waveform: 0,
            op_fine_tune: 128,
        };
        let mut patch = Ym38x6Patch::default();
        patch.operators = [op_params; 4];
        patch.channel.algorithm = 7;
        patch
    }

    #[test]
    fn note_on_produces_non_silent_output() {
        let mut engine = Ym38x6Engine::new(44100.0);
        engine.set_patch(loud_patch(0));
        engine.note_on(0, 0, 440.0, AdsrParams::default());

        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        assert!(buf.iter().any(|&s| s != 0.0), "expected non-silent output");
    }

    #[test]
    fn note_off_operator_silences_single_op_and_note_on_operator_retriggers() {
        let mut engine = Ym38x6Engine::new(44100.0);
        let ch = 0;
        engine.note_on_with_velocity(ch, 440.0, 127, loud_patch(0));

        engine.note_off_operator(ch, 0);
        // チャンネル全体は他のOpが鳴り続けるためidleにならない
        assert!(!engine.channels[&ch].is_idle());

        // rr=255（最速リリース）でOp0が即座にidleになる
        let mut buf = vec![0.0f32; 100];
        engine.render(&mut buf, 1);
        assert!(engine.channels[&ch].operators[0].is_idle());

        // Op0を再キーオン → idleではなくなる
        engine.note_on_operator(ch, 0);
        assert!(!engine.channels[&ch].operators[0].is_idle());
    }

    #[test]
    fn note_off_operator_on_op3_is_independent_like_other_ops() {
        let mut engine = Ym38x6Engine::new(44100.0);
        let ch = 0;
        engine.note_on_with_velocity(ch, 440.0, 127, loud_patch(0));

        engine.note_off_operator(ch, 3);

        // Op3は他Opと同じ独立扱い：チャンネルは消えず、他Opは鳴り続ける（Op3マスター廃止）
        assert!(engine.channels.contains_key(&ch), "Op3 key-off must not remove the channel");
        assert!(!engine.channels[&ch].is_idle());

        // rr=255（最速リリース）でOp3のみ即idle、他Opは生存
        let mut buf = vec![0.0f32; 100];
        engine.render(&mut buf, 1);
        assert!(engine.channels[&ch].operators[3].is_idle());
        assert!(!engine.channels[&ch].operators[0].is_idle());
    }

    #[test]
    fn set_operator_f_number_overrides_single_operator_frequency() {
        let mut engine = Ym38x6Engine::new(44100.0);
        let ch = 0;
        engine.note_on_with_velocity(ch, 440.0, 127, loud_patch(0));

        engine.set_operator_f_number(ch, 0, crate::mapping::F_NUMBER_CENTER / 2);

        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        assert!(buf.iter().all(|&s| s.is_finite()), "expected finite output");
        assert!(buf.iter().any(|&s| s != 0.0), "expected non-silent output");
    }

    #[test]
    fn velocity_changes_output_volume_regardless_of_sensitivity() {
        // Velocity Sensitivity=0（明るさ無効）でも、ベロシティは音量に常時作用する。
        let mut patch = loud_patch(0);
        for op in patch.operators.iter_mut() {
            op.tl = 200;
        }

        let mut engine_lo = Ym38x6Engine::new(44100.0);
        let mut engine_hi = Ym38x6Engine::new(44100.0);
        engine_lo.note_on_with_velocity(0, 440.0, 40, patch);
        engine_hi.note_on_with_velocity(0, 440.0, 120, patch);

        let mut buf_lo = vec![0.0f32; 100];
        let mut buf_hi = vec![0.0f32; 100];
        engine_lo.render(&mut buf_lo, 1);
        engine_hi.render(&mut buf_hi, 1);

        let peak_lo = buf_lo.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak_hi = buf_hi.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(peak_hi > peak_lo, "higher velocity should be louder: {peak_hi} vs {peak_lo}");
    }

    #[test]
    fn velocity_sensitivity_ignored_on_carriers() {
        // 全並列(algo7)＝全OPキャリア。Velocity Sensitivityは明るさ専用なので、
        // 同一ベロシティならvel_sensの値に関わらずキャリアの音量は変わらない。
        let mut patch_zero = loud_patch(0);
        let mut patch_max = loud_patch(255);
        for op in patch_zero.operators.iter_mut() {
            op.tl = 200;
        }
        for op in patch_max.operators.iter_mut() {
            op.tl = 200;
        }

        let mut engine_zero = Ym38x6Engine::new(44100.0);
        let mut engine_max = Ym38x6Engine::new(44100.0);
        engine_zero.note_on_with_velocity(0, 440.0, 100, patch_zero);
        engine_max.note_on_with_velocity(0, 440.0, 100, patch_max);

        let mut buf_zero = vec![0.0f32; 100];
        let mut buf_max = vec![0.0f32; 100];
        engine_zero.render(&mut buf_zero, 1);
        engine_max.render(&mut buf_max, 1);

        let peak_zero = buf_zero.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak_max = buf_max.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            (peak_zero - peak_max).abs() < 1e-6,
            "velocity sensitivity must not change carrier volume: {peak_zero} vs {peak_max}"
        );
    }

    #[test]
    fn velocity_sensitivity_affects_modulator_brightness() {
        // algo0: O1->O2->O3->O4。O1はモジュレーター。
        // モジュレーターのVelocity Sensitivityは変調量（明るさ）を変えるので、
        // 同一ベロシティでもvel_sensの有無で出力波形が変わる。
        let base = OperatorParams {
            tl: 100,
            ar: 255,
            d1r: 0,
            d2r: 0,
            d1l: 255,
            rr: 255,
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
        };
        let make = |op0_sens: u8| {
            let mut patch = Ym38x6Patch::default();
            patch.operators = [base; 4];
            patch.operators[0].velocity_sensitivity = op0_sens;
            patch.channel.algorithm = 0;
            patch.channel.feedback = 0;
            patch
        };

        let mut engine_flat = Ym38x6Engine::new(44100.0);
        let mut engine_bright = Ym38x6Engine::new(44100.0);
        engine_flat.note_on_with_velocity(0, 440.0, 100, make(0));
        engine_bright.note_on_with_velocity(0, 440.0, 100, make(255));

        let mut buf_flat = vec![0.0f32; 512];
        let mut buf_bright = vec![0.0f32; 512];
        engine_flat.render(&mut buf_flat, 1);
        engine_bright.render(&mut buf_bright, 1);

        let differs = buf_flat
            .iter()
            .zip(buf_bright.iter())
            .any(|(a, b)| (a - b).abs() > 1e-6);
        assert!(differs, "modulator velocity sensitivity should change the timbre");
    }

    #[test]
    fn all_algorithms_long_run_no_nan() {
        let op_params = OperatorParams {
            tl: 200,
            ar: 255,
            d1r: 100,
            d2r: 80,
            d1l: 180,
            rr: 150,
            mul: 1,
            dt1: 128,
            ksr: 64,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
        };

        for algorithm in 0u8..8 {
            let mut patch = Ym38x6Patch::default();
            patch.operators = [op_params; 4];
            patch.channel.algorithm = algorithm;
            patch.channel.feedback = 128;

            let mut engine = Ym38x6Engine::new(44100.0);
            let ch = 0;
            engine.note_on_with_velocity(ch, 440.0, 100, patch);

            let mut buf = vec![0.0f32; 44100];
            engine.render(&mut buf, 1);
            engine.note_off(ch);

            let mut buf2 = vec![0.0f32; 44100 * 2];
            engine.render(&mut buf2, 1);

            for &s in buf.iter().chain(buf2.iter()) {
                assert!(s.is_finite(), "algorithm {algorithm}: non-finite sample {s}");
            }
        }
    }

    #[test]
    fn filter_self_oscillation_long_run_no_nan() {
        let op_params = OperatorParams {
            tl: 200,
            ar: 255,
            d1r: 100,
            d2r: 80,
            d1l: 180,
            rr: 150,
            mul: 1,
            dt1: 128,
            ksr: 64,
            am_enable: false,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
        };

        for filter_type in 0u8..3 {
            let mut patch = Ym38x6Patch::default();
            patch.operators = [op_params; 4];
            patch.channel.algorithm = 7;
            patch.channel.filter_type = filter_type;
            patch.channel.filter_cutoff = 64;
            patch.channel.filter_resonance = 255;
            patch.channel.filter_self_oscillation = true;
            patch.channel.filter_eg_attack = 200;
            patch.channel.filter_eg_decay = 150;
            patch.channel.filter_eg_sustain = 128;
            patch.channel.filter_eg_release = 150;
            patch.channel.filter_eg_depth = 255;

            let mut engine = Ym38x6Engine::new(44100.0);
            let ch = 0;
            engine.note_on_with_velocity(ch, 440.0, 100, patch);

            let mut buf = vec![0.0f32; 44100];
            engine.render(&mut buf, 1);
            engine.note_off(ch);

            let mut buf2 = vec![0.0f32; 44100 * 2];
            engine.render(&mut buf2, 1);

            for &s in buf.iter().chain(buf2.iter()) {
                assert!(s.is_finite(), "filter_type {filter_type}: non-finite sample {s}");
            }
        }
    }

    #[test]
    fn tone_lfo_modulates_output_amplitude_periodically() {
        let op_params = OperatorParams {
            tl: 255,
            ar: 255,
            d1r: 0,
            d2r: 0,
            d1l: 255,
            rr: 255,
            mul: 1,
            dt1: 128,
            ksr: 0,
            am_enable: true,
            velocity_sensitivity: 0,
            waveform: 0,
            op_fine_tune: 128,
        };
        let mut patch = Ym38x6Patch::default();
        patch.operators = [op_params; 4];
        patch.channel.algorithm = 7; // 全並列
        patch.channel.tone_lfo_freq = 200; // 速めのLFO（テストを短時間で完結させる）
        patch.channel.tone_lfo_pmd = 255;
        patch.channel.tone_lfo_amd = 255;
        patch.channel.pms = 255;
        patch.channel.ams = 255;

        let mut engine = Ym38x6Engine::new(44100.0);
        engine.note_on_with_velocity(0, 440.0, 127, patch);

        let mut buf = vec![0.0f32; 4410]; // 0.1秒（音色LFO数周期分）
        engine.render(&mut buf, 1);

        // ウィンドウごとの最大振幅を比較し、音色LFOのAMにより振幅が周期的に変化することを確認
        let window = 200;
        let peaks: Vec<f32> = buf
            .chunks(window)
            .map(|chunk| chunk.iter().fold(0.0f32, |a, &b| a.max(b.abs())))
            .collect();

        let max_peak = peaks.iter().cloned().fold(0.0f32, f32::max);
        let min_peak = peaks.iter().cloned().fold(f32::MAX, f32::min);

        assert!(max_peak > 0.5, "expected a loud window: max_peak={max_peak}");
        assert!(min_peak < max_peak * 0.6, "expected amplitude to vary with LFO: min={min_peak} max={max_peak}");
    }
}
