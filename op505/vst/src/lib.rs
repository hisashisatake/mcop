mod editor;
mod param_adapter;
mod params;
mod preset_host;

use op505_midi::{
    cc_to_u7, cc_to_u8, released_notes, ChannelState, DataEntryOutcome, MonoNoteOff, MonoNoteOn,
    ProgramSelection, RHYTHM_BANK_RANGE,
};
/// エフェクトスロット数。`op505_midi::EFFECT_SLOT_COUNT`（クランプ境界の一元管理元）と揃える。
const EFFECT_SLOT_COUNT: usize = op505_midi::EFFECT_SLOT_COUNT as usize;
use params::{
    Op505VstParams, DEFAULT_CHORUS_FEEDBACK, DEFAULT_CHORUS_MOD_DEPTH, DEFAULT_CHORUS_MOD_RATE,
    DEFAULT_CHORUS_SEND_TO_REVERB, DEFAULT_CHORUS_TYPE, DEFAULT_REVERB_TIME, DEFAULT_REVERB_TYPE,
};

use nice_plug::prelude::*;
use nice_plug_egui::EguiState;
use op505_core::{op505_presets_dir, Op505Engine, Op505Patch, Op505PresetBank};
use sound_core::{cc76_to_rate_scale, ChorusType, MasterSection, MeterBridge, ReverbType, Vco};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

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
    // エフェクトスロット配列 + スクラッチ確保 + 合算 + マスターボリューム/レベル計測を
    // まとめて持つ（`sound_core::MasterSection`）。各MIDIチャンネルの
    // `channels[ch].effect_route_slot`（NRPN(0,1) Channel Effect Route、既定0）が指す
    // スロットへ音声・エフェクト設定NRPN・CC91/93が適用される（詳細はspec-fm.md 8章）。
    // DAWパラメーター9個とCC91/93の初期値は従来どおりスロット0のみを対象にする。
    master: MasterSection,
    sample_rate: f32,

    // TimeEg 7本（`params.egs`のpersist状態）のオーディオスレッド側キャッシュ。
    // オーディオスレッドは`RwLock`をブロックせず`try_read()`で取得できたときだけ更新する
    // （取れなければ前ブロックの値を使う＝1〜2ブロック遅れて必ず収束する。plan参照）。
    cached_egs: Op505EgBank,

    // 発音中チャンネルID列挙用のスクラッチバッファ（`Op505Engine::collect_active_channels`が
    // 書き込む）。オーディオスレッドでのアロケーションを避けるため使い回す（毎回`Vec::new()`しない）。
    active_ids: Vec<usize>,

    // MIDIチャンネル別のCC/NRPNシャドウ状態（`op505_midi::ChannelState`、smf2op505/standalone
    // と共有する参照実装）。overrides/rpn/data_entry_msb/lsb/at_destination/
    // poly_at_destination/cc2_destination/cc4_destination/pitch_fg_rpn0_5/pitch_bend_range/
    // operator_f_number_override/effect_route_slotを含め、CC/NRPNの解釈状態はここに
    // 一本化されている（エフェクト系NRPN(0,2)〜(0,8)とCC91/93は各チャンネルの
    // `effect_route_slot`（NRPN(0,1)）が指すスロットへ適用され、DAWパラメーター9個のみ
    // `effects[0]`固定のまま。詳細はspec-fm.md 8章）。
    channels: [ChannelState; MIDI_CHANNEL_COUNT],

    // Algorithm/Waveform/Filter Type/Self-Oscillation：「前回ブロックで見たDAW値」。process()内で
    // DAW値がこの値から変化したら、全16ch分の`channels[ch].overrides`をNoneへクリアする
    // （「最後に触った方が勝つ」。GUIノブ操作後にNRPN上書きが永久に残り続けるのを防ぐ）。
    // build_patch()はDAWパラメーターを直接読むため、これらは差分検知専用でパッチ構築には使わない。
    last_algorithm: u8,
    last_filter_type_param: u8,
    last_filter_self_oscillation_param: bool,
    last_operator_waveforms: [u8; 4],

    // Pitch FG（②③層の補正を受ける唯一のFGスロット、spec-sound.md「演奏層による補正」節）。
    // CC1/76/77/78の生値・RPN(0,5) Modulation Depth Rangeは`channels[ch].pitch_fg_cc1/76/77/78`/
    // `pitch_fg_rpn0_5`（MIDIチャンネル別）に保持し、build_patch()で毎ブロックPitch FGの
    // Depth/rate_scaleへ計算適用する。CC78はop505のTimeEgにDelayフィールドが無いため、
    // 第0段が`level=0`の待ち段であるときに限りその段の`time`へ相対補正する（plan参照）。

    // Reverb/Chorus Send：DAWパラメーターとCC91/93の両方から設定され得るため、
    // マスターエフェクト5パラメーターと同じ1シャドウ差分検知方式で管理する。
    last_rev_send: u8,
    last_cho_send: u8,

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

    // AT/Poly AT Destination（NRPN(0,16)/(0,17)）・CC2/CC4の加算先（NRPN(0,34)/(0,35)）は
    // `channels[ch].at_destination`/`poly_at_destination`/`cc2_destination`/`cc4_destination`
    // （MIDIチャンネル別）に保持する。既定行先はCC2→TLキャリア一括（ウインド楽器風の明るさ/
    // 音量スウェル）、CC4→Filter Cutoff（古典的ワウペダル＝手動ワウ）。
    //
    // NRPN(0,18)〜(0,21) Operator F-Number Op0〜3（CC6+CC38の14bit値→13bit(0〜8191)にclamp）も
    // `channels[ch].operator_f_number_override`（`[Option<u16>;4]`、None=上書きなし）に保持する。

    // Bank Select（CC0=MSB, CC32=LSB）+ Program Change：MIDIチャンネルごとに管理する
    // 状態機械（`op505-midi::ChannelProgramState`）。`channels[ch].program_state`に集約する
    // （`bank_select_msb`等を別配列で並行して持つと「真実が2箇所」になりCC0が片方にしか
    // 反映されないバグの元になるため、意図的にここへ一本化してある）。
    /// リズムキットが`preset_bank`に1つでもロードされているか（`initialize()`/`reset()`で
    /// `preset_bank.has_bank_in(RHYTHM_BANK_RANGE)`から算出）。ch10の初期ドラムON判定に使う
    /// （キット未ロード環境でch10が突然無音になる回帰を防ぐ、`ChannelProgramState::new`参照）。
    rhythm_kits_available: bool,
    /// Program Changeで選択された旋律パッチ（MIDIチャンネルごと）。該当プリセットが無ければ
    /// `None`（`Op505PresetBank::get`と同じくフォールバックせず「見つからない」を伝える。
    /// `.op505`には`.38x6`のような波形メモリ/GM2/プレースホルダー代替が無いため）。
    /// **リズムチャンネルでは使わない**（ノートごとに音色が変わるため、常に`None`のまま
    /// `resolve_note_patch`がノートオンのたびに直接引く）。
    program_patch: [Option<Op505Patch>; 16],

    // `op505_presets_dir()`から読み込んだユーザープリセット集合（`initialize()`で読み込む）。
    // オーディオスレッド側キャッシュ（`cached_egs`と同じ役割）。MIDI Program Change解決は
    // 必ずこちらを読む。
    preset_bank: Op505PresetBank,

    // GUIエディタのPRESETSパネル（Save/Save As/+ New Voice/Delete）が更新する共有バンク。
    // GUIスレッドはblocking writeで更新し、オーディオスレッドは`preset_bank_dirty`が
    // 立っているときだけ`try_read()`で`preset_bank`へ取り込む（`cached_egs`/`params.egs`と
    // 同じ「GUIはブロックしてよい、オーディオは絶対にブロックしない」非対称パターン）。
    shared_preset_bank: Arc<RwLock<Op505PresetBank>>,
    // `shared_preset_bank`を`preset_bank`へ取り込む必要があるかの門番。オーディオスレッドが
    // 毎ブロックRwLockに触れずに済むようにする（保存操作は稀にしか起きないため）。
    preset_bank_dirty: Arc<AtomicBool>,

    // GUIエディターのウィンドウサイズ状態（`editor()`で使い回す）。
    egui_state: Arc<EguiState>,

    // マスター出力の計測値（オーディオスレッド⇄GUIの橋渡し）。オーディオスレッドは
    // `MasterSection::render()`後に`take_measurement()`でピークを読み、`publish()`する
    // （`sound_core::MeterBridge`、`try_lock`のみを使いオーディオ側は待たない）。
    master_meter: Arc<MeterBridge>,
    // レベルメーターのpublish区間中に累積するピーク値（取りこぼし防止。standaloneの
    // オーディオコールバックと同じ設計、`main.rs`参照）。
    meter_peak_l: f32,
    meter_peak_r: f32,
    meter_clipped: bool,
    frames_since_publish: usize,
    // publish間隔（フレーム数）。サンプルレートに依存するため`initialize()`で再計算する。
    publish_interval_frames: usize,
}

impl Default for Op505Plugin {
    fn default() -> Self {
        const DEFAULT_SR: f32 = 44100.0;
        let params = Arc::new(Op505VstParams::default());
        let cached_egs = *params.egs.read().expect("Poisoned RwLock on read");
        Self {
            params,
            engine: Op505Engine::new(DEFAULT_SR),
            master: MasterSection::new(DEFAULT_SR, EFFECT_SLOT_COUNT),
            sample_rate: DEFAULT_SR,
            cached_egs,
            active_ids: Vec::with_capacity(256),
            // preset_bankがまだ空(Default)なのでrhythm_kits_available=falseは正しい既定値。
            // 実際の値はinitialize()で再計算する。
            channels: std::array::from_fn(|i| ChannelState::new(i, false)),
            last_algorithm: params::DEFAULT_ALGORITHM,
            last_filter_type_param: 0,
            last_filter_self_oscillation_param: true,
            last_operator_waveforms: [0; 4],
            last_rev_send: 0,
            last_cho_send: 0,
            last_reverb_type: DEFAULT_REVERB_TYPE,
            last_reverb_time: DEFAULT_REVERB_TIME,
            last_chorus_type: DEFAULT_CHORUS_TYPE,
            last_chorus_mod_rate: DEFAULT_CHORUS_MOD_RATE,
            last_chorus_mod_depth: DEFAULT_CHORUS_MOD_DEPTH,
            last_chorus_feedback: DEFAULT_CHORUS_FEEDBACK,
            last_chorus_send_to_reverb: DEFAULT_CHORUS_SEND_TO_REVERB,
            rhythm_kits_available: false,
            program_patch: [None; 16],
            preset_bank: Op505PresetBank::default(),
            shared_preset_bank: Arc::new(RwLock::new(Op505PresetBank::default())),
            preset_bank_dirty: Arc::new(AtomicBool::new(false)),
            // 既定サイズもeditor_min_width()以上にしておく（下回るとエディタが開いた瞬間から
            // 横スクロールを要求する状態になり体験が悪いため）。
            egui_state: EguiState::from_size(op505_editor::layout::editor_min_width().ceil() as u32, 680),
            master_meter: Arc::new(MeterBridge::new()),
            meter_peak_l: 0.0,
            meter_peak_r: 0.0,
            meter_clipped: false,
            frames_since_publish: 0,
            publish_interval_frames: Self::compute_publish_interval_frames(DEFAULT_SR),
        }
    }
}

impl Op505Plugin {
    /// レベルメーターのpublish間隔（フレーム数）を計算する。`%APPDATA%\op505\ui.json`の
    /// `meter_fps`（既定10fps、standaloneと同じ`op505_core::meter_fps`）を読む。
    /// 設定ファイルの読み込みはファイルI/Oを伴うため、`initialize()`（非リアルタイム
    /// コンテキスト）でのみ呼ぶこと。
    fn compute_publish_interval_frames(sample_rate: f32) -> usize {
        let meter_fps = op505_core::meter_fps(&op505_core::ui_config::load());
        ((sample_rate / meter_fps as f32).max(1.0)) as usize
    }

    /// 現在のDAWパラメーター・`cached_egs`から`Op505Patch`を構築する（MIDIチャンネル非依存。
    /// NRPN(0,9)〜(0,15)由来の`overrides`はここでは適用しない。Program Change選択中の
    /// チャンネルでも必ず後段適用できるよう、`apply_pitch_fg_expression`と同じ「note_patchへの
    /// 後処理」パターンへ分離してある。CC1/76/77/78のPitch FG演奏補正も同様に別途適用する）。
    /// 本体は`params::build_patch`（GUIスレッドの保存処理とロジックを共有する自由関数）へ委譲。
    fn build_patch(&self) -> Op505Patch {
        params::build_patch(&self.params, &self.cached_egs)
    }

    /// `preset_bank`が保存操作で更新された直後に呼ぶ：Program Changeで既に解決済みの
    /// `program_patch`キャッシュを、新しい`preset_bank`の内容で引き直す。**PC未受信のまま
    /// （`program_patch[ch]`がNone＝DAWノブに追従中）のチャンネルは対象外**——巻き込むと
    /// bank0/program0が存在するだけでノブ操作が効かなくなる回帰になる。リズムチャンネルは
    /// `resolve_note_patch`が毎回`preset_bank`を直接引くため対応不要。
    fn refresh_program_patch_cache(&mut self) {
        for ch in 0..MIDI_CHANNEL_COUNT {
            if self.program_patch[ch].is_none() {
                continue;
            }
            if let ProgramSelection::Melodic { bank, program } = self.channels[ch].program_state.selection() {
                self.program_patch[ch] = self.preset_bank.get(bank, program).map(|preset| preset.patch);
            }
        }
    }

    /// このMIDIチャンネル・ノートで鳴らすべきベースパッチ（DAWパラメーター等の後処理を
    /// 適用する前の①層）。`None`ならこのノートは発音しない
    /// （リズムチャンネルでキット内に未定義のノート＝GM2実機で無音になるのと同じ）。
    ///
    /// 旋律チャンネルは従来どおり`program_patch[midi_ch]`（Program Changeで選ばれていれば）→
    /// `build_patch()`（DAWパラメーター由来）の優先順で必ず`Some`を返す。
    /// リズムチャンネルはノートごとに異なる音色を`preset_bank`から直接引く（`program_patch`は
    /// 使わない。オーディオスレッドでのアロケーションは無い：`HashMap::get` + `Op505Patch`は
    /// `Copy`のためmemcpyのみ）。
    fn resolve_note_patch(&self, midi_ch: usize, note: u8) -> Option<Op505Patch> {
        let state = &self.channels[midi_ch].program_state;
        if state.is_rhythm() {
            let (bank, program) = state.lookup_address(note);
            self.preset_bank
                .get(bank, program)
                // GM2: キット内未定義ノートはStandard Kit(kit 0)へフォールバックする。
                .or_else(|| {
                    state.rhythm_fallback_address(note).and_then(|(b, p)| self.preset_bank.get(b, p))
                })
                .map(|preset| preset.patch)
        } else {
            Some(self.program_patch[midi_ch].unwrap_or_else(|| self.build_patch()))
        }
    }

    /// Program Change（`MidiProgramChange`/CC102代替の両方から呼ぶ）。`ChannelProgramState`で
    /// 旋律/リズムを確定させ、旋律なら`program_patch`をキャッシュする（リズムはノートごとに
    /// `resolve_note_patch`が直接引くため、ここではキャッシュしない＝常に`None`にする）。
    fn apply_program_change(&mut self, midi_ch: usize, program: u8) {
        // NRPN離散上書きレイヤーは当該chのみクリアする（`ChannelState::program_change`が担当。
        // 「PC＝音色を選び直す」「その後のNRPN＝その音色への微調整」という役割分担、
        // plan「op505-vstとop505-midiのNRPN上書きレイヤー共有化」参照）。
        match self.channels[midi_ch].program_change(program) {
            ProgramSelection::Melodic { bank, program } => {
                self.program_patch[midi_ch] = self.preset_bank.get(bank, program).map(|preset| preset.patch);
            }
            ProgramSelection::Rhythm { .. } => self.program_patch[midi_ch] = None,
        }
    }

    /// CC76(Vibrato Rate)由来のPitch FG速さスケールを計算する（`cc76_to_rate_scale`、
    /// 64=1.0倍=無補正）。build_patch()とは別に、`engine.set_pitch_fg_rate_scale`で
    /// 直接エンジンへ渡す（ChannelParamsを経由しない、pitch_bend/channel_volumeと同じ経路）。
    fn pitch_fg_rate_scale(&self, midi_ch: usize) -> f32 {
        cc76_to_rate_scale(self.channels[midi_ch].pitch_fg_cc76)
    }

    /// 1ボイスの実際の発音処理（ペダル・Mono状態の更新は含まない）。`NoteEvent::NoteOn`の
    /// match腕から切り出したもの（`smf2op505`/`op505-standalone`の`note_on_voice_core`と同型）。
    ///
    /// Mono Modeのlast-note priorityフォールバック（[`MonoNoteOff::Fallback`]）からも直接
    /// 呼べるよう、ペダル状態の更新（`pedal.note_on`）は呼び出し側で行う（フォールバック
    /// 再発音は「新規の押鍵」ではないため、CC67(Soft Pedal)のsoft_notes判定を押鍵時点の
    /// 状態のまま動かしたい）。
    fn note_on_voice_core(&mut self, midi_ch: usize, note: u8, velocity_u8: u8) {
        let freq = 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);
        let ch_id = midi_channel_note_id(midi_ch as u8, note);
        // リズムチャンネルはノートごとに音色を直接解決する。キット内に未定義の
        // ノートなら発音しない（`Op505Patch::default()`でnote_onしてはいけない。
        // 全段level=0/time=0のEGはrelease_pointで静止し`is_idle()`が永久にfalseに
        // なりボイスが漏れるため。旋律チャンネルは従来どおりProgram Change
        // パッチ優先・無ければDAW値）。
        let Some(base_patch) = self.resolve_note_patch(midi_ch, note) else {
            return;
        };
        // NRPN(0,9)〜(0,15)由来の上書き（live伝播ループと同じ、note_patch組み立て時に適用）。
        let mut note_on_patch = self.channels[midi_ch].build_effective_patch(&base_patch);
        // CC2/CC4/AT/Poly AT/Pitch FG(CC1/76/77/78)/Soft Pedalを第0tickから効かせる
        // （伝播ループと同じ`apply_note_post_processing`。CC78(Delay)が
        // `TimeEg::note_on()`の読む最初のtickから効く必要があるため、伝播ループでの
        // 1ブロック遅れの反映を待たずここで適用する）。
        self.channels[midi_ch].apply_note_post_processing(&mut note_on_patch, note);
        self.engine.set_patch(note_on_patch);
        self.engine.note_on(ch_id, freq, velocity_u8);
        self.engine.set_pitch_bend(ch_id, self.channels[midi_ch].total_pitch_bend_cents());
        self.engine.set_channel_volume(
            ch_id,
            channel_gain(self.channels[midi_ch].cc7, self.channels[midi_ch].cc11),
        );
        self.engine.set_channel_pan(ch_id, self.channels[midi_ch].pan_gains());
        self.engine.set_pitch_fg_rate_scale(ch_id, self.pitch_fg_rate_scale(midi_ch));
        for (op_index, f_number) in self.channels[midi_ch].operator_f_number_override.iter().enumerate() {
            let Some(f_number) = f_number else { continue };
            self.engine.set_operator_f_number(ch_id, op_index, *f_number);
        }
        if let Some((from_note, seconds)) = self.channels[midi_ch].glide_source(note) {
            let from_freq = 440.0 * 2.0_f32.powf((from_note as f32 - 69.0) / 12.0);
            self.engine.start_glide(ch_id, from_freq, seconds);
        }
        self.channels[midi_ch].last_note = Some(note);
    }

    /// CC6(Data Entry MSB)受信時、`ChannelState::apply_data_entry`（`op505-midi`、smf2op505/
    /// standaloneと共有する参照実装）へ委譲する。`value`はCC値の正規化値（0.0〜1.0）で、
    /// `cc_to_u7`で生バイト相当（0〜127）へ変換してから渡す（`cc_to_u7`は冪等なので
    /// `ChannelState`側の`cc_byte_to_u7`と完全一致する）。
    ///
    /// エフェクト系NRPN(0,2)〜(0,8)だけは`DataEntryOutcome::Effect`で通知される
    /// （`MasterEffects`はsound-core型のため`op505-midi`のAPIに出せず、`op505-midi`側は
    /// 状態を変化させず値を返すだけ。呼び出し側＝ここで自分の`effects`へ適用する）。
    ///
    /// `voice_update`は無視してよい：`voice_update:true`を返す全ターゲット
    /// （overrides/at_destination/poly_at_destination/cc2_destination/cc4_destination/
    /// pitch_fg_rpn0_5/operator_f_number_override/tune_coarse/tune_fine）は、毎ブロック先頭の
    /// 伝播ループが無条件に全対象を再構築するため（将来「変化したchだけ伝播」のような最適化を
    /// 入れると静かに壊れるので、この前提を崩す変更をする際は要注意）。
    fn handle_data_entry(&mut self, midi_ch: usize, value: f32) {
        match self.channels[midi_ch].apply_data_entry(cc_to_u7(value)) {
            DataEntryOutcome::StateChanged { voice_update: _ } => {}
            DataEntryOutcome::Effect(slot, target, v) => {
                let fx = self.master.slot_mut((slot as usize).min(EFFECT_SLOT_COUNT - 1));
                sound_midi::apply_effect_control(fx, target, v);
            }
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
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.engine = Op505Engine::new(self.sample_rate);
        self.master = MasterSection::new(self.sample_rate, EFFECT_SLOT_COUNT);
        self.publish_interval_frames = Self::compute_publish_interval_frames(self.sample_rate);
        self.preset_bank = Op505PresetBank::load_from_dir(&op505_presets_dir());
        // GUI側が参照する共有バンクも同じ内容で揃える。エディタを開いたままサンプルレート変更等で
        // 再initialize()されると、presets_dir**外**のファイルをOpenして音声側へ載せていたバンクは
        // ディスク由来の内容へ戻る（presets_dir内のファイルは保存済みなので影響なし）。
        *self.shared_preset_bank.write().expect("Poisoned RwLock on write") = self.preset_bank.clone();
        self.preset_bank_dirty.store(false, Ordering::Release);
        self.rhythm_kits_available = self.preset_bank.has_bank_in(RHYTHM_BANK_RANGE);
        // ChannelState全体ではなくprogram_stateだけをリセットする（rhythm_kits_availableが
        // 確定した直後のため。CC/NRPN/ペダル状態まで消える挙動変更を避ける）。
        let rhythm = self.rhythm_kits_available;
        for (i, st) in self.channels.iter_mut().enumerate() {
            st.program_state.reset(i, rhythm);
        }
        true
    }

    fn reset(&mut self) {
        self.engine = Op505Engine::new(self.sample_rate);
        self.master = MasterSection::new(self.sample_rate, EFFECT_SLOT_COUNT);
        // GM2 System Reset相当。CC121(Reset All Controllers)はbank/programをリセットしない
        // （ChannelProgramState::resetのdocコメント参照）ため、ここでのみ呼ぶ。
        let rhythm = self.rhythm_kits_available;
        for (i, st) in self.channels.iter_mut().enumerate() {
            st.reset(i, rhythm);
        }
        self.last_rev_send = 0;
        self.last_cho_send = 0;
        self.last_reverb_type = DEFAULT_REVERB_TYPE;
        self.last_reverb_time = DEFAULT_REVERB_TIME;
        self.last_chorus_type = DEFAULT_CHORUS_TYPE;
        self.last_chorus_mod_rate = DEFAULT_CHORUS_MOD_RATE;
        self.last_chorus_mod_depth = DEFAULT_CHORUS_MOD_DEPTH;
        self.last_chorus_feedback = DEFAULT_CHORUS_FEEDBACK;
        self.last_chorus_send_to_reverb = DEFAULT_CHORUS_SEND_TO_REVERB;
        self.program_patch = [None; 16];
        self.meter_peak_l = 0.0;
        self.meter_peak_r = 0.0;
        self.meter_clipped = false;
        self.frames_since_publish = 0;
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        // エディタを開くたびの1回だけ読む（GUIスレッドかつ低頻度のためファイルI/Oを許容する）。
        let ui_config = op505_core::ui_config::load();
        let meter_fps = op505_core::meter_fps(&ui_config);
        let level_meter_gap_px = op505_core::level_meter_gap_px(&ui_config);
        editor::create_editor(
            self.egui_state.clone(),
            self.params.clone(),
            self.shared_preset_bank.clone(),
            self.preset_bank_dirty.clone(),
            self.master_meter.clone(),
            meter_fps,
            level_meter_gap_px,
        )
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

        // プリセットバンク：GUIエディタのSave/Save As/+ New Voice/Deleteが`shared_preset_bank`へ
        // 書き込むと`preset_bank_dirty`が立つ。swap(false)で「読む」と「消す」を不可分に行い、
        // try_readが失敗したら（GUIが競合して稀に取れなかった）フラグを立て直して次ブロックで
        // 再挑戦する（更新を取りこぼさない）。
        if self.preset_bank_dirty.swap(false, Ordering::Acquire) {
            // try_read()のガードを先に手放してから&mut selfを要するメソッドを呼ぶ
            // （ガードがself.shared_preset_bankを借用したままだとrefresh_program_patch_cache()の
            // &mut self呼び出しと競合しコンパイルエラーになる）。
            let synced = if let Ok(bank) = self.shared_preset_bank.try_read() {
                self.preset_bank.clone_from(&bank);
                true
            } else {
                false
            };
            if synced {
                self.rhythm_kits_available = self.preset_bank.has_bank_in(RHYTHM_BANK_RANGE);
                self.refresh_program_patch_cache();
            } else {
                self.preset_bank_dirty.store(true, Ordering::Release);
            }
        }

        // TimeEgのテンポ同期用BPM。ホストが未再生等でtempoを返さない場合は前回値を保持する
        // （Op505Engine::set_tempoが0以下を無視するのと同じ防御）。
        if let Some(tempo) = context.transport().tempo {
            self.engine.set_tempo(tempo as f32);
        }

        // Algorithm/Filter Type/Self-Oscillation/Waveform：DAWオートメーションで値が変化したら、
        // NRPN(0,9)〜(0,15)由来の上書きを**全16ch分**クリアする（「最後に触った方が勝つ」。
        // GUIノブ操作後にNRPN上書きが永久に残り続けるのを防ぐ。DAWパラメーターはプラグイン全体で
        // 1組＝`build_patch()`が全ch共通の①層のため、ノブを触ったら全chの上書きを捨てるのが
        // 意味論的に正しい。build_patch()はDAWパラメーターを直接読むため、変化していない間は
        // 何もしなくてよい。比較を先に済ませ、変化があったときだけ16chループを回す）。
        let algorithm = self.params.algorithm.value() as u8;
        let clear_algorithm = algorithm != self.last_algorithm;
        let filter_type_param = self.params.filter_type.value() as u8;
        let clear_filter_type = filter_type_param != self.last_filter_type_param;
        let filter_self_oscillation_param = self.params.filter_self_oscillation.value();
        let clear_filter_self_oscillation = filter_self_oscillation_param != self.last_filter_self_oscillation_param;
        let waveforms: [u8; 4] = std::array::from_fn(|i| self.params.operators[i].waveform.value() as u8);
        let clear_waveform: [bool; 4] = std::array::from_fn(|i| waveforms[i] != self.last_operator_waveforms[i]);

        if clear_algorithm
            || clear_filter_type
            || clear_filter_self_oscillation
            || clear_waveform.iter().any(|&b| b)
        {
            for st in self.channels.iter_mut() {
                if clear_algorithm {
                    st.overrides.algorithm = None;
                }
                if clear_filter_type {
                    st.overrides.filter_type = None;
                }
                if clear_filter_self_oscillation {
                    st.overrides.filter_self_oscillation = None;
                }
                for i in 0..4 {
                    if clear_waveform[i] {
                        st.overrides.operator_waveforms[i] = None;
                    }
                }
            }
        }
        self.last_algorithm = algorithm;
        self.last_filter_type_param = filter_type_param;
        self.last_filter_self_oscillation_param = filter_self_oscillation_param;
        self.last_operator_waveforms = waveforms;

        // Reverb/Chorus Send：DAWパラメーターとCC91/93の両方から設定され得るため、
        // マスターエフェクト5パラメーターと同じ1シャドウ差分検知方式で適用する。
        // DAWパラメーターはスロット選択UIを持たないため、常に`effects[0]`（スロット1）のみを対象にする。
        let rev_send = self.params.rev_send.value() as u8;
        if rev_send != self.last_rev_send {
            self.master.slot_mut(0).set_reverb_send(rev_send);
            self.last_rev_send = rev_send;
        }
        let cho_send = self.params.cho_send.value() as u8;
        if cho_send != self.last_cho_send {
            self.master.slot_mut(0).set_chorus_send(cho_send);
            self.last_cho_send = cho_send;
        }

        // マスター単位パラメーター：DAWオートメーションで値が変化した場合のみスロット0へ反映する。
        // NRPN(0,2)〜(0,8)はeffects[slot]へ直接書き込まれ、ここでの値が前回と同じ間は上書きされない
        // （差分検知方式。NRPNの変更はnice-plug側のパラメーター表示には反映されない）。
        let reverb_type = self.params.reverb_type.value() as u8;
        if reverb_type != self.last_reverb_type {
            self.master.slot_mut(0).set_reverb_type(ReverbType::from_u8(reverb_type));
            self.last_reverb_type = reverb_type;
        }
        let reverb_time = self.params.reverb_time.value() as u8;
        if reverb_time != self.last_reverb_time {
            self.master.slot_mut(0).set_reverb_time(reverb_time);
            self.last_reverb_time = reverb_time;
        }
        let chorus_type = self.params.chorus_type.value() as u8;
        if chorus_type != self.last_chorus_type {
            self.master.slot_mut(0).set_chorus_type(ChorusType::from_u8(chorus_type));
            self.last_chorus_type = chorus_type;
        }
        let chorus_mod_rate = self.params.chorus_mod_rate.value() as u8;
        if chorus_mod_rate != self.last_chorus_mod_rate {
            self.master.slot_mut(0).set_chorus_mod_rate(chorus_mod_rate);
            self.last_chorus_mod_rate = chorus_mod_rate;
        }
        let chorus_mod_depth = self.params.chorus_mod_depth.value() as u8;
        if chorus_mod_depth != self.last_chorus_mod_depth {
            self.master.slot_mut(0).set_chorus_mod_depth(chorus_mod_depth);
            self.last_chorus_mod_depth = chorus_mod_depth;
        }
        let chorus_feedback = self.params.chorus_feedback.value() as u8;
        if chorus_feedback != self.last_chorus_feedback {
            self.master.slot_mut(0).set_chorus_feedback(chorus_feedback);
            self.last_chorus_feedback = chorus_feedback;
        }
        let chorus_send_to_reverb = self.params.chorus_send_to_reverb.value() as u8;
        if chorus_send_to_reverb != self.last_chorus_send_to_reverb {
            self.master.slot_mut(0).set_chorus_send_to_reverb(chorus_send_to_reverb);
            self.last_chorus_send_to_reverb = chorus_send_to_reverb;
        }

        // マスターボリューム：NRPN/SysEx経由の上書き経路が無い（VSTは`SysExMessage = ()`）ため、
        // 他のマスター単位パラメーターと違い差分検知は不要。毎ブロックDAW値をそのまま反映する。
        self.master.output_mut().set_volume(self.params.master_volume.value() as u8);

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
            // リズムチャンネルはノートごとに音色が違うため、MIDIチャンネル単位の
            // channel_patchesキャッシュは使わずノートごとに直接解決する（resolve_note_patch）。
            let base_patch = if self.channels[midi_ch].program_state.is_rhythm() {
                match self.resolve_note_patch(midi_ch, note) {
                    Some(p) => p,
                    // キット切替等でこのノートが指す音色が消えた。既存の発音中パラメーターを
                    // そのまま維持する（無音にしたり別音色へ差し替えたりはしない）。
                    None => continue,
                }
            } else {
                channel_patches[midi_ch].unwrap_or_else(|| {
                    // Program Changeで選ばれていればそれを基底に、無ければDAWパラメーター由来。
                    let patch = self.program_patch[midi_ch].unwrap_or_else(|| self.build_patch());
                    channel_patches[midi_ch] = Some(patch);
                    patch
                })
            };
            // NRPN(0,9)〜(0,15)由来の上書き（Program Change選択中のチャンネルでも必ず効く。
            // Pitch FGと同じ「note_patchへの後処理」パターン、plan参照）。
            let mut note_patch = self.channels[midi_ch].build_effective_patch(&base_patch);
            // CC2/CC4/AT/Poly AT/Pitch FG(CC1/76/77/78)/Soft Pedal（適用順はexpression→
            // pitch_fg→soft_pedalでnote_on側と完全一致＝ビット不変）。
            self.channels[midi_ch].apply_note_post_processing(&mut note_patch, note);
            self.engine.set_channel_params(ch_id, note_patch.channel);
            for (op_index, op) in note_patch.operators.iter().enumerate() {
                self.engine.set_operator_params(ch_id, op_index, *op);
            }
            self.engine.set_pitch_fg_rate_scale(ch_id, self.pitch_fg_rate_scale(midi_ch));
            // RPN(0,1)/(0,2) Channel Fine/Coarse Tuning。ピッチベンドホイールの即時反映
            // （`set_pitch_bend_group`）とは別に、他の全NRPN補正と同じく1ブロック遅れで
            // ここへ伝播する（`total_pitch_bend_cents`＝bend_cents+tune_cents、常に無条件で
            // 再送する設計なので取りこぼしは無い）。
            self.engine.set_pitch_bend(ch_id, self.channels[midi_ch].total_pitch_bend_cents());
            // NRPN(0,18)〜(0,21) Operator F-Number上書き。他の全NRPN補正と同じく1ブロック遅れで
            // 伝播する（Someのみ適用。Noneはnote_on時のリセット値=F_NUMBER_CENTER相当のまま）。
            for (op_index, f_number) in self.channels[midi_ch].operator_f_number_override.iter().enumerate() {
                let Some(f_number) = f_number else { continue };
                self.engine.set_operator_f_number(ch_id, op_index, *f_number);
            }
        }
        self.active_ids = ids;

        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { channel, note, velocity, .. } if velocity > 0.0 => {
                    let velocity_u8 = (velocity * 127.0).round() as u8;
                    let midi_ch = channel as usize;
                    // 弾き直したらペダル保留を解除する（`ym38x6-vst`と同一理由）。
                    self.channels[midi_ch].pedal.note_on(note);
                    // レガート継続（Mono+CC65 ON+レガート）が成立したら再アタックしない。
                    // `process()`本体のループなので`return`で抜けると残りのイベント処理と
                    // このブロックのレンダリングを丸ごとスキップしてしまうため、フラグで制御する。
                    let mut legato_succeeded = false;
                    if self.channels[midi_ch].mono.enabled {
                        let portamento = self.channels[midi_ch].portamento_on;
                        match self.channels[midi_ch].mono.note_on(note, velocity_u8, portamento) {
                            MonoNoteOn::Legato { voice } => {
                                let seconds = self.channels[midi_ch].portamento_seconds();
                                let id = midi_channel_note_id(channel, voice);
                                let freq = 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);
                                if self.engine.glide_to(id, freq, seconds) {
                                    self.channels[midi_ch].last_note = Some(note);
                                    legato_succeeded = true;
                                } else {
                                    // ボイスが既にIdle等で消えていたら通常発音へフォールバックする。
                                    self.channels[midi_ch].mono.demote_legato(note);
                                }
                            }
                            MonoNoteOn::Retrigger { release } => {
                                if let Some(prev) = release {
                                    self.engine.note_off(midi_channel_note_id(channel, prev));
                                }
                            }
                        }
                    }
                    if !legato_succeeded {
                        self.note_on_voice_core(midi_ch, note, velocity_u8);
                    }
                }
                NoteEvent::NoteOn { channel, note, .. } | NoteEvent::NoteOff { channel, note, .. } => {
                    let midi_ch = channel as usize;
                    self.channels[midi_ch].poly_pressure[note as usize] = 0;
                    // ペダルの内部ブックキーピング（keys_down等）は常に更新する。実際に
                    // エンジンへnote_offするかどうかはMono Mode有無で分岐する。
                    let pedal_released = self.channels[midi_ch].pedal.note_off(note);
                    if self.channels[midi_ch].mono.enabled {
                        // Mono Mode: サステインペダルより優先する（CC126/127のコメント参照）。
                        // 押鍵状態（MonoState）だけで解放/フォールバックを決める。
                        let portamento = self.channels[midi_ch].portamento_on;
                        match self.channels[midi_ch].mono.note_off(note, portamento) {
                            MonoNoteOff::Nothing => {}
                            MonoNoteOff::Release(released_note) => {
                                self.engine.note_off(midi_channel_note_id(channel, released_note));
                            }
                            MonoNoteOff::Fallback { release, sound, velocity } => {
                                self.engine.note_off(midi_channel_note_id(channel, release));
                                self.note_on_voice_core(midi_ch, sound, velocity);
                            }
                            MonoNoteOff::LegatoFallback { voice, sound, velocity } => {
                                let seconds = self.channels[midi_ch].portamento_seconds();
                                let id = midi_channel_note_id(channel, voice);
                                let freq = 440.0 * 2.0_f32.powf((sound as f32 - 69.0) / 12.0);
                                if !self.engine.glide_to(id, freq, seconds) {
                                    // ボイスが既にIdle等で消えていたら通常発音へフォールバックする。
                                    self.channels[midi_ch].mono.demote_legato(sound);
                                    self.engine.note_off(id);
                                    self.note_on_voice_core(midi_ch, sound, velocity);
                                }
                            }
                        }
                    } else if pedal_released {
                        self.engine.note_off(midi_channel_note_id(channel, note));
                    }
                }
                NoteEvent::MidiPitchBend { channel, value, .. } => {
                    let midi_ch = channel as usize;
                    let cents = (value - 0.5) * 2.0 * self.channels[midi_ch].pitch_bend_range * 100.0;
                    self.channels[midi_ch].bend_cents = cents;
                    self.engine.set_pitch_bend_group(midi_ch, cents);
                }
                NoteEvent::MidiChannelPressure { channel, pressure, .. } => {
                    self.channels[channel as usize].channel_pressure = cc_to_u8(pressure);
                }
                NoteEvent::PolyPressure { channel, note, pressure, .. } => {
                    self.channels[channel as usize].poly_pressure[note as usize] = cc_to_u8(pressure);
                }
                NoteEvent::MidiProgramChange { program, channel, .. } => {
                    self.apply_program_change(channel as usize, program);
                }
                NoteEvent::MidiCC { cc, value, channel, .. } => {
                    let midi_ch = channel as usize;
                    match cc {
                        7 => {
                            self.channels[midi_ch].cc7 = cc_to_u7(value);
                            self.engine.set_channel_volume_group(
                                midi_ch,
                                channel_gain(self.channels[midi_ch].cc7, self.channels[midi_ch].cc11),
                            );
                        }
                        11 => {
                            self.channels[midi_ch].cc11 = cc_to_u7(value);
                            self.engine.set_channel_volume_group(
                                midi_ch,
                                channel_gain(self.channels[midi_ch].cc7, self.channels[midi_ch].cc11),
                            );
                        }
                        // CC1(モジュレーションホイール)：Pitch FG Depthへの瞬間加算（セント換算、
                        // build_patch()参照）。質感LFOは焼き込み専用のためCC補正を受けない。
                        1 => {
                            self.channels[midi_ch].pitch_fg_cc1 = cc_to_u7(value);
                        }
                        // CC2(ブレス)：Expression Destination（NRPN(0,34)）への加算。既定TLキャリア一括。
                        2 => {
                            self.channels[midi_ch].cc2 = cc_to_u8(value);
                        }
                        // CC4(フット)：Expression Destination（NRPN(0,35)）への加算。既定Filter Cutoff＝手動ワウ。
                        4 => {
                            self.channels[midi_ch].cc4 = cc_to_u8(value);
                        }
                        // CC76(Vibrato Rate)：Pitch FGの速さスケール（64=無補正、rate_scale経由）。
                        76 => {
                            self.channels[midi_ch].pitch_fg_cc76 = cc_to_u7(value);
                        }
                        // CC77(Vibrato Depth)：Pitch FG Depthへの0起点パート加算。
                        77 => {
                            self.channels[midi_ch].pitch_fg_cc77 = cc_to_u8(value);
                        }
                        // CC78(Vibrato Delay)：Pitch FG第0段(level=0のとき)のtimeへの64中心相対補正。
                        78 => {
                            self.channels[midi_ch].pitch_fg_cc78 = cc_to_u7(value);
                        }
                        // CC92(Tremolo Depth)：Gain FG Depthへの0起点加算（CC77と同型、RPN連動レンジは無い）。
                        92 => {
                            self.channels[midi_ch].gain_fg_cc92 = cc_to_u8(value);
                        }
                        // CC10(Pan)：ボイス単位の左右ゲイン（patchではなくVco::set_channel_pan_group
                        // 経由、コンスタントパワー則）。CC7/CC11と同じく受信時に即座に発音中へ反映する。
                        10 => {
                            self.channels[midi_ch].cc10_pan = cc_to_u7(value);
                            self.engine.set_channel_pan_group(midi_ch, self.channels[midi_ch].pan_gains());
                        }
                        // CC71(Resonance)：Filter Resonanceへの64中心相対補正。伝播ループが毎ブロック
                        // apply_note_post_processing経由で自動反映する（値を保持するのみ）。
                        71 => {
                            self.channels[midi_ch].cc71_resonance = cc_to_u7(value);
                        }
                        // CC72(Release Time)：保持区間のピーク検出で分割したRelease区間（キャリアのみ）
                        // の時間スケール（`op505_midi::apply_sound_controllers`参照）。
                        72 => {
                            self.channels[midi_ch].cc72_release = cc_to_u7(value);
                        }
                        // CC73(Attack Time)：Attack区間（キャリアのみ）の時間スケール。
                        73 => {
                            self.channels[midi_ch].cc73_attack = cc_to_u7(value);
                        }
                        // CC74(Brightness)：Filter Cutoffへの64中心相対補正。CC4(既定Filter Cutoff)
                        // とは加算で共存する（互いの寄与が単純加算される、64では寄与ゼロ）。
                        74 => {
                            self.channels[midi_ch].cc74_brightness = cc_to_u7(value);
                        }
                        // CC75(Decay Time)：Decay区間（キャリアのみ）の時間スケール。
                        75 => {
                            self.channels[midi_ch].cc75_decay = cc_to_u7(value);
                        }
                        // CC5(Portamento Time)/CC65(Portamento On/Off)：次のNoteOnでのグライドに
                        // 使うだけなので、発音中ボイスへの即時反映は不要。
                        5 => {
                            self.channels[midi_ch].portamento_time = cc_to_u7(value);
                        }
                        65 => {
                            self.channels[midi_ch].portamento_on = cc_to_u7(value) >= 64;
                        }
                        // CC64(サステインペダル)：ホールドフラグ方式（`ym38x6-vst`と同一設計）。
                        64 => {
                            let released = self.channels[midi_ch].pedal.cc64(cc_to_u7(value));
                            for note in released_notes(released) {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                        }
                        // CC66(Sostenuto)：ON時点でkeys_down中のノートのみをlatchし、CC66 OFF
                        // （かつCC64も踏まれていない）までReleaseに入らせない。
                        66 => {
                            let released = self.channels[midi_ch].pedal.cc66(cc_to_u7(value));
                            for note in released_notes(released) {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                        }
                        // CC67(Soft Pedal)：深さを保持するのみ。ON中に新規キーオンしたノートのみ
                        // への適用はNoteOn/live伝播ループ側（soft_notesビット）で行う。
                        67 => {
                            self.channels[midi_ch].pedal.cc67(cc_to_u7(value));
                        }
                        // CC121(Reset All Controllers)：③ジェスチャー層のみリセットする
                        // （②パート状態・①音色は保持、spec-sound.md「補強規則」）。CC64/66/67ペダル・
                        // Pitch Bend・CC1・アフタータッチが対象。CC2/CC4/CC7/CC11/CC76〜78/センド/
                        // RPN等は保持。GM2でもRACはbank/programをリセットしないため、
                        // 意図的に`program_state`（リズム/旋律の状態）へは一切触れない。
                        121 => {
                            let released = self.channels[midi_ch].reset_all_controllers();
                            for note in released_notes(released) {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                            self.engine.set_pitch_bend_group(midi_ch, 0.0);
                        }
                        // Bank Select（CC0=MSB, CC32=LSB）：これだけでは旋律/リズムは
                        // 切り替わらない（次のProgram Changeで確定する、ChannelProgramState参照）。
                        0 => self.channels[midi_ch].program_state.bank_select_msb(cc_to_u7(value)),
                        32 => self.channels[midi_ch].program_state.bank_select_lsb(cc_to_u7(value)),
                        98 => self.channels[midi_ch].rpn.set_nrpn_lsb(cc_to_u7(value)),
                        99 => self.channels[midi_ch].rpn.set_nrpn_msb(cc_to_u7(value)),
                        100 => self.channels[midi_ch].rpn.set_rpn_lsb(cc_to_u7(value)),
                        101 => self.channels[midi_ch].rpn.set_rpn_msb(cc_to_u7(value)),
                        6 => self.handle_data_entry(midi_ch, value),
                        // CC38(Data Entry LSB)：OP F-Number(NRPN(0,18)〜(0,21))・Channel Fine
                        // Tuning(RPN(0,1))選択中のときだけ14bit値を更新する
                        // （`ChannelState::apply_data_entry_lsb`。戻り値の即時反映フラグは
                        // 無視してよい。伝播ループが毎ブロック無条件に全対象を再構築するため）。
                        38 => {
                            let _ = self.channels[midi_ch].apply_data_entry_lsb(cc_to_u7(value));
                        }
                        // CC91/93: エフェクト送りレベル。送信チャンネルのeffect_route_slotが
                        // 指すスロットへ適用する（既定はスロット0、`op505-midi`側で
                        // EFFECT_SLOT_COUNT-1へクランプ済み）。
                        91 => self
                            .master
                            .slot_mut(self.channels[midi_ch].effect_route_slot as usize)
                            .set_reverb_send(cc_to_u8(value)),
                        93 => self
                            .master
                            .slot_mut(self.channels[midi_ch].effect_route_slot as usize)
                            .set_chorus_send(cc_to_u8(value)),
                        // CC102: Program Change 代替（VST3ではMidiProgramChangeが届かないため、
                        // `ym38x6-vst`と同じくGM2未定義ブロック先頭のCC102で代替する）。
                        102 => self.apply_program_change(midi_ch, cc_to_u7(value)),
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
                            self.channels[midi_ch].pedal.cc120_reset();
                            self.channels[midi_ch].mono.reset();
                            self.channels[midi_ch].last_note = None;
                        }
                        // CC123(All Notes Off)：通常のNote-Off相当（リリースして自然減衰）。
                        123 => {
                            for note in 0u8..MIDI_NOTE_COUNT {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                            self.channels[midi_ch].pedal.cc123_reset();
                            self.channels[midi_ch].mono.reset();
                            self.channels[midi_ch].last_note = None;
                        }
                        // CC126(Mono Mode On)/CC127(Poly Mode On)：データバイトの値は無視し
                        // （一般的な音源シンセの実装に倣う）、モード切替＋そのチャンネルの
                        // 全ノートを一括note_offする（CC123と同じ全ノートオフ処理。
                        // Poly→Mono/Mono→Polyのどちらの遷移でも、遷移前に何が鳴っていたかに
                        // 関わらずそのチャンネルを一旦静かにしてから始める）。
                        // Mono ON中はサステインペダルより優先する（NoteOff側のコメント参照）。
                        126 | 127 => {
                            self.channels[midi_ch].mono.set_enabled(cc == 126);
                            for note in 0u8..MIDI_NOTE_COUNT {
                                self.engine.note_off(midi_channel_note_id(channel, note));
                            }
                            self.channels[midi_ch].pedal.cc123_reset();
                            self.channels[midi_ch].last_note = None;
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

        // 各MIDIチャンネルのeffect_route_slot（NRPN(0,1)、既定0）に従ってスロット別へ
        // レンダリングし、スロットごとにMasterEffectsを適用後に合算・マスターボリューム/
        // レベル計測を適用する（`sound_core::MasterSection`）。誰もNRPN(0,1)を送らなければ
        // 全チャンネルがslot0へ集まり、単一MasterEffects時代と同じ出力になる。
        let channel_slot: [u8; MIDI_CHANNEL_COUNT] =
            std::array::from_fn(|i| self.channels[i].effect_route_slot);
        let engine = &mut self.engine;
        let mixed = self.master.render(interleaved_len, num_channels, |slot_buf, stride| {
            engine.render_routed(slot_buf, stride, &channel_slot, num_channels);
        });

        let output_slices = buffer.as_slice();
        for ch in 0..num_channels {
            for s in 0..num_samples {
                output_slices[ch][s] += mixed[s * num_channels + ch];
            }
        }

        // レベルメーターのpublish（standaloneのオーディオコールバックと同じ設計、
        // `op505-standalone/src/main.rs`参照）。ピーク検出は毎ブロック行い、取りこぼしを
        // 避けるため区間内の最大値を累積してから`meter_fps`間隔でまとめて送る。
        let m = self.master.output_mut().take_measurement();
        self.meter_peak_l = self.meter_peak_l.max(m.peak_l);
        self.meter_peak_r = self.meter_peak_r.max(m.peak_r);
        self.meter_clipped |= m.clipped;
        self.frames_since_publish += num_samples;
        if self.frames_since_publish >= self.publish_interval_frames {
            self.master_meter.publish(&sound_core::Measurement {
                peak_l: self.meter_peak_l,
                peak_r: self.meter_peak_r,
                clipped: self.meter_clipped,
            });
            self.meter_peak_l = 0.0;
            self.meter_peak_r = 0.0;
            self.meter_clipped = false;
            self.frames_since_publish = 0;
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
