mod editor;
mod param_adapter;
mod params;

use params::{Op505VstParams, DEFAULT_REVERB_TIME, DEFAULT_REVERB_TYPE};

use nice_plug::prelude::*;
use nice_plug_egui::EguiState;
use op505_core::{op505_presets_dir, Op505ChannelParams, Op505Engine, Op505OperatorParams, Op505Patch, Op505PresetBank};
use sound_core::{AudioProcessor, ChorusType, MasterEffects, ReverbType, TextureLfo, Vco};
use std::sync::Arc;

use crate::params::Op505EgBank;

/// MIDIノート番号の総数（0〜127）。MIDIノート番号をそのままチャンネルIDとして使うため
/// （1ノート=1チャンネル）、発音中チャンネルを走査するループの上限に使う
/// （`ym38x6-vst`と同じ設計）。
const MIDI_NOTE_COUNT: u8 = 128;

/// MIDI CC値（0.0〜1.0正規化）を本プロジェクトの内部表現（0〜255）に変換。
#[inline]
fn cc_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// MIDI CC値（0.0〜1.0正規化）をGM2準拠の7bit値（0〜127）に変換。
#[inline]
fn cc_to_u7(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 127.0).round() as u8
}

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

    // Reverb Type/Timeのみ変化検知が必要（`MasterEffects::set_reverb_type`は内部で
    // `build_algorithm()`を呼び毎回ディレイラインを再構築する。`set_reverb_time`もReverb
    // AlgorithmがDelay系のときは同様に再構築する。値が変わっていないのに毎ブロック呼ぶと
    // オーディオスレッドで無駄な再構築が走り続け、残響が常にリセットされて実質無効化される）。
    // 他のマスターエフェクトパラメーター（Chorus系・Send系）のsetterは単純なフィールド代入
    // のみで安全なため、毎ブロック無条件に呼んでよい。
    last_reverb_type: u8,
    last_reverb_time: u8,

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
    /// ピッチベンド感度（半音）。フェーズ1はRPN(0,0)未実装のため固定2半音
    /// （フェーズ2でRPN対応時に可変化する）。
    pitch_bend_range: f32,

    // CC7/CC11 チャンネル音量（GM2準拠、`ym38x6-vst`と同一設計）。
    cc7: [u8; 16],
    cc11: [u8; 16],

    // サステインペダル（CC64、ホールドフラグ方式。`ym38x6-vst`と同一設計、
    // CC66/CC67はフェーズ2）。
    keys_down: [u128; 16],
    pedal_down: [bool; 16],
    pending_release: [u128; 16],

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
            last_reverb_type: DEFAULT_REVERB_TYPE,
            last_reverb_time: DEFAULT_REVERB_TIME,
            bank_select_msb: [0; 16],
            bank_select_lsb: [0; 16],
            program_patch: [None; 16],
            channel_bend_cents: [0.0; 16],
            pitch_bend_range: 2.0,
            cc7: [127; 16],
            cc11: [127; 16],
            keys_down: [0; 16],
            pedal_down: [false; 16],
            pending_release: [0; 16],
            preset_bank: Op505PresetBank::default(),
            egui_state: EguiState::from_size(1200, 680),
        }
    }
}

impl Op505Plugin {
    /// 現在のDAWパラメーターと`cached_egs`から`Op505Patch`を構築する。
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
                waveform: op.waveform.value() as u8,
                op_fine_tune: op.op_fine_tune.value() as u8,
                eg_shift: op.eg_shift.value() as u8,
                level_scale: op.level_scale.value() as u8,
                velocity_gain: op.velocity_gain.value() as u8,
            }
        });

        let channel = Op505ChannelParams {
            algorithm: p.algorithm.value() as u8,
            feedback: p.feedback.value() as u8,
            chip_lfo_freq: p.chip_lfo_freq.value() as u8,
            chip_lfo_pmd: p.chip_lfo_pmd.value() as u8,
            chip_lfo_amd: p.chip_lfo_amd.value() as u8,
            chip_lfo_delay: p.chip_lfo_delay.value() as u8,
            pms: p.pms.value() as u8,
            ams: p.ams.value() as u8,
            filter_cutoff: p.cutoff.value() as u8,
            filter_resonance: p.resonance.value() as u8,
            filter_type: p.filter_type.value() as u8,
            filter_self_oscillation: p.filter_self_oscillation.value(),
            pitch_fg: op505_core::Op505BipolarFg { eg: egs.pitch_fg, depth: p.pitch_fg_depth.value() as u8 },
            cutoff_fg: op505_core::Op505BipolarFg { eg: egs.cutoff_fg, depth: p.cutoff_fg_depth.value() as u8 },
            gain_fg: egs.gain_fg,
            texture_lfo: TextureLfo {
                waveform: p.texture_lfo_waveform.value() as u8,
                destination: p.texture_lfo_destination.value() as u8,
                rate: p.texture_lfo_rate.value() as u8,
                depth: p.texture_lfo_depth.value() as u8,
                delay: p.texture_lfo_delay.value() as u8,
                fade_mode: p.texture_lfo_fade_mode.value() as u8,
                fade_time: p.texture_lfo_fade_time.value() as u8,
                offset: p.texture_lfo_offset.value() as u8,
            },
        };

        Op505Patch { operators, channel }
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
        self.last_reverb_type = DEFAULT_REVERB_TYPE;
        self.last_reverb_time = DEFAULT_REVERB_TIME;
        self.bank_select_msb = [0; 16];
        self.bank_select_lsb = [0; 16];
        self.program_patch = [None; 16];
        self.channel_bend_cents = [0.0; 16];
        self.cc7 = [127; 16];
        self.cc11 = [127; 16];
        self.keys_down = [0; 16];
        self.pedal_down = [false; 16];
        self.pending_release = [0; 16];
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

        let channel_patch = self.build_patch();

        // 発音中チャンネルへDAWオートメーションの変更を反映する
        // （MIDIノート番号をそのままチャンネルIDとして使うため0〜127を走査する。
        // 非発音チャンネルへのset_*はno-opになる）。
        for note in 0u8..MIDI_NOTE_COUNT {
            let ch_id = note as usize;
            self.engine.set_channel_params(ch_id, channel_patch.channel);
            for (op_index, op) in channel_patch.operators.iter().enumerate() {
                self.engine.set_operator_params(ch_id, op_index, *op);
            }
        }

        // マスターエフェクト：Reverb Type/Timeのみ変化検知（理由はフィールドコメント参照）、
        // 他は毎ブロック無条件に反映する。
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
        self.effects.set_reverb_send(self.params.rev_send.value() as u8);
        self.effects.set_chorus_send(self.params.cho_send.value() as u8);
        self.effects.set_chorus_type(ChorusType::from_u8(self.params.chorus_type.value() as u8));
        self.effects.set_chorus_mod_rate(self.params.chorus_mod_rate.value() as u8);
        self.effects.set_chorus_mod_depth(self.params.chorus_mod_depth.value() as u8);
        self.effects.set_chorus_feedback(self.params.chorus_feedback.value() as u8);
        self.effects.set_chorus_send_to_reverb(self.params.chorus_send_to_reverb.value() as u8);

        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { channel, note, velocity, .. } if velocity > 0.0 => {
                    let freq = 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);
                    let velocity_u8 = (velocity * 127.0).round() as u8;
                    let ch_id = midi_channel_note_id(channel, note);
                    let bit = 1u128 << note;
                    self.pending_release[channel as usize] &= !bit;
                    self.keys_down[channel as usize] |= bit;
                    let note_on_patch = self.program_patch[channel as usize].unwrap_or(channel_patch);
                    self.engine.set_patch(note_on_patch);
                    self.engine.note_on(ch_id, freq, velocity_u8);
                    self.engine.set_pitch_bend(ch_id, self.channel_bend_cents[channel as usize]);
                    self.engine.set_channel_volume(
                        ch_id,
                        channel_gain(self.cc7[channel as usize], self.cc11[channel as usize]),
                    );
                }
                NoteEvent::NoteOn { channel, note, .. } | NoteEvent::NoteOff { channel, note, .. } => {
                    let bit = 1u128 << note;
                    self.keys_down[channel as usize] &= !bit;
                    if self.pedal_down[channel as usize] {
                        self.pending_release[channel as usize] |= bit;
                    } else {
                        self.engine.note_off(midi_channel_note_id(channel, note));
                    }
                }
                NoteEvent::MidiPitchBend { channel, value, .. } => {
                    let cents = (value - 0.5) * 2.0 * self.pitch_bend_range * 100.0;
                    self.channel_bend_cents[channel as usize] = cents;
                    self.engine.set_pitch_bend_group(channel as usize, cents);
                }
                NoteEvent::MidiProgramChange { program, channel, .. } => {
                    let bank = (self.bank_select_msb[channel as usize] as u16) * 128
                        + self.bank_select_lsb[channel as usize] as u16;
                    self.program_patch[channel as usize] =
                        self.preset_bank.get(bank, program).map(|preset| preset.patch);
                }
                NoteEvent::MidiCC { cc, value, channel, .. } => match cc {
                    7 => {
                        self.cc7[channel as usize] = cc_to_u7(value);
                        self.engine.set_channel_volume_group(
                            channel as usize,
                            channel_gain(self.cc7[channel as usize], self.cc11[channel as usize]),
                        );
                    }
                    11 => {
                        self.cc11[channel as usize] = cc_to_u7(value);
                        self.engine.set_channel_volume_group(
                            channel as usize,
                            channel_gain(self.cc7[channel as usize], self.cc11[channel as usize]),
                        );
                    }
                    // CC64(サステインペダル)：ホールドフラグ方式（`ym38x6-vst`と同一設計、
                    // フェーズ1はCC66/CC67未対応のためother_held/soft_pedal等は無し）。
                    64 => {
                        let ch = channel as usize;
                        if cc_to_u7(value) >= 64 {
                            self.pedal_down[ch] = true;
                        } else {
                            self.pedal_down[ch] = false;
                            let mut mask = self.pending_release[ch];
                            while mask != 0 {
                                let note = mask.trailing_zeros() as u8;
                                let bit = 1u128 << note;
                                mask &= mask - 1;
                                if self.keys_down[ch] & bit == 0 {
                                    self.engine.note_off(midi_channel_note_id(channel, note));
                                    self.pending_release[ch] &= !bit;
                                }
                            }
                        }
                    }
                    0 => self.bank_select_msb[channel as usize] = cc_to_u7(value),
                    32 => self.bank_select_lsb[channel as usize] = cc_to_u7(value),
                    91 => self.effects.set_reverb_send(cc_to_u8(value)),
                    93 => self.effects.set_chorus_send(cc_to_u8(value)),
                    // CC102: Program Change 代替（VST3ではMidiProgramChangeが届かないため、
                    // `ym38x6-vst`と同じくGM2未定義ブロック先頭のCC102で代替する）。
                    102 => {
                        let prog = cc_to_u7(value);
                        let bank = (self.bank_select_msb[channel as usize] as u16) * 128
                            + self.bank_select_lsb[channel as usize] as u16;
                        self.program_patch[channel as usize] =
                            self.preset_bank.get(bank, prog).map(|preset| preset.patch);
                    }
                    _ => {}
                },
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
