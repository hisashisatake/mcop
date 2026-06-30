mod editor;
mod midi;
mod param_adapter;
mod params;

use midi::{apply_at_modulation, cc_to_u8, cc_to_u7, AtDestination, RpnSelection};
use params::{
    Ym38x6Params, DEFAULT_ALGORITHM, DEFAULT_CHORUS_FEEDBACK, DEFAULT_CHORUS_MOD_DEPTH,
    DEFAULT_CHORUS_MOD_RATE, DEFAULT_CHORUS_SEND_TO_REVERB, DEFAULT_CHORUS_TYPE,
    DEFAULT_REVERB_TIME, DEFAULT_REVERB_TYPE,
};

use nice_plug::prelude::*;
use nice_plug_egui::EguiState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ym38x6_core::mapping::F_NUMBER_CENTER;
use ym38x6_core::{
    pitch_depth_cents, presets_dir, volume_depth, ChannelParams, ChorusType, LfoWaveform,
    MasterEffects, OperatorParams, PresetBank, ReverbType, SoundEngine, Ym38x6Engine,
    Ym38x6LfoDestination, Ym38x6Patch,
};

/// MIDIノート番号の総数（0〜127）。MIDIノート番号をそのままチャンネルIDとして使うため
/// （1ノート=1チャンネル）、発音中チャンネルを走査するループの上限に使う。
/// 将来MIDI規格でノート番号空間が拡張された場合はここだけ変えればよい。
const MIDI_NOTE_COUNT: u8 = 128;

struct Ym38x6Plugin {
    params: Arc<Ym38x6Params>,
    engine: Ym38x6Engine,
    effects: MasterEffects,
    render_buffer: Vec<f32>, // プロセスコールバック用インターリーブ作業バッファ
    sample_rate: f32,
    // Algorithm：NRPN(0,9)に加えてnice-plugのチャンネル単位パラメーターとしても公開する
    // （last_algorithmで差分検知、process()参照）。
    algorithm: u8,
    // NRPN専用パラメーター（DAWオートメーション非公開）。
    filter_type: u8,
    filter_self_oscillation: bool,
    // operator_waveformsはDAWパラメーター（params.operators[i].waveform）との二重管理。
    // DAWオートメーション変化時はprocess()内の差分検知で上書き、NRPN(0,10)〜(0,13)は直接書き込む。
    operator_waveforms: [u8; 4],
    last_operator_waveforms: [u8; 4],

    // パフォーマンスLFO（CC1/76/77/78・RPN0,5・NRPN(0,0)/(0,1)）の状態
    lfo_cc1: u8,    // CC1 Modulation Wheel（Depth加算分）
    lfo_rpn0_5: u8, // RPN0,5 Modulation Depth Range（GM2準拠 0〜127、デフォルト64）
    lfo_destination: Ym38x6LfoDestination, // NRPN(0,0)
    lfo_waveform: LfoWaveform,             // NRPN(0,1)
    // lfo_rate/lfo_depth/lfo_delayはDAWパラメーターとCC76/77/78の両方から設定され得るため、
    // 2シャドウ方式で管理する（last_*_param: DAW側差分検知用、effective_*:
    // apply_performance_lfoへ実際に渡す値。CC受信時はlast_*_paramを更新せず
    // effective_*のみ書き換えることで、次ブロックのDAW差分検知に上書きされないようにする）。
    last_lfo_rate_param: u8,
    effective_lfo_rate: u8,
    last_lfo_depth_param: u8,
    effective_lfo_depth: u8,
    last_lfo_delay_param: u8,
    effective_lfo_delay: u8,

    // Reverb/Chorus Send：DAWパラメーターとCC91/93の両方から設定され得るため、
    // マスターエフェクト5パラメーターと同じ1シャドウ差分検知方式で管理する。
    last_rev_send: u8,
    last_cho_send: u8,

    // RPN/NRPN選択状態
    rpn_msb: u8,
    rpn_lsb: u8,
    nrpn_msb: u8,
    nrpn_lsb: u8,
    rpn_selection: RpnSelection,

    // Algorithmの「前回ブロックで適用したnice-plug値」（1シャドウ差分検知方式、下記マスター5パラメーターと同型）
    last_algorithm: u8,

    // マスター単位パラメーターの「前回ブロックで適用したnice-plug値」（1シャドウ差分検知方式）。
    // Reverb/Chorus TypeはNRPN(0,2)/(0,3)からも直接effectsへ書き込まれるため、
    // DAW値が変化していない間はNRPN側の設定が上書きされない（last_reverb_time等と同型）。
    last_reverb_type: u8,
    last_reverb_time: u8,
    last_chorus_type: u8,
    last_chorus_mod_rate: u8,
    last_chorus_mod_depth: u8,
    last_chorus_feedback: u8,
    last_chorus_send_to_reverb: u8,

    // AT/Poly AT Destination（NRPN(0,16)/(0,17)）と、加算対象のプレッシャー値
    at_destination: AtDestination,
    poly_at_destination: AtDestination,
    channel_pressure: u8,
    poly_pressure: HashMap<u8, u8>, // MIDIノート番号 → Poly Key Pressure

    // NRPN(0,18)〜(0,21): Operator F-Number Op0〜3（CC6+CC38の14bit値→13bit(0〜8191)にclamp）
    data_entry_msb: u8,                     // CC6 (Data Entry MSB) の最新値
    data_entry_lsb: u8,                     // CC38 (Data Entry LSB) の最新値
    operator_f_number_override: [u16; 4],   // 各Opの上書き値。初期値F_NUMBER_CENTER（上書きなし）

    // Bank Select（CC0=MSB, CC32=LSB）+ Program Change：MIDIチャンネルごとに管理。
    // CC0/CC32/CC102/Program Changeはすべて論理MIDIチャンネル単位で作用する。
    bank_select_msb: [u8; 16],           // CC0 per MIDI ch
    bank_select_lsb: [u8; 16],           // CC32 per MIDI ch
    // Program Change（CC102/CLAP MidiProgramChange）で選択されたパッチ（MIDIチャンネルごと）。
    // GUIでプリセットを選択した際に全チャンネルをNoneへ戻す（pending_gui_presetハンドラ参照）。
    program_patch: [Option<Ym38x6Patch>; 16],

    // ピッチベンド（MIDIチャンネル単位）。エンジンのボイスIDは midi_ch*128+note で符号化し、
    // ベンドは set_pitch_bend_group で同一MIDIチャンネルの全ノートへ一括適用する
    // （和音が一緒に滑らかに上下する。VGM/OPMのソフトビブラート＝チャンネル全体のピッチ移動に一致）。
    channel_bend_cents: [f32; 16], // 各MIDIチャンネルの現在のベンド量（セント）
    // ピッチベンド感度（半音）。RPN(0,0)で設定。デフォルト±2半音。
    // 全MIDIチャンネル共通。vgm2x6 SMFは全chに同値を送る。
    pitch_bend_range: f32,

    // CC7/CC11 チャンネル音量（GM2準拠）。
    // 実効ゲイン = (cc7/127)^2 × (cc11/127)^2（GM2の40·log10カーブ ⇔ 二乗）。
    // note-on 毎に新ボイスへ set_channel_volume で再適用する。
    cc7:  [u8; 16], // Channel Volume（既定127=フル）
    cc11: [u8; 16], // Expression（既定127=フル）

    // presets_dir()から読み込んだユーザープリセット集合（initialize()で読み込む）
    preset_bank: PresetBank,

    // GUIプリセット選択時のNRPN専用パラメーター転送用（egui→processスレッド）。
    // DAW公開パラメーターはParamSetterで書き戻し済みのため、ここではfilter_type/
    // filter_self_oscillationの2種のみ運ぶ（waveformはDAWパラメーターなのでParamSetter経由）。
    pending_gui_preset: Arc<Mutex<Option<Ym38x6Patch>>>,

    // GUIエディターのウィンドウサイズ状態（editor()で使い回す）
    egui_state: Arc<EguiState>,
}

/// CC7(Channel Volume) と CC11(Expression) の値（0〜127）から GM2 準拠のゲインを計算する。
/// GM2 の実効音量カーブは 40·log10(cc/127) dB ⇔ リニアゲイン = (cc/127)^2。
/// CC7 と CC11 はそれぞれ二乗して積を取る。
#[inline]
fn channel_gain(cc7: u8, cc11: u8) -> f32 {
    let v7  = cc7  as f32 / 127.0;
    let v11 = cc11 as f32 / 127.0;
    v7 * v7 * v11 * v11
}

/// MIDIチャンネル(0〜15)とノート番号(0〜127)からエンジンのボイスIDを符号化する。
/// `midi_ch*128 + note`。一意性（Note Off・同音再アタックの突き合わせ）と、
/// グループ性（`id >> 7` でMIDIチャンネルを復元してベンド一括適用）を両立する。
#[inline]
fn midi_channel_note_id(channel: u8, note: u8) -> usize {
    (channel as usize) * 128 + note as usize
}

impl Default for Ym38x6Plugin {
    fn default() -> Self {
        const DEFAULT_SR: f32 = 44100.0;
        Self {
            params: Arc::new(Ym38x6Params::default()),
            engine: Ym38x6Engine::new(DEFAULT_SR),
            effects: MasterEffects::new(DEFAULT_SR),
            render_buffer: Vec::new(),
            sample_rate: DEFAULT_SR,
            algorithm: DEFAULT_ALGORITHM,
            filter_type: 0,
            filter_self_oscillation: true,
            operator_waveforms: [0; 4],
            last_operator_waveforms: [0; 4],
            lfo_cc1: 0,
            lfo_rpn0_5: 64,
            lfo_destination: Ym38x6LfoDestination::Pitch,
            lfo_waveform: LfoWaveform::Triangle,
            last_lfo_rate_param: 0,
            effective_lfo_rate: 0,
            last_lfo_depth_param: 0,
            effective_lfo_depth: 0,
            last_lfo_delay_param: 0,
            effective_lfo_delay: 0,
            last_rev_send: 0,
            last_cho_send: 0,
            rpn_msb: 0,
            rpn_lsb: 0,
            nrpn_msb: 0,
            nrpn_lsb: 0,
            rpn_selection: RpnSelection::default(),
            last_algorithm: DEFAULT_ALGORITHM,
            last_reverb_type: DEFAULT_REVERB_TYPE,
            last_reverb_time: DEFAULT_REVERB_TIME,
            last_chorus_type: DEFAULT_CHORUS_TYPE,
            last_chorus_mod_rate: DEFAULT_CHORUS_MOD_RATE,
            last_chorus_mod_depth: DEFAULT_CHORUS_MOD_DEPTH,
            last_chorus_feedback: DEFAULT_CHORUS_FEEDBACK,
            last_chorus_send_to_reverb: DEFAULT_CHORUS_SEND_TO_REVERB,
            at_destination: AtDestination::default(),
            poly_at_destination: AtDestination::default(),
            channel_pressure: 0,
            poly_pressure: HashMap::new(),
            data_entry_msb: 0,
            data_entry_lsb: 0,
            operator_f_number_override: [F_NUMBER_CENTER; 4],
            bank_select_msb: [0; 16],
            bank_select_lsb: [0; 16],
            program_patch: [None; 16],
            channel_bend_cents: [0.0; 16],
            pitch_bend_range: 2.0,
            cc7:  [127; 16],
            cc11: [127; 16],
            preset_bank: PresetBank::default(),
            pending_gui_preset: Arc::new(Mutex::new(None)),
            egui_state: EguiState::from_size(1200, 680),
        }
    }
}

impl Ym38x6Plugin {
    /// 現在のDAWパラメーターとNRPN専用状態から`Ym38x6Patch`を構築する。
    fn build_patch(&self) -> Ym38x6Patch {
        let p = &self.params;
        let operators = std::array::from_fn(|i| {
            let op = &p.operators[i];
            OperatorParams {
                tl: op.tl.value() as u8,
                ar: op.ar.value() as u8,
                d1r: op.d1r.value() as u8,
                d2r: op.d2r.value() as u8,
                d1l: op.d1l.value() as u8,
                rr: op.rr.value() as u8,
                mul: op.mul.value() as u8,
                dt1: op.dt1.value() as u8,
                ksr: op.ksr.value() as u8,
                am_enable: op.ame.value(),
                velocity_sensitivity: op.vel_sens.value() as u8,
                waveform: self.operator_waveforms[i],
                op_fine_tune: op.op_fine_tune.value() as u8,
            }
        });

        let channel = ChannelParams {
            algorithm: self.algorithm,
            feedback: p.feedback.value() as u8,
            tone_lfo_freq: p.tone_freq.value() as u8,
            tone_lfo_pmd: p.tone_pmd.value() as u8,
            tone_lfo_amd: p.tone_amd.value() as u8,
            tone_lfo_delay: p.tone_delay.value() as u8,
            pms: p.pms.value() as u8,
            ams: p.ams.value() as u8,
            filter_cutoff: p.cutoff.value() as u8,
            filter_resonance: p.resonance.value() as u8,
            filter_type: self.filter_type,
            filter_self_oscillation: self.filter_self_oscillation,
            filter_eg_attack: p.feg_a.value() as u8,
            filter_eg_decay: p.feg_d.value() as u8,
            filter_eg_sustain: p.feg_s.value() as u8,
            filter_eg_release: p.feg_r.value() as u8,
            filter_eg_depth: p.feg_depth.value() as u8,
        };

        Ym38x6Patch { operators, channel }
    }

    /// 指定チャンネルへ現在のパフォーマンスLFO設定を適用する
    fn apply_performance_lfo(&mut self, channel: usize) {
        let depth = match self.lfo_destination {
            Ym38x6LfoDestination::Pitch => {
                pitch_depth_cents(self.effective_lfo_depth, self.lfo_cc1, self.lfo_rpn0_5)
            }
            Ym38x6LfoDestination::Volume | Ym38x6LfoDestination::TlCarrier => {
                volume_depth(self.effective_lfo_depth, self.lfo_cc1)
            }
        };
        self.engine.set_performance_lfo(
            channel,
            self.effective_lfo_rate,
            self.effective_lfo_delay,
            self.lfo_waveform,
            self.lfo_destination,
            depth,
        );
    }

    /// 発音中の全チャンネルへ現在のパフォーマンスLFO設定を再適用する
    fn apply_performance_lfo_to_active(&mut self) {
        for note in 0u8..MIDI_NOTE_COUNT {
            self.apply_performance_lfo(note as usize);
        }
    }

    /// NRPN(0,18)〜(0,21)：CC6(Data Entry MSB)+CC38(Data Entry LSB)の14bit値を
    /// 13bit(0〜8191)にclampし、Operator F-Numberとして発音中の全チャンネルへ適用する。
    fn apply_operator_f_number_override(&mut self, op_index: usize) {
        let combined = (self.data_entry_msb as u16) * 128 + self.data_entry_lsb as u16;
        let f_number = combined.min(8191);
        self.operator_f_number_override[op_index] = f_number;
        for note in 0u8..MIDI_NOTE_COUNT {
            self.engine.set_operator_f_number(note as usize, op_index, f_number);
        }
    }

    /// CC99/98(NRPN)・CC101/100(RPN)受信時に選択状態を更新する。
    /// MSB,LSB=127,127（Null）の場合は選択解除する
    fn update_rpn_selection(&mut self, is_nrpn: bool) {
        let (msb, lsb) = if is_nrpn { (self.nrpn_msb, self.nrpn_lsb) } else { (self.rpn_msb, self.rpn_lsb) };
        self.rpn_selection = if msb == 127 && lsb == 127 {
            RpnSelection::None
        } else if is_nrpn {
            RpnSelection::Nrpn(msb, lsb)
        } else {
            RpnSelection::Rpn(msb, lsb)
        };
    }

    /// CC6(Data Entry MSB)受信時、選択中のRPN/NRPNに応じて値を適用する。
    /// `value`はCC値の正規化値（0.0〜1.0）。enum系パラメーターは`cc_to_u7`、
    /// 0〜255連続値パラメーターは`cc_to_u8`で変換する
    fn handle_data_entry(&mut self, value: f32) {
        self.data_entry_msb = cc_to_u7(value);
        match self.rpn_selection {
            // RPN(0,0): Pitch Bend Sensitivity（半音）。CC6の生値(0〜127)を半音数とする。
            RpnSelection::Rpn(0, 0) => {
                self.pitch_bend_range = cc_to_u7(value) as f32;
            }
            // RPN0,5: Modulation Depth Range
            RpnSelection::Rpn(0, 5) => {
                self.lfo_rpn0_5 = cc_to_u7(value);
                self.apply_performance_lfo_to_active();
            }
            // NRPN(0,0): Performance LFO Destination（38x6拡張：2=TLキャリア一括）
            RpnSelection::Nrpn(0, 0) => {
                self.lfo_destination = match cc_to_u7(value) {
                    0 => Ym38x6LfoDestination::Pitch,
                    1 => Ym38x6LfoDestination::Volume,
                    _ => Ym38x6LfoDestination::TlCarrier,
                };
                self.apply_performance_lfo_to_active();
            }
            // NRPN(0,1): Performance LFO Waveform
            RpnSelection::Nrpn(0, 1) => {
                self.lfo_waveform = match cc_to_u7(value) {
                    1 => LfoWaveform::Sine,
                    2 => LfoWaveform::Square,
                    3 => LfoWaveform::SampleHold,
                    _ => LfoWaveform::Triangle,
                };
                self.apply_performance_lfo_to_active();
            }
            // NRPN(0,2): Reverb Type
            RpnSelection::Nrpn(0, 2) => {
                self.effects.set_reverb_type(ReverbType::from_u8(cc_to_u7(value)));
            }
            // NRPN(0,3): Chorus Type
            RpnSelection::Nrpn(0, 3) => {
                self.effects.set_chorus_type(ChorusType::from_u8(cc_to_u7(value)));
            }
            // NRPN(0,4): Reverb Time
            RpnSelection::Nrpn(0, 4) => {
                self.effects.set_reverb_time(cc_to_u8(value));
            }
            // NRPN(0,5): Chorus Mod Rate
            RpnSelection::Nrpn(0, 5) => {
                self.effects.set_chorus_mod_rate(cc_to_u8(value));
            }
            // NRPN(0,6): Chorus Mod Depth
            RpnSelection::Nrpn(0, 6) => {
                self.effects.set_chorus_mod_depth(cc_to_u8(value));
            }
            // NRPN(0,7): Chorus Feedback
            RpnSelection::Nrpn(0, 7) => {
                self.effects.set_chorus_feedback(cc_to_u8(value));
            }
            // NRPN(0,8): Chorus Send To Reverb
            RpnSelection::Nrpn(0, 8) => {
                self.effects.set_chorus_send_to_reverb(cc_to_u8(value));
            }
            // NRPN(0,9): Algorithm（0〜7、範囲外は7にclamp）
            RpnSelection::Nrpn(0, 9) => {
                self.algorithm = cc_to_u7(value).min(7);
            }
            // NRPN(0,10)〜(0,13): Waveform Op0〜3（0〜255）
            RpnSelection::Nrpn(0, 10) => {
                self.operator_waveforms[0] = cc_to_u8(value);
            }
            RpnSelection::Nrpn(0, 11) => {
                self.operator_waveforms[1] = cc_to_u8(value);
            }
            RpnSelection::Nrpn(0, 12) => {
                self.operator_waveforms[2] = cc_to_u8(value);
            }
            RpnSelection::Nrpn(0, 13) => {
                self.operator_waveforms[3] = cc_to_u8(value);
            }
            // NRPN(0,14): Filter Type（0=LP/1=HP/2=BP、範囲外は2にclamp）
            RpnSelection::Nrpn(0, 14) => {
                self.filter_type = cc_to_u7(value).min(2);
            }
            // NRPN(0,15): Filter Self-Oscillation（0=OFF/1=ON）
            RpnSelection::Nrpn(0, 15) => {
                self.filter_self_oscillation = cc_to_u7(value) != 0;
            }
            // NRPN(0,16): AT Destination（Channel Pressureの加算先）
            RpnSelection::Nrpn(0, 16) => {
                self.at_destination = AtDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,17): Poly AT Destination（Poly Key Pressureの加算先）
            RpnSelection::Nrpn(0, 17) => {
                self.poly_at_destination = AtDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,18)〜(0,21): Operator F-Number Op0〜3
            RpnSelection::Nrpn(0, lsb @ 18..=21) => {
                self.apply_operator_f_number_override((lsb - 18) as usize);
            }
            _ => {}
        }
    }
}

impl Plugin for Ym38x6Plugin {
    const NAME: &'static str = "38x6";
    const VENDOR: &'static str = "ym38x6";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.engine = Ym38x6Engine::new(self.sample_rate);
        self.effects = MasterEffects::new(self.sample_rate);
        let num_out = audio_io_layout
            .main_output_channels
            .map(|n| n.get() as usize)
            .unwrap_or(2);
        // プロセスコールバック内でアロケーションしないよう最大サイズで確保
        self.render_buffer
            .resize(buffer_config.max_buffer_size as usize * num_out, 0.0);
        self.preset_bank = PresetBank::load_from_dir(&presets_dir());
        true
    }

    fn reset(&mut self) {
        self.engine = Ym38x6Engine::new(self.sample_rate);
        self.effects = MasterEffects::new(self.sample_rate);
        self.lfo_cc1 = 0;
        self.lfo_rpn0_5 = 64;
        self.lfo_destination = Ym38x6LfoDestination::Pitch;
        self.lfo_waveform = LfoWaveform::Triangle;
        self.last_lfo_rate_param = 0;
        self.effective_lfo_rate = 0;
        self.last_lfo_depth_param = 0;
        self.effective_lfo_depth = 0;
        self.last_lfo_delay_param = 0;
        self.effective_lfo_delay = 0;
        self.last_rev_send = 0;
        self.last_cho_send = 0;
        self.rpn_msb = 0;
        self.rpn_lsb = 0;
        self.nrpn_msb = 0;
        self.nrpn_lsb = 0;
        self.rpn_selection = RpnSelection::default();
        self.last_reverb_type = DEFAULT_REVERB_TYPE;
        self.last_reverb_time = DEFAULT_REVERB_TIME;
        self.last_chorus_type = DEFAULT_CHORUS_TYPE;
        self.last_chorus_mod_rate = DEFAULT_CHORUS_MOD_RATE;
        self.last_chorus_mod_depth = DEFAULT_CHORUS_MOD_DEPTH;
        self.last_chorus_feedback = DEFAULT_CHORUS_FEEDBACK;
        self.last_chorus_send_to_reverb = DEFAULT_CHORUS_SEND_TO_REVERB;
        self.at_destination = AtDestination::default();
        self.poly_at_destination = AtDestination::default();
        self.channel_pressure = 0;
        self.poly_pressure.clear();
        self.data_entry_msb = 0;
        self.data_entry_lsb = 0;
        self.operator_f_number_override = [F_NUMBER_CENTER; 4];
        self.bank_select_msb = [0; 16];
        self.bank_select_lsb = [0; 16];
        self.program_patch = [None; 16];
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_editor(
            self.egui_state.clone(),
            self.pending_gui_preset.clone(),
            self.params.clone(),
        )
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Algorithm：DAWオートメーションで値が変化した場合のみ反映する（NRPN(0,9)はself.algorithmへ
        // 直接書き込まれ、ここでの値が前回と同じ間は上書きされない。差分検知方式）。
        let algorithm = self.params.algorithm.value() as u8;
        if algorithm != self.last_algorithm {
            self.algorithm = algorithm;
            self.last_algorithm = algorithm;
        }

        // Waveform Op0〜3：algorithmと同じ差分検知方式。NRPN(0,10)〜(0,13)直接書き込みと共存する。
        for i in 0..4 {
            let wf = self.params.operators[i].waveform.value() as u8;
            if wf != self.last_operator_waveforms[i] {
                self.operator_waveforms[i] = wf;
                self.last_operator_waveforms[i] = wf;
            }
        }

        // GUIプリセット選択のNRPN専用パラメーター（filter_type/filter_self_oscillation/
        // operator_waveforms）を反映し、CLAPのprogram_patchをクリアする。
        // DAW公開パラメーターはeditor()内でParamSetter経由で書き戻し済み。
        if let Ok(mut pending) = self.pending_gui_preset.try_lock() {
            if let Some(patch) = pending.take() {
                self.filter_type = patch.channel.filter_type;
                self.filter_self_oscillation = patch.channel.filter_self_oscillation;
                self.program_patch = [None; 16];
            }
        }

        // channel_patchは発音中ボイスのリアルタイムDAWパラメーター更新用（常にGUI/DAW値）。
        // Program Change（CC102/MidiProgramChange）はMIDIチャンネルごとに program_patch[ch] へ保存し、
        // note-on時にそのMIDIチャンネルの program_patch を参照する。
        // 発音中の既存ボイスへは影響しない（チャンネルループは常にchannel_patchを使用）。
        let channel_patch = self.build_patch();
        self.engine.set_patch(channel_patch);

        // 発音中チャンネルへDAWオートメーションの変更とAT/Poly AT Destinationの加算を反映する
        // （MIDIノート番号をそのままチャンネルIDとして使うため0〜127を走査する。
        // 非発音チャンネルへのset_*はno-opになる）
        for note in 0u8..MIDI_NOTE_COUNT {
            let ch_id = note as usize;
            let mut note_patch = channel_patch;
            apply_at_modulation(
                note,
                self.at_destination,
                self.poly_at_destination,
                self.channel_pressure,
                &self.poly_pressure,
                &mut note_patch,
            );
            self.engine.set_channel_params(ch_id, note_patch.channel);
            for (op_index, op) in note_patch.operators.iter().enumerate() {
                self.engine.set_operator_params(ch_id, op_index, *op);
            }
        }

        // パフォーマンスLFO Rate/Depth/Delay：DAWパラメーターとCC76/77/78の両方から
        // 設定され得るため、2シャドウ方式で差分検知する。
        let lfo_rate_param = self.params.lfo_rate.value() as u8;
        if lfo_rate_param != self.last_lfo_rate_param {
            self.last_lfo_rate_param = lfo_rate_param;
            self.effective_lfo_rate = lfo_rate_param;
            self.apply_performance_lfo_to_active();
        }
        let lfo_depth_param = self.params.lfo_depth.value() as u8;
        if lfo_depth_param != self.last_lfo_depth_param {
            self.last_lfo_depth_param = lfo_depth_param;
            self.effective_lfo_depth = lfo_depth_param;
            self.apply_performance_lfo_to_active();
        }
        let lfo_delay_param = self.params.lfo_delay.value() as u8;
        if lfo_delay_param != self.last_lfo_delay_param {
            self.last_lfo_delay_param = lfo_delay_param;
            self.effective_lfo_delay = lfo_delay_param;
            self.apply_performance_lfo_to_active();
        }

        // Reverb/Chorus Send：DAWパラメーターとCC91/93の両方から設定され得るため、
        // マスターエフェクト5パラメーターと同じ1シャドウ差分検知方式で適用する。
        let rev_send = self.params.rev_send.value() as u8;
        if rev_send != self.last_rev_send {
            self.effects.set_reverb_send(rev_send);
            self.last_rev_send = rev_send;
        }
        let cho_send = self.params.cho_send.value() as u8;
        if cho_send != self.last_cho_send {
            self.effects.set_chorus_send(cho_send);
            self.last_cho_send = cho_send;
        }

        // マスター単位パラメーター：DAWオートメーションで値が変化した場合のみeffectsへ反映する。
        // NRPN(0,2)〜(0,8)はeffectsへ直接書き込まれ、ここでの値が前回と同じ間は上書きされない
        // （差分検知方式。NRPNの変更はnice-plug側のパラメーター表示には反映されない）。
        let reverb_type = self.params.reverb_type.value() as u8;
        if reverb_type != self.last_reverb_type {
            self.effects.set_reverb_type(ReverbType::from_u8(reverb_type));
            self.last_reverb_type = reverb_type;
        }
        let reverb_time = self.params.reverb_time.value() as u8;
        if reverb_time != self.last_reverb_time {
            self.effects.set_reverb_time(reverb_time);
            self.last_reverb_time = reverb_time;
        }
        let chorus_type = self.params.chorus_type.value() as u8;
        if chorus_type != self.last_chorus_type {
            self.effects.set_chorus_type(ChorusType::from_u8(chorus_type));
            self.last_chorus_type = chorus_type;
        }
        let chorus_mod_rate = self.params.chorus_mod_rate.value() as u8;
        if chorus_mod_rate != self.last_chorus_mod_rate {
            self.effects.set_chorus_mod_rate(chorus_mod_rate);
            self.last_chorus_mod_rate = chorus_mod_rate;
        }
        let chorus_mod_depth = self.params.chorus_mod_depth.value() as u8;
        if chorus_mod_depth != self.last_chorus_mod_depth {
            self.effects.set_chorus_mod_depth(chorus_mod_depth);
            self.last_chorus_mod_depth = chorus_mod_depth;
        }
        let chorus_feedback = self.params.chorus_feedback.value() as u8;
        if chorus_feedback != self.last_chorus_feedback {
            self.effects.set_chorus_feedback(chorus_feedback);
            self.last_chorus_feedback = chorus_feedback;
        }
        let chorus_send_to_reverb = self.params.chorus_send_to_reverb.value() as u8;
        if chorus_send_to_reverb != self.last_chorus_send_to_reverb {
            self.effects.set_chorus_send_to_reverb(chorus_send_to_reverb);
            self.last_chorus_send_to_reverb = chorus_send_to_reverb;
        }

        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { channel, note, velocity, .. } if velocity > 0.0 => {
                    let freq = 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);
                    let velocity_u8 = (velocity * 127.0).round() as u8;
                    // ボイスIDは midi_ch*128+note で符号化する。一意性（Note Off/同音再アタック）と
                    // グループ性（ピッチベンドのMIDIチャンネル一括適用 = id>>7）を同時に満たす。
                    let ch_id = midi_channel_note_id(channel, note);
                    // このMIDIチャンネルのProgram Change（CC102/CLAP）パッチを優先。なければGUI値。
                    let note_on_patch = self.program_patch[channel as usize].unwrap_or(channel_patch);
                    self.engine.note_on_with_velocity(ch_id, freq, velocity_u8, note_on_patch);
                    // このMIDIチャンネルの現在のベンド量と音量ゲインを新ボイスへ反映する
                    self.engine.set_pitch_bend(ch_id, self.channel_bend_cents[channel as usize]);
                    self.engine.set_channel_volume(ch_id, channel_gain(self.cc7[channel as usize], self.cc11[channel as usize]));
                    self.apply_performance_lfo(ch_id);
                    for (op_index, &f_number) in self.operator_f_number_override.iter().enumerate() {
                        self.engine.set_operator_f_number(ch_id, op_index, f_number);
                    }
                }
                NoteEvent::NoteOn { channel, note, .. } | NoteEvent::NoteOff { channel, note, .. } => {
                    // velocity=0 の NoteOn も NoteOff として扱う（MIDI仕様）
                    self.engine.note_off(midi_channel_note_id(channel, note));
                    self.poly_pressure.remove(&note);
                }
                // MIDIピッチベンド：このMIDIチャンネルの全ノートへセント換算で一括適用する。
                // nice-plug/nih-plugのvalueは0.0〜1.0（0.5=センター）。
                NoteEvent::MidiPitchBend { channel, value, .. } => {
                    let cents = (value - 0.5) * 2.0 * self.pitch_bend_range * 100.0;
                    self.channel_bend_cents[channel as usize] = cents;
                    self.engine.set_pitch_bend_group(channel as usize, cents);
                }
                // AT/Poly AT Destination（NRPN(0,16)/(0,17)）の加算対象
                NoteEvent::MidiChannelPressure { pressure, .. } => {
                    self.channel_pressure = cc_to_u8(pressure);
                }
                NoteEvent::PolyPressure { note, pressure, .. } => {
                    self.poly_pressure.insert(note, cc_to_u8(pressure));
                }
                // Program Change：CC0/CC32で選択中のバンクと合わせてパッチを選択する
                // （VST3では届かない。CLAPのみ。MidiConfig::MidiCCsの仕様。VST3ではGUI経由のParamSetterを使う）。
                NoteEvent::MidiProgramChange { program, channel, .. } => {
                    let bank = (self.bank_select_msb[channel as usize] as u16) * 128
                        + self.bank_select_lsb[channel as usize] as u16;
                    self.program_patch[channel as usize] =
                        Some(self.preset_bank.patch_for_program(bank, program));
                }
                // パフォーマンスLFO（CC1/76/77/78・RPN0,5・NRPN Destination/Waveform）・
                // マスターエフェクトセンドレベル（CC91/93）
                NoteEvent::MidiCC { cc, value, channel, .. } => match cc {
                    // CC7/CC11: GM2準拠のチャンネル音量（set_pitch_bend_groupと同パターン）。
                    // 実効ゲイン = (cc7/127)^2 × (cc11/127)^2（GM2の40·log10カーブ）。
                    7 => {
                        self.cc7[channel as usize] = cc_to_u7(value);
                        self.engine.set_channel_volume_group(channel as usize, channel_gain(self.cc7[channel as usize], self.cc11[channel as usize]));
                    }
                    11 => {
                        self.cc11[channel as usize] = cc_to_u7(value);
                        self.engine.set_channel_volume_group(channel as usize, channel_gain(self.cc7[channel as usize], self.cc11[channel as usize]));
                    }
                    1 => {
                        self.lfo_cc1 = cc_to_u8(value);
                        self.apply_performance_lfo_to_active();
                    }
                    76 => {
                        self.effective_lfo_rate = cc_to_u8(value);
                        self.apply_performance_lfo_to_active();
                    }
                    77 => {
                        self.effective_lfo_depth = cc_to_u8(value);
                        self.apply_performance_lfo_to_active();
                    }
                    78 => {
                        self.effective_lfo_delay = cc_to_u8(value);
                        self.apply_performance_lfo_to_active();
                    }
                    // Bank Select（CC0=MSB, CC32=LSB）：MIDIチャンネルごとに管理
                    0 => self.bank_select_msb[channel as usize] = cc_to_u7(value),
                    32 => self.bank_select_lsb[channel as usize] = cc_to_u7(value),
                    98 => {
                        self.nrpn_lsb = cc_to_u7(value);
                        self.update_rpn_selection(true);
                    }
                    99 => {
                        self.nrpn_msb = cc_to_u7(value);
                        self.update_rpn_selection(true);
                    }
                    100 => {
                        self.rpn_lsb = cc_to_u7(value);
                        self.update_rpn_selection(false);
                    }
                    101 => {
                        self.rpn_msb = cc_to_u7(value);
                        self.update_rpn_selection(false);
                    }
                    6 => self.handle_data_entry(value),
                    38 => {
                        self.data_entry_lsb = cc_to_u7(value);
                        if let RpnSelection::Nrpn(0, lsb @ 18..=21) = self.rpn_selection {
                            self.apply_operator_f_number_override((lsb - 18) as usize);
                        }
                    }
                    91 => self.effects.set_reverb_send(cc_to_u8(value)),
                    93 => self.effects.set_chorus_send(cc_to_u8(value)),
                    // CC102: Program Change 代替（VST3 では MidiProgramChange が届かないため）。
                    // 値 0-127 をプログラム番号として、現在の CC0/CC32 バンクと合わせてパッチを選択する。
                    // 旧実装は VOPMex 互換で CC92 を使っていたが、CC92 は GM2 で Effects 2 Depth
                    // （トレモロ）に予約され衝突するため、GM2 未定義ブロックの先頭 CC102 へ移した。
                    102 => {
                        let prog = cc_to_u7(value);
                        let bank = (self.bank_select_msb[channel as usize] as u16) * 128
                            + self.bank_select_lsb[channel as usize] as u16;
                        self.program_patch[channel as usize] =
                            Some(self.preset_bank.patch_for_program(bank, prog));
                    }
                    // Operator Key On/Off（CC103〜106、≧64でキーオン/<64でキーオフ、spec-sound.md参照）。
                    // CC102 を Program Change 代替に使うため、OP単位キーオンは1つ繰り下げた。
                    103..=106 => {
                        // 全OP独立（Op3も特別扱いしない）。全消しはNote-Off/CC120で行う。
                        let op_index = (cc - 103) as usize;
                        let key_on = cc_to_u7(value) >= 64;
                        for note in 0u8..MIDI_NOTE_COUNT {
                            let ch_id = note as usize;
                            if key_on {
                                self.engine.note_on_operator(ch_id, op_index);
                            } else {
                                self.engine.note_off_operator(ch_id, op_index);
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        let num_channels = buffer.channels();
        let num_samples = buffer.samples();
        let interleaved_len = num_samples * num_channels;

        // 作業バッファが足りない場合（ホスト規約違反）は拡張
        if interleaved_len > self.render_buffer.len() {
            self.render_buffer.resize(interleaved_len, 0.0);
        }
        let buf = &mut self.render_buffer[..interleaved_len];
        buf.fill(0.0);
        self.engine.render(buf, num_channels);
        self.effects.process(buf, num_channels);

        // インターリーブ → nice-plugのチャンネル分離レイアウトに変換
        let output_slices = buffer.as_slice();
        for ch in 0..num_channels {
            for s in 0..num_samples {
                output_slices[ch][s] += buf[s * num_channels + ch];
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Ym38x6Plugin {
    const CLAP_ID: &'static str = "com.ym38x6.ym38x6";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("38x6 FM Synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
    ];
}

impl Vst3Plugin for Ym38x6Plugin {
    const VST3_CLASS_ID: [u8; 16] = *b"Ym38x6--FM4-----";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
    ];
}

nice_export_clap!(Ym38x6Plugin);
nice_export_vst3!(Ym38x6Plugin);
