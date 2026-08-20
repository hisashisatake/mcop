mod editor;
mod param_adapter;
mod params;

use op505_midi::{
    apply_expression_modulation, apply_pitch_fg_expression, apply_soft_pedal, cc_to_u7, cc_to_u8,
    control_target, released_notes, ControlTarget, ExpressionDestination, PedalState, RpnTracker,
};
use params::{
    Op505VstParams, DEFAULT_CHORUS_FEEDBACK, DEFAULT_CHORUS_MOD_DEPTH, DEFAULT_CHORUS_MOD_RATE,
    DEFAULT_CHORUS_SEND_TO_REVERB, DEFAULT_CHORUS_TYPE, DEFAULT_REVERB_TIME, DEFAULT_REVERB_TYPE,
};

use nice_plug::prelude::*;
use nice_plug_egui::EguiState;
use op505_core::{
    op505_presets_dir, Op505BipolarFg, Op505ChannelParams, Op505Engine, Op505OperatorParams, Op505Patch,
    Op505PresetBank,
};
use sound_core::{cc76_to_rate_scale, AudioProcessor, ChorusType, MasterEffects, ReverbType, Vco};
use sound_fm::mapping::F_NUMBER_CENTER;
use std::sync::Arc;

use crate::params::Op505EgBank;

/// MIDIノート番号の総数（0〜127）。MIDIノート番号をそのままチャンネルIDとして使うため
/// （1ノート=1チャンネル）、発音中チャンネルを走査するループの上限に使う
/// （`ym38x6-vst`と同じ設計）。
const MIDI_NOTE_COUNT: u8 = 128;

/// 対応するMIDIチャンネル数（0〜15の16ch。表情CC/ペダルはチャンネルごとに独立管理する）。
const MIDI_CHANNEL_COUNT: usize = 16;

/// CC7(Channel Volume) と CC11(Expression) の値（0〜127）から GM2 準拠のゲインを計算する
/// （`ym38x6-vst`と同一式）。
#[inline]
fn channel_gain(cc7: u8, cc11: u8) -> f32 {
    let v7 = cc7 as f32 / 127.0;
    let v11 = cc11 as f32 / 127.0;
    v7 * v7 * v11 * v11
}

/// MIDIチャンネル(0〜15)とノート番号(0〜127)からエンジンのボイスIDを符号化する。
/// `midi_ch*128 + note`（`ym38x6-vst`と同一設計）。
#[inline]
fn midi_channel_note_id(channel: u8, note: u8) -> usize {
    (channel as usize) * 128 + note as usize
}

struct Op505Plugin {
    params: Arc<Op505VstParams>,
    engine: Op505Engine,
    effects: MasterEffects,
    render_buffer: Vec<f32>,
    sample_rate: f32,

    // TimeEg 7本（`params.egs`のpersist状態）のオーディオスレッド側キャッシュ。
    // オーディオスレッドは`RwLock`をブロックせず`try_read()`で取得できたときだけ更新する
    // （取れなければ前ブロックの値を使う＝1〜2ブロック遅れて必ず収束する。plan参照）。
    cached_egs: Op505EgBank,

    // 発音中チャンネルID列挙用のスクラッチバッファ（`Op505Engine::collect_active_channels`が
    // 書き込む）。オーディオスレッドでのアロケーションを避けるため使い回す（毎回`Vec::new()`しない）。
    active_ids: Vec<usize>,

    // Algorithm：NRPN(0,9)に加えてDAWパラメーターとしても公開する（1シャドウ差分検知方式、
    // `ym38x6-vst`と同型）。
    algorithm: u8,
    last_algorithm: u8,

    // Filter Type / Self-Oscillation：op505ではDAWパラメーターとして存在するため、
    // NRPN(0,14)/(0,15)と1シャドウ差分検知で二重公開する（ym38x6ではNRPN専用だったが、
    // op505フェーズ1でDAWパラメーター化済みのため扱いが異なる）。
    filter_type: u8,
    last_filter_type_param: u8,
    filter_self_oscillation: bool,
    last_filter_self_oscillation_param: bool,

    // operator_waveformsはDAWパラメーター（params.operators[i].waveform）との二重管理。
    // DAWオートメーション変化時はprocess()内の差分検知で上書き、NRPN(0,10)〜(0,13)は直接書き込む。
    operator_waveforms: [u8; 4],
    last_operator_waveforms: [u8; 4],

    // Pitch FG（②③層の補正を受ける唯一のFGスロット、spec-sound.md「演奏層による補正」節）。
    // CC1/76/77/78の生値をMIDIチャンネルごとに保持し、build_patch()で毎ブロックPitch FGの
    // Depth/rate_scaleへ計算適用する。CC78はop505のTimeEgにDelayフィールドが無いため、
    // 第0段が`level=0`の待ち段であるときに限りその段の`time`へ相対補正する（plan参照）。
    pitch_fg_cc1: [u8; MIDI_CHANNEL_COUNT],  // CC1 Modulation Wheel（0〜127、Depthへ瞬間加算・セント換算）
    pitch_fg_cc76: [u8; MIDI_CHANNEL_COUNT], // CC76 Vibrato Rate（0〜127、64=無補正、rate_scale経由）
    pitch_fg_cc77: [u8; MIDI_CHANNEL_COUNT], // CC77 Vibrato Depth（0〜255、Depthへ0起点加算）
    pitch_fg_cc78: [u8; MIDI_CHANNEL_COUNT], // CC78 Vibrato Delay（0〜127、64=無補正、第0段timeへ相対補正）
    pitch_fg_rpn0_5: u8,                      // RPN0,5 Modulation Depth Range（GM2準拠0〜127、デフォルト64）

    // Reverb/Chorus Send：DAWパラメーターとCC91/93の両方から設定され得るため、
    // マスターエフェクト5パラメーターと同じ1シャドウ差分検知方式で管理する。
    last_rev_send: u8,
    last_cho_send: u8,

    // RPN/NRPN選択状態（MIDIチャンネル非依存のグローバル状態、`ym38x6-vst`と同型。
    // Algorithm/Filter Type等のNRPN対象はパッチ全体のグローバルパラメーターのため）。
    rpn: RpnTracker,

    // マスター単位パラメーターの「前回ブロックで適用したDAW値」（1シャドウ差分検知方式）。
    // Reverb/Chorus TypeはNRPN(0,2)/(0,3)等からも直接effectsへ書き込まれるため、
    // DAW値が変化していない間はNRPN側の設定が上書きされない。
    last_reverb_type: u8,
    last_reverb_time: u8,
    last_chorus_type: u8,
    last_chorus_mod_rate: u8,
    last_chorus_mod_depth: u8,
    last_chorus_feedback: u8,
    last_chorus_send_to_reverb: u8,

    // AT/Poly AT Destination（NRPN(0,16)/(0,17)）と、加算対象のプレッシャー値。
    // Destinationはグローバル単一（NRPN同様）、プレッシャー値はMIDIチャンネルごとに保持する。
    at_destination: ExpressionDestination,
    poly_at_destination: ExpressionDestination,
    channel_pressure: [u8; MIDI_CHANNEL_COUNT],
    /// Poly Key Pressure（ノート番号→圧力値）。オーディオスレッドでのアロケーションを避けるため
    /// `HashMap`ではなくノート番号(0〜127)で直接引ける固定長配列にしている
    /// （`ym38x6-vst`からの変更点、`midi.rs`のコメント参照）。
    poly_pressure: [[u8; 128]; MIDI_CHANNEL_COUNT],

    // CC2(ブレス)/CC4(フット)の加算先（NRPN(0,34)/(0,35)）はグローバル単一シャドウ
    // （既存のapply_expression_modulation経路がグローバル単一パッチ前提のため揃える）。
    // 現在値はMIDIチャンネルごとに保持する。
    // 既定行先はCC2→TLキャリア一括（ウインド楽器風の明るさ/音量スウェル）、
    // CC4→Filter Cutoff（古典的ワウペダル＝手動ワウ）。
    cc2: [u8; MIDI_CHANNEL_COUNT],
    cc4: [u8; MIDI_CHANNEL_COUNT],
    cc2_destination: ExpressionDestination,
    cc4_destination: ExpressionDestination,

    // NRPN(0,18)〜(0,21): Operator F-Number Op0〜3（CC6+CC38の14bit値→13bit(0〜8191)にclamp）
    data_entry_msb: u8,                   // CC6 (Data Entry MSB) の最新値
    data_entry_lsb: u8,                   // CC38 (Data Entry LSB) の最新値
    operator_f_number_override: [u16; 4], // 各Opの上書き値。初期値F_NUMBER_CENTER（上書きなし）

    // Bank Select（CC0=MSB, CC32=LSB）+ Program Change：MIDIチャンネルごとに管理
    // （`ym38x6-vst`と同一設計）。
    bank_select_msb: [u8; 16],
    bank_select_lsb: [u8; 16],
    /// Program Changeで選択されたパッチ（MIDIチャンネルごと）。該当プリセットが無ければ
    /// `None`（`Op505PresetBank::get`と同じくフォールバックせず「見つからない」を伝える。
    /// `.op505`には`.38x6`のような波形メモリ/GM2/プレースホルダー代替が無いため）。
    program_patch: [Option<Op505Patch>; 16],

    // ピッチベンド（MIDIチャンネル単位、`ym38x6-vst`と同一設計）。
    channel_bend_cents: [f32; 16],
    /// ピッチベンド感度（半音）。RPN(0,0)で設定可能（フェーズ2）。既定2半音。
    pitch_bend_range: f32,

    // CC7/CC11 チャンネル音量（GM2準拠、`ym38x6-vst`と同一設計）。
    cc7: [u8; 16],
    cc11: [u8; 16],

    // ペダル（CC64 Sustain / CC66 Sostenuto / CC67 Soft、ホールドフラグ方式。
    // spec-sound.md「サステインペダル（CC64）の実装方針」参照）。エンジン無改造・
    // 1ノート=1チャンネルのまま、MIDIチャンネルごとに管理する。
    pedals: [PedalState; 16],

    // `op505_presets_dir()`から読み込んだユーザープリセット集合（`initialize()`で読み込む）。
    preset_bank: Op505PresetBank,

    // GUIエディターのウィンドウサイズ状態（`editor()`で使い回す）。
    egui_state: Arc<EguiState>,
}

impl Default for Op505Plugin {
    fn default() -> Self {
        const DEFAULT_SR: f32 = 44100.0;
        let params = Arc::new(Op505VstParams::default());
        let cached_egs = *params.egs.read().expect("Poisoned RwLock on read");
        Self {
            params,
            engine: Op505Engine::new(DEFAULT_SR),
            effects: MasterEffects::new(DEFAULT_SR),
            render_buffer: Vec::new(),
            sample_rate: DEFAULT_SR,
            cached_egs,
            active_ids: Vec::with_capacity(256),
            algorithm: params::DEFAULT_ALGORITHM,
            last_algorithm: params::DEFAULT_ALGORITHM,
            filter_type: 0,
            last_filter_type_param: 0,
            filter_self_oscillation: true,
            last_filter_self_oscillation_param: true,
            operator_waveforms: [0; 4],
            last_operator_waveforms: [0; 4],
            pitch_fg_cc1: [0; MIDI_CHANNEL_COUNT],
            pitch_fg_cc76: [64; MIDI_CHANNEL_COUNT],
            pitch_fg_cc77: [0; MIDI_CHANNEL_COUNT],
            pitch_fg_cc78: [64; MIDI_CHANNEL_COUNT],
            pitch_fg_rpn0_5: 64,
            last_rev_send: 0,
            last_cho_send: 0,
            rpn: RpnTracker::default(),
            last_reverb_type: DEFAULT_REVERB_TYPE,
            last_reverb_time: DEFAULT_REVERB_TIME,
            last_chorus_type: DEFAULT_CHORUS_TYPE,
            last_chorus_mod_rate: DEFAULT_CHORUS_MOD_RATE,
            last_chorus_mod_depth: DEFAULT_CHORUS_MOD_DEPTH,
            last_chorus_feedback: DEFAULT_CHORUS_FEEDBACK,
            last_chorus_send_to_reverb: DEFAULT_CHORUS_SEND_TO_REVERB,
            at_destination: ExpressionDestination::default(),
            poly_at_destination: ExpressionDestination::default(),
            channel_pressure: [0; MIDI_CHANNEL_COUNT],
            poly_pressure: [[0; 128]; MIDI_CHANNEL_COUNT],
            cc2: [0; MIDI_CHANNEL_COUNT],
            cc4: [0; MIDI_CHANNEL_COUNT],
            cc2_destination: ExpressionDestination::TlCarriers,
            cc4_destination: ExpressionDestination::FilterCutoff,
            data_entry_msb: 0,
            data_entry_lsb: 0,
            operator_f_number_override: [F_NUMBER_CENTER; 4],
            bank_select_msb: [0; 16],
            bank_select_lsb: [0; 16],
            program_patch: [None; 16],
            channel_bend_cents: [0.0; 16],
            pitch_bend_range: 2.0,
            cc7: [127; 16],
            cc11: [127; 16],
            pedals: [PedalState::default(); 16],
            preset_bank: Op505PresetBank::default(),
            egui_state: EguiState::from_size(1200, 680),
        }
    }
}

impl Op505Plugin {
    /// 現在のDAWパラメーター・NRPN状態・`cached_egs`から`Op505Patch`を構築する（MIDIチャンネル
    /// 非依存。CC1/76/77/78のPitch FG演奏補正は`apply_pitch_fg_expression`で別途適用する）。
    fn build_patch(&self) -> Op505Patch {
        let p = &self.params;
        let egs = &self.cached_egs;
        let operators = std::array::from_fn(|i| {
            let op = &p.operators[i];
            Op505OperatorParams {
                tl: op.tl.value() as u8,
                eg: egs.operators[i],
                mul: op.mul.value() as u8,
                dt1: op.dt1.value() as u8,
                ksr: op.ksr.value() as u8,
                am_enable: op.ame.value(),
                velocity_sensitivity: op.vel_sens.value() as u8,
                waveform: self.operator_waveforms[i],
                op_fine_tune: op.op_fine_tune.value() as u8,
                eg_shift: op.eg_shift.value() as u8,
                level_scale: op.level_scale.value() as u8,
                velocity_gain: op.velocity_gain.value() as u8,
            }
        });

        // Pitch FGの生値（①音色パッチそのまま）をここでは詰めるだけにする。CC1/76/77/78の
        // ②③層補正は`apply_pitch_fg_expression`で後段適用する（`build_patch()`内に埋め込むと
        // Program Change選択中（`program_patch`がSomeでbuild_patch()自体が呼ばれない）チャンネルで
        // 補正が丸ごとスキップされてしまうバグがあったため、apply_expression_modulation/
        // apply_soft_pedalと同じ「note_patchへの後処理」パターンへ分離した）。
        let channel = Op505ChannelParams {
            algorithm: self.algorithm,
            feedback: p.feedback.value() as u8,
            filter_cutoff: p.cutoff.value() as u8,
            filter_resonance: p.resonance.value() as u8,
            filter_type: self.filter_type,
            filter_self_oscillation: self.filter_self_oscillation,
            pitch_fg: Op505BipolarFg { eg: egs.pitch_fg, depth: p.pitch_fg_depth.value() as u8 },
            cutoff_fg: Op505BipolarFg { eg: egs.cutoff_fg, depth: p.cutoff_fg_depth.value() as u8 },
            gain_fg: egs.gain_fg,
            gain_fg_to_master: p.gain_fg_to_master.value(),
            gain_fg_to_operators: p.gain_fg_to_operators.value(),
        };

        Op505Patch { operators, channel }
    }

    /// CC76(Vibrato Rate)由来のPitch FG速さスケールを計算する（`cc76_to_rate_scale`、
    /// 64=1.0倍=無補正）。build_patch()とは別に、`engine.set_pitch_fg_rate_scale`で
    /// 直接エンジンへ渡す（ChannelParamsを経由しない、pitch_bend/channel_volumeと同じ経路）。
    fn pitch_fg_rate_scale(&self, midi_ch: usize) -> f32 {
        cc76_to_rate_scale(self.pitch_fg_cc76[midi_ch])
    }

    /// NRPN(0,18)〜(0,21)：CC6(Data Entry MSB)+CC38(Data Entry LSB)の14bit値を
    /// 13bit(0〜8191)にclampし、Operator F-Numberとして発音中の全チャンネルへ適用する
    /// （NRPN状態はMIDIチャンネル非依存のグローバルのため、全MIDIチャンネルの発音中ボイスが対象）。
    fn apply_operator_f_number_override(&mut self, op_index: usize) {
        let combined = (self.data_entry_msb as u16) * 128 + self.data_entry_lsb as u16;
        let f_number = combined.min(8191);
        self.operator_f_number_override[op_index] = f_number;
        let mut ids = std::mem::take(&mut self.active_ids);
        self.engine.collect_active_channels(&mut ids);
        for &ch_id in ids.iter() {
            self.engine.set_operator_f_number(ch_id, op_index, f_number);
        }
        self.active_ids = ids;
    }

    /// CC6(Data Entry MSB)受信時、`op505_midi::control_target`で解決した制御対象に応じて
    /// 値を適用する。`value`はCC値の正規化値（0.0〜1.0）。enum系パラメーターは`cc_to_u7`、
    /// 0〜255連続値パラメーターは`cc_to_u8`で変換する。`_ =>`を使わず全バリアントを列挙する
    /// ことで、`op505-midi`側にNRPNアドレスが追加されたときここもコンパイルエラーで気づける。
    ///
    /// `ControlTarget::ReservedFgLoopCurve`（ym38x6版FG Loop/Curve相当）は欠番として何もしない
    /// （op505のTimeEg 7本はDAWパラメーターではなくpersist状態のため、NRPNから直接触ると
    /// GUI表示と実音がズレる。plan「NRPNからTimeEgは触らない」参照）。
    fn handle_data_entry(&mut self, value: f32) {
        self.data_entry_msb = cc_to_u7(value);
        match control_target(self.rpn.selection) {
            // RPN(0,0): Pitch Bend Sensitivity（半音）。CC6の生値(0〜127)を半音数とする。
            ControlTarget::PitchBendRange => {
                self.pitch_bend_range = cc_to_u7(value) as f32;
            }
            // RPN0,5: Modulation Depth Range（Pitch FGのCC1セント換算係数、64≈50セント）。
            ControlTarget::ModulationDepthRange => {
                self.pitch_fg_rpn0_5 = cc_to_u7(value);
            }
            // NRPN(0,0)〜(0,1)・(0,22)〜(0,27): 旧質感LFOのアドレス。質感LFO退役に伴い
            // 欠番として予約し何もしない（`ReservedFgLoopCurve`と同じ扱い）。
            ControlTarget::ReservedTextureLfo => {}
            // NRPN(0,2): Reverb Type
            ControlTarget::ReverbType => {
                self.effects.set_reverb_type(ReverbType::from_u8(cc_to_u7(value)));
            }
            // NRPN(0,3): Chorus Type
            ControlTarget::ChorusType => {
                self.effects.set_chorus_type(ChorusType::from_u8(cc_to_u7(value)));
            }
            // NRPN(0,4): Reverb Time
            ControlTarget::ReverbTime => {
                self.effects.set_reverb_time(cc_to_u8(value));
            }
            // NRPN(0,5): Chorus Mod Rate
            ControlTarget::ChorusModRate => {
                self.effects.set_chorus_mod_rate(cc_to_u8(value));
            }
            // NRPN(0,6): Chorus Mod Depth
            ControlTarget::ChorusModDepth => {
                self.effects.set_chorus_mod_depth(cc_to_u8(value));
            }
            // NRPN(0,7): Chorus Feedback
            ControlTarget::ChorusFeedback => {
                self.effects.set_chorus_feedback(cc_to_u8(value));
            }
            // NRPN(0,8): Chorus Send To Reverb
            ControlTarget::ChorusSendToReverb => {
                self.effects.set_chorus_send_to_reverb(cc_to_u8(value));
            }
            // NRPN(0,9): Algorithm（0〜7、範囲外は7にclamp）
            ControlTarget::Algorithm => {
                self.algorithm = cc_to_u7(value).min(7);
            }
            // NRPN(0,10)〜(0,13): Waveform Op0〜3（0〜255）
            ControlTarget::OperatorWaveform(op_index) => {
                self.operator_waveforms[op_index as usize] = cc_to_u8(value);
            }
            // NRPN(0,14): Filter Type（0=LP/1=HP/2=BP、範囲外は2にclamp）
            ControlTarget::FilterType => {
                self.filter_type = cc_to_u7(value).min(2);
            }
            // NRPN(0,15): Filter Self-Oscillation（0=OFF/1=ON）
            ControlTarget::FilterSelfOscillation => {
                self.filter_self_oscillation = cc_to_u7(value) != 0;
            }
            // NRPN(0,16): AT Destination（Channel Pressureの加算先）
            ControlTarget::AtDestination => {
                self.at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,17): Poly AT Destination（Poly Key Pressureの加算先）
            ControlTarget::PolyAtDestination => {
                self.poly_at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,18)〜(0,21): Operator F-Number Op0〜3
            ControlTarget::OperatorFNumber(op_index) => {
                self.apply_operator_f_number_override(op_index as usize);
            }
            // NRPN(0,28)〜(0,33): 欠番として予約（TimeEgはpersist状態のためNRPNから触らない）。
            ControlTarget::ReservedFgLoopCurve => {}
            // NRPN(0,34): CC2(ブレス)Destination
            ControlTarget::Cc2Destination => {
                self.cc2_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,35): CC4(フット)Destination。既定FilterCutoff＝手動ワウ。
            ControlTarget::Cc4Destination => {
                self.cc4_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            }
            ControlTarget::Unassigned => {}
        }
    }
}

impl Plugin for Op505Plugin {
    const NAME: &'static str = "OP505";
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
        self.engine = Op505Engine::new(self.sample_rate);
        self.effects = MasterEffects::new(self.sample_rate);
        let num_out = audio_io_layout
            .main_output_channels
            .map(|n| n.get() as usize)
            .unwrap_or(2);
        self.render_buffer
            .resize(buffer_config.max_buffer_size as usize * num_out, 0.0);
        self.preset_bank = Op505PresetBank::load_from_dir(&op505_presets_dir());
        true
    }

    fn reset(&mut self) {
        self.engine = Op505Engine::new(self.sample_rate);
        self.effects = MasterEffects::new(self.sample_rate);
        self.pitch_fg_cc1 = [0; MIDI_CHANNEL_COUNT];
        self.pitch_fg_cc76 = [64; MIDI_CHANNEL_COUNT];
        self.pitch_fg_cc77 = [0; MIDI_CHANNEL_COUNT];
        self.pitch_fg_cc78 = [64; MIDI_CHANNEL_COUNT];
        self.pitch_fg_rpn0_5 = 64;
        self.last_rev_send = 0;
        self.last_cho_send = 0;
        self.rpn.reset();
        self.last_reverb_type = DEFAULT_REVERB_TYPE;
        self.last_reverb_time = DEFAULT_REVERB_TIME;
        self.last_chorus_type = DEFAULT_CHORUS_TYPE;
        self.last_chorus_mod_rate = DEFAULT_CHORUS_MOD_RATE;
        self.last_chorus_mod_depth = DEFAULT_CHORUS_MOD_DEPTH;
        self.last_chorus_feedback = DEFAULT_CHORUS_FEEDBACK;
        self.last_chorus_send_to_reverb = DEFAULT_CHORUS_SEND_TO_REVERB;
        self.at_destination = ExpressionDestination::default();
        self.poly_at_destination = ExpressionDestination::default();
        self.channel_pressure = [0; MIDI_CHANNEL_COUNT];
        self.poly_pressure = [[0; 128]; MIDI_CHANNEL_COUNT];
        self.cc2 = [0; MIDI_CHANNEL_COUNT];
        self.cc4 = [0; MIDI_CHANNEL_COUNT];
        self.cc2_destination = ExpressionDestination::TlCarriers;
        self.cc4_destination = ExpressionDestination::FilterCutoff;
        self.data_entry_msb = 0;
        self.data_entry_lsb = 0;
        self.operator_f_number_override = [F_NUMBER_CENTER; 4];
        self.bank_select_msb = [0; 16];
        self.bank_select_lsb = [0; 16];
        self.program_patch = [None; 16];
        self.channel_bend_cents = [0.0; 16];
        self.pitch_bend_range = 2.0;
        self.cc7 = [127; 16];
        self.cc11 = [127; 16];
        self.pedals = [PedalState::default(); 16];
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_editor(self.egui_state.clone(), self.params.clone())
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // TimeEg 7本：GUIエディタが書き込んだ`params.egs`をブロックしないtry_readで取り込む
        // （取れなければ前ブロックのcached_egsを使い続ける。plan参照）。
        if let Ok(egs) = self.params.egs.try_read() {
            if *egs != self.cached_egs {
                self.cached_egs = *egs;
            }
        }

        // TimeEgのテンポ同期用BPM。ホストが未再生等でtempoを返さない場合は前回値を保持する
        // （Op505Engine::set_tempoが0以下を無視するのと同じ防御）。
        if let Some(tempo) = context.transport().tempo {
            self.engine.set_tempo(tempo as f32);
        }

        // Algorithm：DAWオートメーションで値が変化した場合のみ反映する（NRPN(0,9)はself.algorithmへ
        // 直接書き込まれ、ここでの値が前回と同じ間は上書きされない。差分検知方式）。
        let algorithm = self.params.algorithm.value() as u8;
        if algorithm != self.last_algorithm {
            self.algorithm = algorithm;
            self.last_algorithm = algorithm;
        }

        // Filter Type / Self-Oscillation：algorithmと同じ1シャドウ差分検知方式。
        // NRPN(0,14)/(0,15)直接書き込みと共存する。
        let filter_type_param = self.params.filter_type.value() as u8;
        if filter_type_param != self.last_filter_type_param {
            self.filter_type = filter_type_param;
            self.last_filter_type_param = filter_type_param;
        }
        let filter_self_oscillation_param = self.params.filter_self_oscillation.value();
        if filter_self_oscillation_param != self.last_filter_self_oscillation_param {
            self.filter_self_oscillation = filter_self_oscillation_param;
            self.last_filter_self_oscillation_param = filter_self_oscillation_param;
        }

        // Waveform Op0〜3：algorithmと同じ差分検知方式。NRPN(0,10)〜(0,13)直接書き込みと共存する。
        for i in 0..4 {
            let wf = self.params.operators[i].waveform.value() as u8;
            if wf != self.last_operator_waveforms[i] {
                self.operator_waveforms[i] = wf;
                self.last_operator_waveforms[i] = wf;
            }
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

        // 発音中チャンネルへDAWオートメーション・NRPN状態・表情CC/Soft Pedalの変更を反映する
        // （`Op505Engine::collect_active_channels`で発音中ボイスのみ列挙し、MIDIチャンネルごとの
        // 実効パッチを組み立てる。借用衝突（&self.active_ids と &mut self.engine）を避けるため
        // 一時的にmoveする。Vecの実体は移動するだけでアロケーションは走らない）。
        let mut ids = std::mem::take(&mut self.active_ids);
        self.engine.collect_active_channels(&mut ids);
        // MIDIチャンネルごとの基底パッチをキャッシュする（16chループのたびbuild_patch()を
        // 再計算しないよう、実際に発音中のチャンネルだけ計算する）。
        let mut channel_patches: [Option<Op505Patch>; MIDI_CHANNEL_COUNT] = [None; MIDI_CHANNEL_COUNT];
        for &ch_id in ids.iter() {
            let midi_ch = ch_id >> 7;
            let note = (ch_id & 127) as u8;
            let base_patch = channel_patches[midi_ch].unwrap_or_else(|| {
                // Program Changeで選ばれていればそれを基底に、無ければDAWパラメーター由来。
                let patch = self.program_patch[midi_ch].unwrap_or_else(|| self.build_patch());
                channel_patches[midi_ch] = Some(patch);
                patch
            });
            let mut note_patch = base_patch;
            apply_expression_modulation(
                note,
                &[
                    (self.cc2[midi_ch], self.cc2_destination),
                    (self.cc4[midi_ch], self.cc4_destination),
                    (self.channel_pressure[midi_ch], self.at_destination),
                ],
                self.poly_at_destination,
                &self.poly_pressure[midi_ch],
                &mut note_patch,
            );
            apply_pitch_fg_expression(
                &mut note_patch,
                self.pitch_fg_cc1[midi_ch],
                self.pitch_fg_cc77[midi_ch],
                self.pitch_fg_cc78[midi_ch],
                self.pitch_fg_rpn0_5,
            );
            if self.pedals[midi_ch].soft_notes & (1u128 << note) != 0 {
                apply_soft_pedal(&mut note_patch, self.pedals[midi_ch].cc67);
            }
            self.engine.set_channel_params(ch_id, note_patch.channel);
            for (op_index, op) in note_patch.operators.iter().enumerate() {
                self.engine.set_operator_params(ch_id, op_index, *op);
            }
            self.engine.set_pitch_fg_rate_scale(ch_id, self.pitch_fg_rate_scale(midi_ch));
        }
        self.active_ids = ids;

        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { channel, note, velocity, .. } if velocity > 0.0 => {
                    let freq = 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);
                    let velocity_u8 = (velocity * 127.0).round() as u8;
                    let ch_id = midi_channel_note_id(channel, note);
                    let midi_ch = channel as usize;
                    let bit = 1u128 << note;
                    // 弾き直したらペダル保留を解除する（`ym38x6-vst`と同一理由）。
                    self.pedals[midi_ch].note_on(note);
                    // このMIDIチャンネルのProgram Change（CC102/CLAP）パッチを優先。なければDAW値。
                    let mut note_on_patch = self.program_patch[midi_ch].unwrap_or_else(|| self.build_patch());
                    // CC78(Delay)は`TimeEg::note_on()`が読む最初のtickから効かせる必要があるため、
                    // 伝播ループでの1ブロック遅れの反映を待たずここで適用する（CC1/77も併せて統一適用）。
                    apply_pitch_fg_expression(
                        &mut note_on_patch,
                        self.pitch_fg_cc1[midi_ch],
                        self.pitch_fg_cc77[midi_ch],
                        self.pitch_fg_cc78[midi_ch],
                        self.pitch_fg_rpn0_5,
                    );
                    if self.pedals[midi_ch].soft_notes & bit != 0 {
                        apply_soft_pedal(&mut note_on_patch, self.pedals[midi_ch].cc67);
                    }
                    self.engine.set_patch(note_on_patch);
                    self.engine.note_on(ch_id, freq, velocity_u8);
                    self.engine.set_pitch_bend(ch_id, self.channel_bend_cents[midi_ch]);
                    self.engine.set_channel_volume(
                        ch_id,
                        channel_gain(self.cc7[midi_ch], self.cc11[midi_ch]),
                    );
                    self.engine.set_pitch_fg_rate_scale(ch_id, self.pitch_fg_rate_scale(midi_ch));
                    for (op_index, &f_number) in self.operator_f_number_override.iter().enumerate() {
                        self.engine.set_operator_f_number(ch_id, op_index, f_number);
                    }
                }
                NoteEvent::NoteOn { channel, note, .. } | NoteEvent::NoteOff { channel, note, .. } => {
                    let midi_ch = channel as usize;
                    self.poly_pressure[midi_ch][note as usize] = 0;
                    if self.pedals[midi_ch].note_off(note) {
                        self.engine.note_off(midi_channel_note_id(channel, note));
                    }
                }
                NoteEvent::MidiPitchBend { channel, value, .. } => {
                    let cents = (value - 0.5) * 2.0 * self.pitch_bend_range * 100.0;
                    self.channel_bend_cents[channel as usize] = cents;
                    self.engine.set_pitch_bend_group(channel as usize, cents);
                }
                NoteEvent::MidiChannelPressure { channel, pressure, .. } => {
                    self.channel_pressure[channel as usize] = cc_to_u8(pressure);
                }
                NoteEvent::PolyPressure { channel, note, pressure, .. } => {
                    self.poly_pressure[channel as usize][note as usize] = cc_to_u8(pressure);
                }
                NoteEvent::MidiProgramChange { program, channel, .. } => {
                    let bank = (self.bank_select_msb[channel as usize] as u16) * 128
                        + self.bank_select_lsb[channel as usize] as u16;
                    self.program_patch[channel as usize] =
                        self.preset_bank.get(bank, program).map(|preset| preset.patch);
                }
                NoteEvent::MidiCC { cc, value, channel, .. } => {
                    let midi_ch = channel as usize;
                    match cc {
                        7 => {
                            self.cc7[midi_ch] = cc_to_u7(value);
                            self.engine.set_channel_volume_group(
                                midi_ch,
                                channel_gain(self.cc7[midi_ch], self.cc11[midi_ch]),
                            );
                        }
                        11 => {
                            self.cc11[midi_ch] = cc_to_u7(value);
                            self.engine.set_channel_volume_group(
                                midi_ch,
                                channel_gain(self.cc7[midi_ch], self.cc11[midi_ch]),
                            );
                        }
                        // CC1(モジュレーションホイール)：Pitch FG Depthへの瞬間加算（セント換算、
                        // build_patch()参照）。質感LFOは焼き込み専用のためCC補正を受けない。
                        1 => {
                            self.pitch_fg_cc1[midi_ch] = cc_to_u7(value);
                        }
                        // CC2(ブレス)：Expression Destination（NRPN(0,34)）への加算。既定TLキャリア一括。
                        2 => {
                            self.cc2[midi_ch] = cc_to_u8(value);
                        }
                        // CC4(フット)：Expression Destination（NRPN(0,35)）への加算。既定Filter Cutoff＝手動ワウ。
                        4 => {
                            self.cc4[midi_ch] = cc_to_u8(value);
                        }
                        // CC76(Vibrato Rate)：Pitch FGの速さスケール（64=無補正、rate_scale経由）。
                        76 => {
                            self.pitch_fg_cc76[midi_ch] = cc_to_u7(value);
                        }
                        // CC77(Vibrato Depth)：Pitch FG Depthへの0起点パート加算。
                        77 => {
                            self.pitch_fg_cc77[midi_ch] = cc_to_u8(value);
                        }
                        // CC78(Vibrato Delay)：Pitch FG第0段(level=0のとき)のtimeへの64中心相対補正。
                        78 => {
                            self.pitch_fg_cc78[midi_ch] = cc_to_u7(value);
                        }
                        // CC64(サステインペダル)：ホールドフラグ方式（`ym38x6-vst`と同一設計）。
                        64 => {
                            let released = self.pedals[midi_ch].cc64(cc_to_u7(value));
                            for note in released_notes(released) {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                        }
                        // CC66(Sostenuto)：ON時点でkeys_down中のノートのみをlatchし、CC66 OFF
                        // （かつCC64も踏まれていない）までReleaseに入らせない。
                        66 => {
                            let released = self.pedals[midi_ch].cc66(cc_to_u7(value));
                            for note in released_notes(released) {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                        }
                        // CC67(Soft Pedal)：深さを保持するのみ。ON中に新規キーオンしたノートのみ
                        // への適用はNoteOn/live伝播ループ側（soft_notesビット）で行う。
                        67 => {
                            self.pedals[midi_ch].cc67(cc_to_u7(value));
                        }
                        // CC121(Reset All Controllers)：③ジェスチャー層のみリセットする
                        // （②パート状態・①音色は保持、spec-sound.md「補強規則」）。CC64/66/67ペダル・
                        // Pitch Bend・CC1・アフタータッチが対象。CC2/CC4/CC7/CC11/CC76〜78/センド/
                        // RPN等は保持。
                        121 => {
                            let released = self.pedals[midi_ch].cc121();
                            for note in released_notes(released) {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                            self.channel_bend_cents[midi_ch] = 0.0;
                            self.engine.set_pitch_bend_group(midi_ch, 0.0);
                            self.pitch_fg_cc1[midi_ch] = 0;
                            self.channel_pressure[midi_ch] = 0;
                            self.poly_pressure[midi_ch] = [0; 128];
                        }
                        // Bank Select（CC0=MSB, CC32=LSB）：MIDIチャンネルごとに管理
                        0 => self.bank_select_msb[midi_ch] = cc_to_u7(value),
                        32 => self.bank_select_lsb[midi_ch] = cc_to_u7(value),
                        98 => self.rpn.set_nrpn_lsb(cc_to_u7(value)),
                        99 => self.rpn.set_nrpn_msb(cc_to_u7(value)),
                        100 => self.rpn.set_rpn_lsb(cc_to_u7(value)),
                        101 => self.rpn.set_rpn_msb(cc_to_u7(value)),
                        6 => self.handle_data_entry(value),
                        38 => {
                            self.data_entry_lsb = cc_to_u7(value);
                            if let ControlTarget::OperatorFNumber(op_index) = control_target(self.rpn.selection) {
                                self.apply_operator_f_number_override(op_index as usize);
                            }
                        }
                        91 => self.effects.set_reverb_send(cc_to_u8(value)),
                        93 => self.effects.set_chorus_send(cc_to_u8(value)),
                        // CC102: Program Change 代替（VST3ではMidiProgramChangeが届かないため、
                        // `ym38x6-vst`と同じくGM2未定義ブロック先頭のCC102で代替する）。
                        102 => {
                            let prog = cc_to_u7(value);
                            let bank = (self.bank_select_msb[midi_ch] as u16) * 128
                                + self.bank_select_lsb[midi_ch] as u16;
                            self.program_patch[midi_ch] =
                                self.preset_bank.get(bank, prog).map(|preset| preset.patch);
                        }
                        // Operator Key On/Off（CC103〜106、≧64でキーオン/<64でキーオフ、spec-sound.md参照）。
                        // 発音中チャンネルのうち、このMIDIチャンネルに属するものだけへ適用する。
                        103..=106 => {
                            let op_index = (cc - 103) as usize;
                            let key_on = cc_to_u7(value) >= 64;
                            let mut ids = std::mem::take(&mut self.active_ids);
                            self.engine.collect_active_channels(&mut ids);
                            for &ch_id in ids.iter() {
                                if ch_id >> 7 != midi_ch {
                                    continue;
                                }
                                if key_on {
                                    self.engine.note_on_operator(ch_id, op_index);
                                } else {
                                    self.engine.note_off_operator(ch_id, op_index);
                                }
                            }
                            self.active_ids = ids;
                        }
                        // CC120(All Sound Off)：リリースを経ず即座に消音する（GM2準拠、CC123の
                        // リリースとは区別する）。`silence_group`はnote_offのReleaseを経ないため
                        // 残響も無い。
                        120 => {
                            self.engine.silence_group(midi_ch);
                            self.pedals[midi_ch].cc120_reset();
                        }
                        // CC123(All Notes Off)：通常のNote-Off相当（リリースして自然減衰）。
                        123 => {
                            for note in 0u8..MIDI_NOTE_COUNT {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                            self.pedals[midi_ch].cc123_reset();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        let num_channels = buffer.channels();
        let num_samples = buffer.samples();
        let interleaved_len = num_samples * num_channels;

        if interleaved_len > self.render_buffer.len() {
            self.render_buffer.resize(interleaved_len, 0.0);
        }
        let buf = &mut self.render_buffer[..interleaved_len];
        buf.fill(0.0);
        self.engine.render(buf, num_channels);
        self.effects.process(buf, num_channels);

        let output_slices = buffer.as_slice();
        for ch in 0..num_channels {
            for s in 0..num_samples {
                output_slices[ch][s] += buf[s * num_channels + ch];
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Op505Plugin {
    const CLAP_ID: &'static str = "com.ym38x6.op505";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("OP505 FM Synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Synthesizer];
}

impl Vst3Plugin for Op505Plugin {
    const VST3_CLASS_ID: [u8; 16] = *b"Op505---FM4-----";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(Op505Plugin);
nice_export_vst3!(Op505Plugin);
