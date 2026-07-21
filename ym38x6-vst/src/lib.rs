mod editor;
mod midi;
mod param_adapter;
mod params;

use midi::{
    apply_expression_modulation, apply_soft_pedal, cc_to_u8, cc_to_u7, ExpressionDestination,
    RpnSelection,
};
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
    cc76_to_rate_scale, lfo_fade_mode_from_index, presets_dir, AudioProcessor, BipolarFg,
    ChannelParams, ChorusType, EgParams, LfoFadeMode, MasterEffects, OperatorParams, PresetBank,
    ReverbType, TextureLfo, Vco, Ym38x6Engine, Ym38x6LfoDestination, Ym38x6Patch,
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

    // 質感LFO（NRPN(0,0)/(0,1)/(0,22)〜(0,27)）の状態。全項目が①音色/NRPN専用で、
    // 演奏系CC(1/76/77/78)による補正は受けない（焼き込み専用、spec-sound.md参照）。
    // NRPN直書き込み＋DAWパラメーターとの1シャドウ差分検知（algorithmと同型）で管理する
    // （旧effective_lfo_*の2シャドウは、CC76/77/78がPitch FGへ移ったことで不要になった＝
    // これが「shadow/effectiveの二重ソースを層分離で解消」の実体）。
    texture_lfo_destination: Ym38x6LfoDestination, // NRPN(0,0)
    texture_lfo_waveform: u8,                       // NRPN(0,1)、0〜4
    texture_lfo_fade_mode: LfoFadeMode,             // NRPN(0,22)
    texture_lfo_rate: u8,                           // NRPN(0,23)
    texture_lfo_depth: u8,                          // NRPN(0,24)
    texture_lfo_delay: u8,                          // NRPN(0,25)
    texture_lfo_fade_time: u8,                      // NRPN(0,26)
    texture_lfo_offset: u8,                         // NRPN(0,27)
    last_texture_lfo_waveform_param: u8,
    last_texture_lfo_fade_mode_param: u8,
    last_texture_lfo_rate_param: u8,
    last_texture_lfo_depth_param: u8,
    last_texture_lfo_delay_param: u8,
    last_texture_lfo_fade_time_param: u8,
    last_texture_lfo_offset_param: u8,

    // Pitch FG（②③層の補正を受ける唯一のFGスロット、spec-sound.md「演奏層による補正」節）。
    // CC1/76/77/78・RPN0,5の生値を保持し、build_patch()で毎ブロックPitch FGの
    // AR/D1R/Delay/Depthへ計算適用する（シャドー不要、常に最新のCC状態から再計算するため）。
    pitch_fg_cc1: u8,             // CC1 Modulation Wheel（0〜127、Depthへ瞬間加算・セント換算）
    pitch_fg_cc76: u8,            // CC76 Vibrato Rate（0〜127、64=無補正、AR/D1Rの速さスケール）
    pitch_fg_cc77_depth_add: u8,  // CC77 Vibrato Depth（0〜255、Depthへ0起点加算）
    pitch_fg_cc78: u8,            // CC78 Vibrato Delay（0〜127、64=無補正、Delayへ加算）
    pitch_fg_rpn0_5: u8,          // RPN0,5 Modulation Depth Range（GM2準拠0〜127、デフォルト64）

    // Pitch/Cutoff/Gain FGのLoop/Curve（NRPN(0,28)〜(0,33)+DAWパラメーター、算法同型の1シャドウ）。
    pitch_fg_loop: bool,
    pitch_fg_curve: bool,
    cutoff_fg_loop: bool,
    cutoff_fg_curve: bool,
    gain_fg_loop: bool,
    gain_fg_curve: bool,
    last_pitch_fg_loop_param: bool,
    last_pitch_fg_curve_param: bool,
    last_cutoff_fg_loop_param: bool,
    last_cutoff_fg_curve_param: bool,
    last_gain_fg_loop_param: bool,
    last_gain_fg_curve_param: bool,

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
    at_destination: ExpressionDestination,
    poly_at_destination: ExpressionDestination,
    channel_pressure: u8,
    poly_pressure: HashMap<u8, u8>, // MIDIノート番号 → Poly Key Pressure

    // CC2(ブレス)/CC4(フット)の加算先（NRPN(0,34)/(0,35)）と現在値。AT同様グローバル単一シャドウ
    // （既存のapply_expression_modulation経路がグローバル単一パッチ前提のため揃える）。
    // 既定行先はCC2→TLキャリア一括（ウインド楽器風の明るさ/音量スウェル）、
    // CC4→Filter Cutoff（古典的ワウペダル＝手動ワウ）。
    cc2: u8,
    cc4: u8,
    cc2_destination: ExpressionDestination,
    cc4_destination: ExpressionDestination,

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

    // ペダル（CC64 Sustain / CC66 Sostenuto / CC67 Soft、ホールドフラグ方式。
    // spec-sound.md「サステインペダル（CC64）の実装方針」参照）。エンジン無改造・
    // 1ノート=1チャンネルのまま、MIDIチャンネルごとに管理する。
    /// 物理的に押下中の鍵（bit N = ノート番号N。NoteOnでセット、NoteOffでクリア）。
    keys_down: [u128; 16],
    pedal_down: [bool; 16],
    /// ペダル（CC64またはCC66）保持中に保留したNote Off（bit N = ノート番号Nが保留中）。
    pending_release: [u128; 16],
    /// CC66 ON時点でkeys_downをスナップショットしたノート（bit N = ノート番号N）。CC66 OFFで解除。
    sostenuto: [u128; 16],
    /// Soft Pedal（CC67）の深さ（0〜127、0=無効）。
    cc67: [u8; 16],
    /// cc67>0の間にNote-Onしたノート（bit N = ノート番号N）。実効TL/Cutoff減算の対象。
    soft_notes: [u128; 16],

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
            texture_lfo_destination: Ym38x6LfoDestination::Pitch,
            texture_lfo_waveform: 0,
            texture_lfo_fade_mode: LfoFadeMode::default(),
            texture_lfo_rate: 0,
            texture_lfo_depth: 0,
            texture_lfo_delay: 0,
            texture_lfo_fade_time: 0,
            texture_lfo_offset: 128,
            last_texture_lfo_waveform_param: 0,
            last_texture_lfo_fade_mode_param: 0,
            last_texture_lfo_rate_param: 0,
            last_texture_lfo_depth_param: 0,
            last_texture_lfo_delay_param: 0,
            last_texture_lfo_fade_time_param: 0,
            last_texture_lfo_offset_param: 0,
            pitch_fg_cc1: 0,
            pitch_fg_cc76: 64,
            pitch_fg_cc77_depth_add: 0,
            pitch_fg_cc78: 64,
            pitch_fg_rpn0_5: 64,
            pitch_fg_loop: false,
            pitch_fg_curve: false,
            cutoff_fg_loop: false,
            cutoff_fg_curve: false,
            gain_fg_loop: false,
            gain_fg_curve: false,
            last_pitch_fg_loop_param: false,
            last_pitch_fg_curve_param: false,
            last_cutoff_fg_loop_param: false,
            last_cutoff_fg_curve_param: false,
            last_gain_fg_loop_param: false,
            last_gain_fg_curve_param: false,
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
            at_destination: ExpressionDestination::default(),
            poly_at_destination: ExpressionDestination::default(),
            channel_pressure: 0,
            poly_pressure: HashMap::new(),
            cc2: 0,
            cc4: 0,
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
            cc7:  [127; 16],
            cc11: [127; 16],
            keys_down: [0; 16],
            pedal_down: [false; 16],
            pending_release: [0; 16],
            sostenuto: [0; 16],
            cc67: [0; 16],
            soft_notes: [0; 16],
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
                floor: op.floor.value() as u8,
                loop_enabled: op.op_loop.value() as u8,
                curve: op.curve.value() as u8,
                eg_shift: op.eg_shift.value() as u8,
                level_scale: op.level_scale.value() as u8,
                velocity_gain: op.velocity_gain.value() as u8,
            }
        });

        // Pitch FG: ②パート状態(CC76/77/78)・③ジェスチャー(CC1)の補正を毎ブロック計算し直す
        // （spec-sound.md「演奏層による補正」節。実効Depth=①パッチ基準値+②③の加算）。
        // CC76(Rate)はAR/D1Rの生値ではなくrate_scale経由（process()のset_pitch_fg_rate_scale参照、
        // 「一括スケール」という語義通りの乗算的な速さ変更にするため、詳細はプラン参照）。
        let delay_delta = self.pitch_fg_cc78 as i32 - 64;
        let effective_pitch_fg_delay = (p.pitch_fg_delay.value() + delay_delta).clamp(0, 255) as u8;
        // CC1のセント換算分をDepthと同じ0〜255単位空間へ逆変換して加算する
        // （Pitch FGの`(depth-128)/128*1200`セント変換式の逆算、cc1_cents = cc1/127 * rpn0_5*50/64）。
        let cc1_cents = (self.pitch_fg_cc1 as f32 / 127.0) * (self.pitch_fg_rpn0_5 as f32 * 50.0 / 64.0);
        let cc1_depth_units = (cc1_cents / 1200.0 * 128.0).round() as i32;
        let effective_pitch_fg_depth = (p.pitch_fg_depth.value()
            + self.pitch_fg_cc77_depth_add as i32
            + cc1_depth_units)
            .clamp(0, 255) as u8;

        let channel = ChannelParams {
            algorithm: self.algorithm,
            feedback: p.feedback.value() as u8,
            chip_lfo_freq: p.chip_lfo_freq.value() as u8,
            chip_lfo_pmd: p.chip_lfo_pmd.value() as u8,
            chip_lfo_amd: p.chip_lfo_amd.value() as u8,
            chip_lfo_delay: p.chip_lfo_delay.value() as u8,
            pms: p.pms.value() as u8,
            ams: p.ams.value() as u8,
            filter_cutoff: p.cutoff.value() as u8,
            filter_resonance: p.resonance.value() as u8,
            filter_type: self.filter_type,
            filter_self_oscillation: self.filter_self_oscillation,
            pitch_fg: BipolarFg {
                eg: EgParams {
                    ar: p.pitch_fg_ar.value() as u8,
                    d1r: p.pitch_fg_d1r.value() as u8,
                    d1l: p.pitch_fg_d1l.value() as u8,
                    d2r: p.pitch_fg_d2r.value() as u8,
                    rr: p.pitch_fg_rr.value() as u8,
                    floor: p.pitch_fg_floor.value() as u8,
                    loop_enabled: self.pitch_fg_loop as u8,
                    curve: self.pitch_fg_curve as u8,
                    delay: effective_pitch_fg_delay,
                },
                depth: effective_pitch_fg_depth,
            },
            cutoff_fg: BipolarFg {
                eg: EgParams {
                    ar: p.cutoff_fg_ar.value() as u8,
                    d1r: p.cutoff_fg_d1r.value() as u8,
                    d1l: p.cutoff_fg_d1l.value() as u8,
                    d2r: p.cutoff_fg_d2r.value() as u8,
                    rr: p.cutoff_fg_rr.value() as u8,
                    floor: p.cutoff_fg_floor.value() as u8,
                    loop_enabled: self.cutoff_fg_loop as u8,
                    curve: self.cutoff_fg_curve as u8,
                    delay: p.cutoff_fg_delay.value() as u8,
                },
                // 新bipolar Depth(中心128)をコアへ直接コピーする（旧unipolarとの変換式は撤去済み）。
                depth: p.cutoff_fg_depth.value() as u8,
            },
            gain_fg: EgParams {
                ar: p.gain_fg_ar.value() as u8,
                d1r: p.gain_fg_d1r.value() as u8,
                d1l: p.gain_fg_d1l.value() as u8,
                d2r: p.gain_fg_d2r.value() as u8,
                rr: p.gain_fg_rr.value() as u8,
                floor: p.gain_fg_floor.value() as u8,
                loop_enabled: self.gain_fg_loop as u8,
                curve: self.gain_fg_curve as u8,
                delay: p.gain_fg_delay.value() as u8,
            },
            // 質感LFOは焼き込み専用のためCC補正を受けない（NRPN/DAWパラメーターのみ、
            // 演奏系CC1/76/77/78はすべてPitch FGへ行く）。
            texture_lfo: TextureLfo {
                waveform: self.texture_lfo_waveform,
                destination: self.texture_lfo_destination as u8,
                rate: self.texture_lfo_rate,
                depth: self.texture_lfo_depth,
                delay: self.texture_lfo_delay,
                fade_mode: self.texture_lfo_fade_mode as u8,
                fade_time: self.texture_lfo_fade_time,
                offset: self.texture_lfo_offset,
            },
        };

        Ym38x6Patch { operators, channel }
    }

    /// `candidates & pending_release[channel]` のうち、`other_held`（他のペダルが保持中の
    /// ノート）にも`keys_down`（再押下中のノート）にも該当しないものをnote_offして
    /// `pending_release`/`soft_notes`から除去する。CC64/CC66 OFF・CC121の解放処理で共有する
    /// （CC64 OFF: candidates=pending_release, other_held=sostenuto。
    /// 　CC66 OFF: candidates=sostenuto, other_held=pedal_down相当の全ビット。
    /// 　CC121: candidates=pending_release, other_held=0）。smf2wav `release_unheld`の複製。
    fn release_unheld(&mut self, channel: u8, candidates: u128, other_held: u128) {
        let ch = channel as usize;
        let mut mask = candidates & self.pending_release[ch];
        while mask != 0 {
            let note = mask.trailing_zeros() as u8;
            let bit = 1u128 << note;
            mask &= mask - 1;
            if other_held & bit == 0 && self.keys_down[ch] & bit == 0 {
                self.engine.note_off(midi_channel_note_id(channel, note));
                self.pending_release[ch] &= !bit;
                self.soft_notes[ch] &= !bit;
            }
        }
    }

    /// CC76(Vibrato Rate)由来のPitch FG速さスケールを計算する（`cc76_to_rate_scale`、
    /// 64=1.0倍=無補正）。build_patch()とは別に、`engine.set_pitch_fg_rate_scale`で
    /// 直接エンジンへ渡す（ChannelParamsを経由しない、pitch_bend/channel_volumeと同じ経路）。
    fn pitch_fg_rate_scale(&self) -> f32 {
        cc76_to_rate_scale(self.pitch_fg_cc76)
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
            // RPN0,5: Modulation Depth Range（Pitch FGのCC1セント換算係数、64≈50セント）。
            RpnSelection::Rpn(0, 5) => {
                self.pitch_fg_rpn0_5 = cc_to_u7(value);
            }
            // NRPN(0,0): 質感LFO Destination（38x6拡張：2=TLキャリア一括、3=Cutoff/オートワウ）。
            // build_patch()が毎ブロックtexture_lfo.destinationへ詰め替え、set_channel_paramsの
            // 定期伝播に乗るため、明示的な即時反映呼び出しは不要。
            RpnSelection::Nrpn(0, 0) => {
                self.texture_lfo_destination = match cc_to_u7(value) {
                    0 => Ym38x6LfoDestination::Pitch,
                    1 => Ym38x6LfoDestination::Volume,
                    2 => Ym38x6LfoDestination::TlCarrier,
                    _ => Ym38x6LfoDestination::Cutoff,
                };
            }
            // NRPN(0,1): 質感LFO Waveform（0〜4、質感LFOの5波形パレットへ直接対応）。
            RpnSelection::Nrpn(0, 1) => {
                self.texture_lfo_waveform = cc_to_u7(value).min(4);
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
                self.at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,17): Poly AT Destination（Poly Key Pressureの加算先）
            RpnSelection::Nrpn(0, 17) => {
                self.poly_at_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,18)〜(0,21): Operator F-Number Op0〜3
            RpnSelection::Nrpn(0, lsb @ 18..=21) => {
                self.apply_operator_f_number_override((lsb - 18) as usize);
            }
            // NRPN(0,22): 質感LFO Fade Mode（0=ON-IN/1=ON-OUT/2=OFF-IN/3=OFF-OUT）。
            RpnSelection::Nrpn(0, 22) => {
                self.texture_lfo_fade_mode = lfo_fade_mode_from_index(cc_to_u7(value));
            }
            // NRPN(0,23)〜(0,27): 質感LFO Rate/Depth/Delay/FadeTime/Offset（焼き込み専用、
            // DAWパラメーターとの1シャドウ差分検知はprocess()側。ここはNRPNからの直接書き込み）。
            RpnSelection::Nrpn(0, 23) => {
                self.texture_lfo_rate = cc_to_u8(value);
            }
            RpnSelection::Nrpn(0, 24) => {
                self.texture_lfo_depth = cc_to_u8(value);
            }
            RpnSelection::Nrpn(0, 25) => {
                self.texture_lfo_delay = cc_to_u8(value);
            }
            RpnSelection::Nrpn(0, 26) => {
                self.texture_lfo_fade_time = cc_to_u8(value);
            }
            RpnSelection::Nrpn(0, 27) => {
                self.texture_lfo_offset = cc_to_u8(value);
            }
            // NRPN(0,28)〜(0,33): Pitch/Cutoff/Gain FG Loop/Curve（0=OFF/1=ON、DAWパラメーターと
            // 両方から設定できる例外的な離散パラメーター。process()側の1シャドウ差分検知と共存）。
            RpnSelection::Nrpn(0, 28) => {
                self.pitch_fg_loop = cc_to_u7(value) != 0;
            }
            RpnSelection::Nrpn(0, 29) => {
                self.pitch_fg_curve = cc_to_u7(value) != 0;
            }
            RpnSelection::Nrpn(0, 30) => {
                self.cutoff_fg_loop = cc_to_u7(value) != 0;
            }
            RpnSelection::Nrpn(0, 31) => {
                self.cutoff_fg_curve = cc_to_u7(value) != 0;
            }
            RpnSelection::Nrpn(0, 32) => {
                self.gain_fg_loop = cc_to_u7(value) != 0;
            }
            RpnSelection::Nrpn(0, 33) => {
                self.gain_fg_curve = cc_to_u7(value) != 0;
            }
            // NRPN(0,34): CC2(ブレス)Destination
            RpnSelection::Nrpn(0, 34) => {
                self.cc2_destination = ExpressionDestination::from_u8(cc_to_u7(value));
            }
            // NRPN(0,35): CC4(フット)Destination。既定FilterCutoff＝手動ワウ。
            RpnSelection::Nrpn(0, 35) => {
                self.cc4_destination = ExpressionDestination::from_u8(cc_to_u7(value));
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
        self.texture_lfo_destination = Ym38x6LfoDestination::Pitch;
        self.texture_lfo_waveform = 0;
        self.texture_lfo_fade_mode = LfoFadeMode::default();
        self.texture_lfo_rate = 0;
        self.texture_lfo_depth = 0;
        self.texture_lfo_delay = 0;
        self.texture_lfo_fade_time = 0;
        self.texture_lfo_offset = 128;
        self.last_texture_lfo_waveform_param = 0;
        self.last_texture_lfo_fade_mode_param = 0;
        self.last_texture_lfo_rate_param = 0;
        self.last_texture_lfo_depth_param = 0;
        self.last_texture_lfo_delay_param = 0;
        self.last_texture_lfo_fade_time_param = 0;
        self.last_texture_lfo_offset_param = 0;
        self.pitch_fg_cc1 = 0;
        self.pitch_fg_cc76 = 64;
        self.pitch_fg_cc77_depth_add = 0;
        self.pitch_fg_cc78 = 64;
        self.pitch_fg_rpn0_5 = 64;
        self.pitch_fg_loop = false;
        self.pitch_fg_curve = false;
        self.cutoff_fg_loop = false;
        self.cutoff_fg_curve = false;
        self.gain_fg_loop = false;
        self.gain_fg_curve = false;
        self.last_pitch_fg_loop_param = false;
        self.last_pitch_fg_curve_param = false;
        self.last_cutoff_fg_loop_param = false;
        self.last_cutoff_fg_curve_param = false;
        self.last_gain_fg_loop_param = false;
        self.last_gain_fg_curve_param = false;
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
        self.at_destination = ExpressionDestination::default();
        self.poly_at_destination = ExpressionDestination::default();
        self.channel_pressure = 0;
        self.poly_pressure.clear();
        self.cc2 = 0;
        self.cc4 = 0;
        self.cc2_destination = ExpressionDestination::TlCarriers;
        self.cc4_destination = ExpressionDestination::FilterCutoff;
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

        // 質感LFO（7個）：algorithmと同じ1シャドウ差分検知方式。NRPN(0,1)/(0,22)〜(0,27)直接
        // 書き込みと共存する（build_patch()が毎ブロック読むため、明示的な即時反映呼び出しは不要）。
        let texture_lfo_waveform_param = self.params.texture_lfo_waveform.value() as u8;
        if texture_lfo_waveform_param != self.last_texture_lfo_waveform_param {
            self.texture_lfo_waveform = texture_lfo_waveform_param;
            self.last_texture_lfo_waveform_param = texture_lfo_waveform_param;
        }
        let texture_lfo_fade_mode_param = self.params.texture_lfo_fade_mode.value() as u8;
        if texture_lfo_fade_mode_param != self.last_texture_lfo_fade_mode_param {
            self.texture_lfo_fade_mode = lfo_fade_mode_from_index(texture_lfo_fade_mode_param);
            self.last_texture_lfo_fade_mode_param = texture_lfo_fade_mode_param;
        }
        let texture_lfo_rate_param = self.params.texture_lfo_rate.value() as u8;
        if texture_lfo_rate_param != self.last_texture_lfo_rate_param {
            self.texture_lfo_rate = texture_lfo_rate_param;
            self.last_texture_lfo_rate_param = texture_lfo_rate_param;
        }
        let texture_lfo_depth_param = self.params.texture_lfo_depth.value() as u8;
        if texture_lfo_depth_param != self.last_texture_lfo_depth_param {
            self.texture_lfo_depth = texture_lfo_depth_param;
            self.last_texture_lfo_depth_param = texture_lfo_depth_param;
        }
        let texture_lfo_delay_param = self.params.texture_lfo_delay.value() as u8;
        if texture_lfo_delay_param != self.last_texture_lfo_delay_param {
            self.texture_lfo_delay = texture_lfo_delay_param;
            self.last_texture_lfo_delay_param = texture_lfo_delay_param;
        }
        let texture_lfo_fade_time_param = self.params.texture_lfo_fade_time.value() as u8;
        if texture_lfo_fade_time_param != self.last_texture_lfo_fade_time_param {
            self.texture_lfo_fade_time = texture_lfo_fade_time_param;
            self.last_texture_lfo_fade_time_param = texture_lfo_fade_time_param;
        }
        let texture_lfo_offset_param = self.params.texture_lfo_offset.value() as u8;
        if texture_lfo_offset_param != self.last_texture_lfo_offset_param {
            self.texture_lfo_offset = texture_lfo_offset_param;
            self.last_texture_lfo_offset_param = texture_lfo_offset_param;
        }

        // Pitch/Cutoff/Gain FG Loop/Curve（6個）：algorithmと同じ1シャドウ差分検知方式。
        // NRPN(0,28)〜(0,33)直接書き込みと共存する。
        let pitch_fg_loop_param = self.params.pitch_fg_loop.value();
        if pitch_fg_loop_param != self.last_pitch_fg_loop_param {
            self.pitch_fg_loop = pitch_fg_loop_param;
            self.last_pitch_fg_loop_param = pitch_fg_loop_param;
        }
        let pitch_fg_curve_param = self.params.pitch_fg_curve.value();
        if pitch_fg_curve_param != self.last_pitch_fg_curve_param {
            self.pitch_fg_curve = pitch_fg_curve_param;
            self.last_pitch_fg_curve_param = pitch_fg_curve_param;
        }
        let cutoff_fg_loop_param = self.params.cutoff_fg_loop.value();
        if cutoff_fg_loop_param != self.last_cutoff_fg_loop_param {
            self.cutoff_fg_loop = cutoff_fg_loop_param;
            self.last_cutoff_fg_loop_param = cutoff_fg_loop_param;
        }
        let cutoff_fg_curve_param = self.params.cutoff_fg_curve.value();
        if cutoff_fg_curve_param != self.last_cutoff_fg_curve_param {
            self.cutoff_fg_curve = cutoff_fg_curve_param;
            self.last_cutoff_fg_curve_param = cutoff_fg_curve_param;
        }
        let gain_fg_loop_param = self.params.gain_fg_loop.value();
        if gain_fg_loop_param != self.last_gain_fg_loop_param {
            self.gain_fg_loop = gain_fg_loop_param;
            self.last_gain_fg_loop_param = gain_fg_loop_param;
        }
        let gain_fg_curve_param = self.params.gain_fg_curve.value();
        if gain_fg_curve_param != self.last_gain_fg_curve_param {
            self.gain_fg_curve = gain_fg_curve_param;
            self.last_gain_fg_curve_param = gain_fg_curve_param;
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

        // 発音中チャンネルへDAWオートメーションの変更とAT/Poly AT Destinationの加算を反映する
        // （MIDIノート番号をそのままチャンネルIDとして使うため0〜127を走査する。
        // 非発音チャンネルへのset_*はno-opになる）
        let pitch_fg_rate_scale = self.pitch_fg_rate_scale();
        for note in 0u8..MIDI_NOTE_COUNT {
            let ch_id = note as usize;
            let mut note_patch = channel_patch;
            apply_expression_modulation(
                note,
                &[
                    (self.cc2, self.cc2_destination),
                    (self.cc4, self.cc4_destination),
                    (self.channel_pressure, self.at_destination),
                ],
                self.poly_at_destination,
                &self.poly_pressure,
                &mut note_patch,
            );
            // Soft Pedal（CC67）: soft_notesビットが立っているノートへ現在のcc67深さを
            // 毎回再適用する（このループがchannel_patchから毎回組み立て直すため、NoteOn時の
            // 減算を上書きで消さないようにする）。このループはMIDIチャンネル0のみを扱う
            // 既存仕様（ch_id=note）のため、soft_notes[0]/cc67[0]を参照する。
            if self.soft_notes[0] & (1u128 << note) != 0 {
                apply_soft_pedal(&mut note_patch, self.cc67[0]);
            }
            self.engine.set_channel_params(ch_id, note_patch.channel);
            for (op_index, op) in note_patch.operators.iter().enumerate() {
                self.engine.set_operator_params(ch_id, op_index, *op);
            }
            // CC76(Vibrato Rate)由来のPitch FG速さスケール（ChannelParamsを経由しない、
            // pitch_bend/channel_volumeと同じ単一ボイス直接setterパターン）。
            self.engine.set_pitch_fg_rate_scale(ch_id, pitch_fg_rate_scale);
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
                    let bit = 1u128 << note;
                    // 弾き直したらペダル保留を解除する（離鍵→ペダルアップ前に再度弾いた場合、
                    // 古い保留ビットが残っていると鍵盤を押している最中にペダルアップでnote_offが
                    // 誤発火する。spec-sound.md「サステインペダル（CC64）の実装方針」参照）。
                    self.pending_release[channel as usize] &= !bit;
                    self.keys_down[channel as usize] |= bit;
                    // Soft Pedal（CC67）: ON中に新規キーオンしたノートのみ対象（spec-sound.md参照）。
                    if self.cc67[channel as usize] > 0 {
                        self.soft_notes[channel as usize] |= bit;
                    } else {
                        self.soft_notes[channel as usize] &= !bit;
                    }
                    // このMIDIチャンネルのProgram Change（CC102/CLAP）パッチを優先。なければGUI値。
                    // ボイス固有パッチを`set_patch`でカレントに置いてから`Vco::note_on`する
                    // （Channelがnote_on時点のカレントパッチをコピー保持するため、後続ボイスと混ざらない）。
                    let mut note_on_patch = self.program_patch[channel as usize].unwrap_or(channel_patch);
                    if self.soft_notes[channel as usize] & bit != 0 {
                        apply_soft_pedal(&mut note_on_patch, self.cc67[channel as usize]);
                    }
                    self.engine.set_patch(note_on_patch);
                    self.engine.note_on(ch_id, freq, velocity_u8);
                    // このMIDIチャンネルの現在のベンド量と音量ゲインを新ボイスへ反映する
                    self.engine.set_pitch_bend(ch_id, self.channel_bend_cents[channel as usize]);
                    self.engine.set_channel_volume(ch_id, channel_gain(self.cc7[channel as usize], self.cc11[channel as usize]));
                    self.engine.set_pitch_fg_rate_scale(ch_id, self.pitch_fg_rate_scale());
                    // texture_lfo（rate/delay/waveform/destination/depth）はnote_on_patchに
                    // 既に含まれておりnote_on内で適用されるため、別途の反映呼び出しは不要。
                    for (op_index, &f_number) in self.operator_f_number_override.iter().enumerate() {
                        self.engine.set_operator_f_number(ch_id, op_index, f_number);
                    }
                }
                NoteEvent::NoteOn { channel, note, .. } | NoteEvent::NoteOff { channel, note, .. } => {
                    // velocity=0 の NoteOn も NoteOff として扱う（MIDI仕様）
                    self.poly_pressure.remove(&note);
                    let bit = 1u128 << note;
                    self.keys_down[channel as usize] &= !bit;
                    let held = self.pedal_down[channel as usize] || self.sostenuto[channel as usize] & bit != 0;
                    if held {
                        // いずれかのペダル（CC64/CC66）保持中: Note Offを保留する（ホールドフラグ方式）
                        self.pending_release[channel as usize] |= bit;
                    } else {
                        self.engine.note_off(midi_channel_note_id(channel, note));
                        self.soft_notes[channel as usize] &= !bit;
                    }
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
                    // CC1(モジュレーションホイール)：Pitch FG Depthへの瞬間加算（セント換算、
                    // build_patch()参照）。質感LFOは焼き込み専用のためCC補正を受けない。
                    1 => {
                        self.pitch_fg_cc1 = cc_to_u7(value);
                    }
                    // CC2(ブレス)：Expression Destination（NRPN(0,34)）への加算。既定TLキャリア一括。
                    2 => {
                        self.cc2 = cc_to_u8(value);
                    }
                    // CC4(フット)：Expression Destination（NRPN(0,35)）への加算。既定Filter Cutoff＝手動ワウ。
                    4 => {
                        self.cc4 = cc_to_u8(value);
                    }
                    // CC76(Vibrato Rate)：Pitch FGの速さスケール（64=無補正、rate_scale経由）。
                    76 => {
                        self.pitch_fg_cc76 = cc_to_u7(value);
                    }
                    // CC77(Vibrato Depth)：Pitch FG Depthへの0起点パート加算。
                    77 => {
                        self.pitch_fg_cc77_depth_add = cc_to_u8(value);
                    }
                    // CC78(Vibrato Delay)：Pitch FG Delayへの64中心相対補正。
                    78 => {
                        self.pitch_fg_cc78 = cc_to_u7(value);
                    }
                    // CC64(サステインペダル)：ホールドフラグ方式（(a)、spec-sound.md参照）。
                    // 64以上でペダルON、64未満でOFF。OFF時は保留していたNote Offのうち、
                    // sostenuto保持中・再押下中でないものだけ送出する（CC66併用時の順序）。
                    64 => {
                        let ch = channel as usize;
                        if cc_to_u7(value) >= 64 {
                            self.pedal_down[ch] = true;
                        } else {
                            self.pedal_down[ch] = false;
                            let candidates = self.pending_release[ch];
                            let other_held = self.sostenuto[ch];
                            self.release_unheld(channel, candidates, other_held);
                        }
                    }
                    // CC66(Sostenuto)：ON時点でkeys_down中のノートのみをlatchし、CC66 OFF
                    // （かつCC64も踏まれていない）までReleaseに入らせない
                    // （spec-sound.md「Sostenuto（CC66）」）。
                    66 => {
                        let ch = channel as usize;
                        if cc_to_u7(value) >= 64 {
                            self.sostenuto[ch] = self.keys_down[ch];
                        } else {
                            let candidates = self.sostenuto[ch];
                            let other_held = if self.pedal_down[ch] { u128::MAX } else { 0 };
                            self.release_unheld(channel, candidates, other_held);
                            self.sostenuto[ch] = 0;
                        }
                    }
                    // CC67(Soft Pedal)：深さを保持するのみ。ON中に新規キーオンしたノートのみ
                    // への適用はNoteOn/live伝播ループ側（soft_notesビット）で行う
                    // （spec-sound.md「Soft Pedal（CC67）」）。
                    67 => {
                        self.cc67[channel as usize] = cc_to_u7(value);
                    }
                    // CC121(Reset All Controllers)：③ジェスチャー層のみリセットする
                    // （②パート状態・①音色は保持、spec-sound.md「補強規則」）。CC64/66/67ペダル・
                    // Pitch Bend・CC1・アフタータッチが対象。CC2/CC4/CC7/CC11/CC76〜78/センド/
                    // RPN等は保持。次ブロックの毎ブロック伝播ループがchannel_patch/ATを
                    // 再適用するため、ここではset_pitch_bend_group以外の即時反映は不要。
                    121 => {
                        let ch = channel as usize;
                        let candidates = self.pending_release[ch];
                        self.release_unheld(channel, candidates, 0);
                        self.pedal_down[ch] = false;
                        self.sostenuto[ch] = 0;
                        self.cc67[ch] = 0;
                        self.soft_notes[ch] = 0;
                        self.channel_bend_cents[ch] = 0.0;
                        self.engine.set_pitch_bend_group(ch, 0.0);
                        self.pitch_fg_cc1 = 0;
                        self.channel_pressure = 0;
                        self.poly_pressure.clear();
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
                    // CC120(All Sound Off)：リリースを経ず即座に消音する（GM2準拠、CC123の
                    // リリースとは区別する）。`silence_group`はnote_offのReleaseを経ないため
                    // 残響も無い。
                    120 => {
                        let ch = channel as usize;
                        self.engine.silence_group(ch);
                        self.keys_down[ch] = 0;
                        self.pending_release[ch] = 0;
                        self.pedal_down[ch] = false;
                        self.sostenuto[ch] = 0;
                        self.soft_notes[ch] = 0;
                    }
                    // CC123(All Notes Off)：通常のNote-Off相当（リリースして自然減衰）。
                    123 => {
                        let ch = channel as usize;
                        for note in 0u8..MIDI_NOTE_COUNT {
                            self.engine.note_off(midi_channel_note_id(channel, note));
                        }
                        self.keys_down[ch] = 0;
                        self.pending_release[ch] = 0;
                        self.sostenuto[ch] = 0;
                        self.soft_notes[ch] = 0;
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
