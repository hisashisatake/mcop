// コンソールウィンドウを持たないGUIサブシステムのアプリとしてビルドする（タスクトレイ常駐）。
// これに伴い標準出力ハンドルが無効な環境があり得るため（Explorer起動等）、println!/eprintln!は
// 使わず[`log`]モジュール（ファイル出力）へ統一している（println!は書き込み失敗時にpanicする）。
#![windows_subsystem = "windows"]

//! op505をDomino等のMME専用MIDIシーケンサから直接鳴らすための常駐アプリ。
//!
//! MIDI入力の供給元は[`midi_source::MidiSource`]で抽象化され、複数併存できる
//! （[`sources`]配下が個々の実装）。現状2系統ある。①loopMIDI等が作った既存の仮想MIDI
//! ポート（または実機のUSB-MIDIキーボード）を[`midir`]経由で開く[`sources::midir_src`]。
//! ②[`sources::pipe_src`]が受け付ける名前付きパイプ（`\\.\pipe\op505.mme.v1`）経由の
//! 経路——`op505-mme-driver`（Drivers32方式のユーザーモードMMEドライバDLL、
//! `op505/mme-driver/`）がDomino等のクライアントプロセスに読み込まれ、そこから
//! 転送されてくる。②によりop505自身がWindowsのMIDI OUTデバイス一覧に「op505」として
//! 現れ、loopMIDIの仮想ポート作成を経由せずに直接選択できる。将来のWindows MIDI
//! Services経由の供給元も同じトレイトを実装するだけで足りる設計（詳細はop505/mme-driver/
//! 配下の設計メモ参照）。どの供給元も受け取ったMIDIバイト列を同じキューへ積み、
//! op505-coreへ渡してcpalでWASAPI直出しする。
//!
//! MIDI入力は各供給元のスレッドから[`midi_source::MidiSink::push`]経由で届き、
//! オーディオ処理はcpalのコールバックスレッドで行われる。いずれもキュー
//! （Mutex<VecDeque<Vec<u8>>>）でのみやり取りし、Op505Engine/MasterEffectsは
//! オーディオスレッドが単独で所有する（gesture-appのArc<Mutex<Op505Engine>>と違い、
//! UIスレッドからの同時アクセスが無いため排他制御そのものを避けられる）。
//!
//! CC/NRPN・プログラムチェンジの解釈は`op505-midi`の[`ChannelState`]を使う
//! （`op505-vst`・`op505/tools/smf2op505`に続く3本目の利用側、詳細はspec-fm.md 8章）。
//! GM2リズムチャンネルに対応する（`op505_presets_dir()`配下に`bank=15360+キット番号`の
//! ドラムキットバンクが1つでもあれば、Bank Select MSB(CC0)=120 + Program Changeで該当
//! MIDIチャンネルがリズムチャンネルになる。専用のCLIオプションは無く、プリセット
//! ディレクトリに置くだけでよい。判定・アドレス解決は`op505_midi::ChannelProgramState`参照）。
//! 対応イベント: Note On/Off・Program Change・Pitch Bend・Channel/Poly Pressure・
//! CC0/1/2/4/7/10/11/32/64/66/67/71/72/73/74/75/76/77/78/91/93/98〜101/103〜106/120/121/123、
//! RPN(0,0)/(0,5)、NRPN(0,1)〜(0,21)/(0,34)/(0,35)。NRPN(0,1) Channel Effect Routeは
//! 送信チャンネルの音声・エフェクト設定NRPN(0,2)〜(0,8)・CC91/93の適用先エフェクトスロットを
//! 選択する（既定はスロット0、詳細はspec-fm.md 8章）。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use op505_core::{op505_presets_dir, Op505Engine, Op505Patch, Op505PresetBank};
use op505_midi::{
    cc_byte_to_u7 as cc_to_u7, cc_byte_to_u8 as cc_to_u8, released_notes, ChannelState, DataEntryOutcome,
    MonoNoteOff, MonoNoteOn, ProgramSelection, RHYTHM_BANK_RANGE,
};
use sound_core::{cc76_to_rate_scale, ChorusType, MasterSection, ReverbType, Vco};

mod config;
mod editor;
mod log;
mod midi_source;
mod shared;
mod sources;
mod tray;

use shared::SharedEditState;

type MidiQueue = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// MIDIノート番号の総数（0〜127）。発音中ボイス走査ループの上限に使う
/// （`op505-vst`/`op505/tools/smf2op505`の`MIDI_NOTE_COUNT`と同じ）。
const MIDI_NOTE_COUNT: usize = 128;

/// エフェクトスロット数。`op505_midi::EFFECT_SLOT_COUNT`（クランプ境界の一元管理元）と揃える。
const EFFECT_SLOT_COUNT: usize = op505_midi::EFFECT_SLOT_COUNT as usize;

/// MIDIチャンネル(0〜15)とノート番号からVco::note_on/note_offの`channel`引数(ボイスID)を作る。
/// `op505/tools/smf2op505`のrender.rsと同じ規約（`channel_index * 128 + note`）に揃えている。
fn voice_id(channel: usize, note: u8) -> usize {
    channel * 128 + note as usize
}

/// MIDIノート番号(0〜127)を周波数(Hz)に変換する。A4=69=440Hzの平均律。
fn note_to_frequency(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// CC7(Channel Volume) と CC11(Expression) の値（0〜127）から GM2 準拠のゲインを計算する
/// （`op505-vst`/`smf2op505`と同一式。`op505-midi`には置かない小関数のためここでも複製する）。
#[inline]
fn channel_gain(cc7: u8, cc11: u8) -> f32 {
    let v7 = cc7 as f32 / 127.0;
    let v11 = cc11 as f32 / 127.0;
    v7 * v7 * v11 * v11
}

/// MIDI入力・音色バンク・16チャンネル分のCC/NRPNシャドウ状態をまとめて持つ。
/// オーディオコールバックスレッドが単独で所有する（Step 1のコメント参照）。
struct MidiState {
    channels: Vec<ChannelState>,
    /// `op505_presets_dir()`から読み込んだ全プリセット。Program Changeで(bank,program)を引く。
    presets: Op505PresetBank,
    /// `presets`に該当(bank,program)が無いときのフォールバック（起動時の既定パッチ）。
    default_patch: Op505Patch,
    /// トレイ起動音色エディタが編集中のパッチ（`shared::SharedEditState`から`try_read()`で
    /// 取り込んだキャッシュ）。`edit_channel`が`Some`のときだけ`base_patch_for`から参照される。
    edit_patch: Op505Patch,
    /// エディタが編集対象としているMIDIチャンネル。`None`（エディタ未使用時は常にこれ）なら
    /// 全チャンネルが従来どおりProgram Change解決のみで音色を決める。
    edit_channel: Option<usize>,
}

impl MidiState {
    fn new(presets: Op505PresetBank, default_patch: Op505Patch) -> Self {
        // `presets`（`op505_presets_dir()`配下の全`.op505`）にリズムキットバンク
        // （bank=15360+キット番号）が1件でもあれば、ch10は起動時からドラムモードONで始まる
        // （`ChannelProgramState::new`参照。専用CLIオプションは無く、プリセットディレクトリに
        // 置くだけで有効になる）。
        let rhythm_kits_available = presets.has_bank_in(RHYTHM_BANK_RANGE);
        if rhythm_kits_available {
            log::log("GM2リズムキットを検出。ch10は起動時からドラムモードで開始します。");
        }
        let channels = (0..16).map(|chi| ChannelState::new(chi, rhythm_kits_available)).collect();
        Self { channels, presets, default_patch, edit_patch: default_patch, edit_channel: None }
    }

    /// このチャンネル・ノートの現在のProgram Change選択が指すベースパッチ。
    /// - エディタ編集中: `edit_channel`が`chi`を指していれば、Program Change選択を無視して
    ///   `edit_patch`を返す（トレイ起動音色エディタでノブを操作した音をそのチャンネルで
    ///   試聴するための経路。エディタ未使用時は`edit_channel`が常に`None`のためこの分岐に
    ///   入らず、以降の判定は無改造）。
    /// - 旋律: `presets`に該当(bank,program)が無ければ`default_patch`にフォールバック。
    /// - リズム: `presets`のkit内に該当ノートが無ければkit0へフォールバックし、それも無ければ
    ///   `None`（このノートは発音しない。GM2実機でキット内に未定義のノートが無音になるのと同じ、
    ///   `op505/tools/smf2op505`の`base_patch_for`と同じ意味論）。
    fn base_patch_for(&self, chi: usize, note: u8) -> Option<Op505Patch> {
        if self.edit_channel == Some(chi) {
            return Some(self.edit_patch);
        }
        match self.channels[chi].program_state.selection() {
            ProgramSelection::Melodic { bank, program } => {
                Some(self.presets.get(bank, program).map(|p| p.patch).unwrap_or(self.default_patch))
            }
            ProgramSelection::Rhythm { .. } => {
                let program_state = &self.channels[chi].program_state;
                let (b, p) = program_state.lookup_address(note);
                self.presets
                    .get(b, p)
                    .or_else(|| program_state.rhythm_fallback_address(note).and_then(|(fb, fp)| self.presets.get(fb, fp)))
                    .map(|preset| preset.patch)
            }
        }
    }
}

fn main() {
    let midi_queue: MidiQueue = Arc::new(Mutex::new(VecDeque::new()));
    let sink = midi_source::MidiSink::new(Arc::clone(&midi_queue));
    let mut registry = midi_source::SourceRegistry::new();

    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    let supported = device.default_output_config().expect("no default output config");
    let num_channels = supported.channels() as usize;
    let sample_rate = supported.sample_rate().0 as f32;
    let stream_config: cpal::StreamConfig = supported.into();

    let mut engine = Op505Engine::new(sample_rate);
    // 各MIDIチャンネルのeffect_route_slot（NRPN(0,1)、既定0）が指すスロットへルーティングし、
    // 合算後にマスターボリューム/レベル計測を適用する（`sound_core::MasterSection`、
    // スロット配列・スクラッチ確保・合算ループを一本化した共通実装）。
    let mut master = MasterSection::new(sample_rate, EFFECT_SLOT_COUNT);
    // トレイ起動音色エディタ（Step 1以降）が発音中ボイスへ即時反映する際の、発音中ボイスID
    // 一覧のスクラッチバッファ（`slot_buf`と同じ理由でオーディオスレッドの反復ヒープ確保を
    // 避ける）。エディタが無い/未操作の間は`apply_live_active`自体が呼ばれないため使われない。
    let mut active_ids: Vec<usize> = Vec::new();
    let (presets, default_patch) = load_presets();
    engine.set_patch(default_patch);
    let mut state = MidiState::new(presets.clone(), default_patch);
    // トレイ起動音色エディタとの共有状態（Step 1以降）。エディタが一度も開かれなければ
    // 全dirtyフラグがfalseのままで、`sync_editor_state`は即座に返り既存挙動を変えない。
    let shared_edit_state = Arc::new(SharedEditState::new(default_patch, presets));
    // エディタスレッドはこのクローンを持つ（オーディオコールバックへは別クローンをmoveする）。
    // `sink.clone()`はエディタ下部の鍵盤が試聴用MIDIを積むために使う（実MIDI入力と同じキュー）。
    let editor_handle = editor::EditorHandle::spawn(Arc::clone(&shared_edit_state), sink.clone());

    // midirソース（実機キーボード/loopMIDI）はタスクトレイのメニューから動的に
    // 切り替えられる必要があるため、SourceRegistryへは入れず`tray::run`が
    // 自分で所有・管理する（初回接続もそちら側で行う。設定ファイルの読み込みも同様）。
    // OpenEditorフレーム（kind=3、gesture-appのEキー押下）を受けたら`editor_handle`の
    // クローンで`show()`する（editor_handleが必要なため`pipe_src::spawn`はここまで遅延させる）。
    registry.add(Box::new(sources::pipe_src::spawn(sink.clone(), editor_handle.clone())));

    let stream = device
        .build_output_stream::<f32, _, _>(
            &stream_config,
            move |output: &mut [f32], _| {
                sync_editor_state(&shared_edit_state, &mut engine, &mut master, &mut state, &mut active_ids);
                drain_midi_queue(&midi_queue, &mut engine, &mut master, &mut state);
                output.fill(0.0);

                let interleaved_len = output.len();
                let channel_slot: [u8; 16] = std::array::from_fn(|i| state.channels[i].effect_route_slot);
                let engine_ref = &mut engine;
                let mixed = master.render(interleaved_len, num_channels, |slot_buf, stride| {
                    engine_ref.render_routed(slot_buf, stride, &channel_slot, num_channels);
                });
                for (o, v) in output.iter_mut().zip(mixed.iter()) {
                    *o += v;
                }
            },
            |err| log::log(&format!("audio error: {err}")),
            None,
        )
        .expect("failed to build output stream");

    stream.play().expect("failed to start audio stream");

    log::log("op505-standalone: 再生中。");
    tray::run(sink, editor_handle);
}

/// `op505_presets_dir()`から全プリセットを読み込み、その先頭を起動時の既定パッチとして返す。
/// `Op505Patch::default()`は全段level=0/time=0のEGで無音になるため、プリセットが1つも
/// 見つからない場合は警告を出す。
fn load_presets() -> (Op505PresetBank, Op505Patch) {
    let dir = op505_presets_dir();
    let presets = Op505PresetBank::load_from_dir(&dir);
    match presets.sorted_entries().into_iter().next() {
        Some(((bank, program), preset)) => {
            log::log(&format!("既定パッチ: {} (bank={bank}, program={program})", preset.name));
            (presets.clone(), preset.patch)
        }
        None => {
            log::log(&format!(
                "警告: {} にプリセットが見つかりません。無音のデフォルトパッチのまま起動します。",
                dir.display()
            ));
            (presets, Op505Patch::default())
        }
    }
}

/// トレイ起動音色エディタ（Step 1以降）からの入力をオーディオスレッドへ取り込む。
/// `drain_midi_queue`より先に呼ぶこと（`op505-vst`の`process()`が`try_read()`を最初に
/// 処理するのと同じ順序、`shared.rs`のモジュールdoc参照）。
///
/// エディタが一度も操作しない限り（`shared`の3つのdirtyフラグが全てfalseのまま）、この関数は
/// `edit_channel()`と3回の`swap`だけで即座に返る——命令列も演算も既存挙動から変わらない。
fn sync_editor_state(
    shared: &SharedEditState,
    engine: &mut Op505Engine,
    master: &mut MasterSection,
    state: &mut MidiState,
    active_ids: &mut Vec<usize>,
) {
    state.edit_channel = shared.edit_channel();

    if let Some(patch) = shared.take_patch_if_dirty() {
        state.edit_patch = patch;
        if let Some(chi) = state.edit_channel {
            apply_live_active(engine, state, chi, active_ids);
        }
    }

    if let Some(presets) = shared.take_presets_if_dirty() {
        state.presets = presets;
    }

    if let Some((slot, values)) = shared.take_fx_if_dirty() {
        let fx = master.slot_mut((slot as usize).min(EFFECT_SLOT_COUNT - 1));
        fx.set_reverb_send(values[shared::FX_REVERB_SEND]);
        fx.set_reverb_type(ReverbType::from_u8(values[shared::FX_REVERB_TYPE]));
        fx.set_reverb_time(values[shared::FX_REVERB_TIME]);
        fx.set_chorus_send(values[shared::FX_CHORUS_SEND]);
        fx.set_chorus_type(ChorusType::from_u8(values[shared::FX_CHORUS_TYPE]));
        fx.set_chorus_mod_rate(values[shared::FX_CHORUS_MOD_RATE]);
        fx.set_chorus_mod_depth(values[shared::FX_CHORUS_MOD_DEPTH]);
        fx.set_chorus_feedback(values[shared::FX_CHORUS_FEEDBACK]);
        fx.set_chorus_send_to_reverb(values[shared::FX_CHORUS_SEND_TO_REVERB]);
    }
}

/// キューに溜まったMIDIメッセージを全て取り出し、エンジンへ適用する。
fn drain_midi_queue(
    queue: &MidiQueue,
    engine: &mut Op505Engine,
    master: &mut MasterSection,
    state: &mut MidiState,
) {
    let mut pending = queue.lock().unwrap();
    while let Some(bytes) = pending.pop_front() {
        handle_midi_message(engine, master, state, &bytes);
    }
}

/// 1つのMIDIメッセージを解釈する。メッセージ長はステータスバイトごとに異なる
/// （Note On/Off・Pitch Bend・CCは3バイト、Program Change・Channel Pressureは2バイト）ため、
/// 固定長スライスパターンではなくステータス別に必要な長さを都度チェックする。
fn handle_midi_message(
    engine: &mut Op505Engine,
    master: &mut MasterSection,
    state: &mut MidiState,
    bytes: &[u8],
) {
    let &[status, ..] = bytes else { return };
    let chi = (status & 0x0F) as usize;

    match status & 0xF0 {
        0x80 => {
            let &[_, note, _vel] = bytes else { return };
            note_off_voice(engine, state, chi, note);
        }
        0x90 => {
            let &[_, note, vel] = bytes else { return };
            if vel == 0 {
                note_off_voice(engine, state, chi, note);
            } else {
                note_on_voice(engine, state, chi, note, vel);
            }
        }
        0xA0 => {
            let &[_, note, value] = bytes else { return };
            state.channels[chi].poly_pressure[note as usize] = cc_to_u8(value);
            apply_live(engine, state, chi);
        }
        0xB0 => {
            let &[_, cc, val] = bytes else { return };
            handle_control_change(engine, master, state, chi, cc, val);
        }
        0xC0 => {
            let &[_, program] = bytes else { return };
            state.channels[chi].program_change(program & 0x7f);
        }
        0xD0 => {
            let &[_, value] = bytes else { return };
            state.channels[chi].channel_pressure = cc_to_u8(value);
            apply_live(engine, state, chi);
        }
        0xE0 => {
            let &[_, lsb, msb] = bytes else { return };
            let raw = (((msb as i32) << 7) | lsb as i32) - 8192;
            let cents = raw as f32 / 8192.0 * state.channels[chi].pitch_bend_range * 100.0;
            state.channels[chi].bend_cents = cents;
            engine.set_pitch_bend_group(chi, cents);
        }
        _ => {}
    }
}

/// 1ボイスの実際の発音処理（ペダル・Mono状態の更新は含まない）。現在のProgram Changeが
/// 指す音色へ、NRPN上書き・CC2/CC4/AT/Pitch FG演奏補正/Soft Pedalを重ねた実効パッチで
/// 発音する（`op505-vst`/`smf2op505`のnote_on適用と同型）。`base_patch_for`がNoneなら
/// 発音しない（リズムキット内に未定義のノート）。
///
/// Mono Modeのlast-note priorityフォールバック（[`MonoNoteOff::Fallback`]）からも直接
/// 呼べるよう、ペダル状態の更新（`pedal.note_on`）は呼び出し側で行う（フォールバック
/// 再発音は「新規の押鍵」ではないため、CC67(Soft Pedal)のsoft_notes判定を押鍵時点の
/// 状態のまま動かしたい）。
fn note_on_voice_core(engine: &mut Op505Engine, state: &mut MidiState, chi: usize, note: u8, vel: u8) {
    let Some(base) = state.base_patch_for(chi, note) else { return };
    let id = voice_id(chi, note);
    let freq = note_to_frequency(note);
    {
        let st = &state.channels[chi];
        let mut eff = st.build_effective_patch(&base);
        st.apply_note_post_processing(&mut eff, note);
        engine.set_patch(eff);
        engine.note_on(id, freq, vel);
        engine.set_channel_volume(id, channel_gain(st.cc7, st.cc11));
        engine.set_channel_pan(id, st.pan_gains());
        engine.set_pitch_bend(id, st.total_pitch_bend_cents());
        engine.set_pitch_fg_rate_scale(id, cc76_to_rate_scale(st.pitch_fg_cc76));
        for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
            if let Some(f_number) = f {
                engine.set_operator_f_number(id, op_index, *f_number);
            }
        }
    }
    if let Some((from_note, seconds)) = state.channels[chi].glide_source(note) {
        engine.start_glide(id, note_to_frequency(from_note), seconds);
    }
    state.channels[chi].last_note = Some(note);
}

/// 外部からの実際のNote On（MIDI入力）。ペダル状態の更新とMono Modeの処理（前の音の解放、
/// またはCC65 ON+レガート時はボイスを継続したままピッチだけ滑らせるレガート、詳細は
/// op505-midiのmono.rsモジュールdoc）を行ってから[`note_on_voice_core`]を呼ぶ。
fn note_on_voice(engine: &mut Op505Engine, state: &mut MidiState, chi: usize, note: u8, vel: u8) {
    state.channels[chi].pedal.note_on(note);
    if state.channels[chi].mono.enabled {
        let portamento = state.channels[chi].portamento_on;
        match state.channels[chi].mono.note_on(note, vel, portamento) {
            MonoNoteOn::Legato { voice } => {
                let seconds = state.channels[chi].portamento_seconds();
                let id = voice_id(chi, voice);
                if engine.glide_to(id, note_to_frequency(note), seconds) {
                    state.channels[chi].last_note = Some(note);
                    return; // 再アタックしない
                }
                // ボイスが既にIdle等で消えていたら通常発音へフォールバックする。
                state.channels[chi].mono.demote_legato(note);
            }
            MonoNoteOn::Retrigger { release } => {
                if let Some(prev) = release {
                    engine.note_off(voice_id(chi, prev));
                }
            }
        }
    }
    note_on_voice_core(engine, state, chi, note, vel);
}

/// ペダル保持中でなければ即座にNote Offする（保持中なら`pending_release`へ回るのみ）。
/// Mono Mode中はサステインペダルより優先する（押鍵状態＝`MonoState`だけで解放/
/// フォールバックを決める。理由は`handle_control_change`のCC126/127コメント参照）。
fn note_off_voice(engine: &mut Op505Engine, state: &mut MidiState, chi: usize, note: u8) {
    state.channels[chi].poly_pressure[note as usize] = 0;
    let pedal_released = state.channels[chi].pedal.note_off(note);
    if state.channels[chi].mono.enabled {
        let portamento = state.channels[chi].portamento_on;
        match state.channels[chi].mono.note_off(note, portamento) {
            MonoNoteOff::Nothing => {}
            MonoNoteOff::Release(released_note) => {
                engine.note_off(voice_id(chi, released_note));
            }
            MonoNoteOff::Fallback { release, sound, velocity } => {
                engine.note_off(voice_id(chi, release));
                note_on_voice_core(engine, state, chi, sound, velocity);
            }
            MonoNoteOff::LegatoFallback { voice, sound, velocity } => {
                let seconds = state.channels[chi].portamento_seconds();
                let id = voice_id(chi, voice);
                if !engine.glide_to(id, note_to_frequency(sound), seconds) {
                    // ボイスが既にIdle等で消えていたら通常発音へフォールバックする。
                    state.channels[chi].mono.demote_legato(sound);
                    engine.note_off(id);
                    note_on_voice_core(engine, state, chi, sound, velocity);
                }
            }
        }
    } else if pedal_released {
        engine.note_off(voice_id(chi, note));
    }
}

/// CC/NRPNで変わった実効パッチ・CC76 rate_scale・AT・OP F-Numberを、`notes`が列挙する
/// ノート番号（そのチャンネルの）へ伝播する共通本体。`apply_live`（CC経路、`0..128`昇順を
/// 維持——エンジンがボイスID昇順の決定論的処理順序に依存するため）と`apply_live_active`
/// （エディタ経路、発音中ボイスのみ）が呼び分ける。
///
/// リズムチャンネルはノートごとに音色が違うため、`base_patch_for`をノートごとに呼ぶ
/// （旋律のようにチャンネル全体で1回だけ計算する最適化はできない）。キット内に未定義の
/// ノート（`base_patch_for`がNone）はスキップし、既存の発音中パラメーターをそのまま維持する。
fn apply_live_notes(engine: &mut Op505Engine, state: &MidiState, chi: usize, notes: impl Iterator<Item = u8>) {
    let rate_scale = cc76_to_rate_scale(state.channels[chi].pitch_fg_cc76);
    for note_u8 in notes {
        let Some(base) = state.base_patch_for(chi, note_u8) else { continue };
        let st = &state.channels[chi];
        let eff = st.build_effective_patch(&base);
        let id = voice_id(chi, note_u8);
        let mut note_patch = eff;
        st.apply_note_post_processing(&mut note_patch, note_u8);
        engine.set_channel_params(id, note_patch.channel);
        for (op_index, op) in note_patch.operators.iter().enumerate() {
            engine.set_operator_params(id, op_index, *op);
        }
        engine.set_pitch_fg_rate_scale(id, rate_scale);
        engine.set_channel_pan(id, st.pan_gains());
        // RPN(0,1)/(0,2) Channel Fine/Coarse Tuning。他の全NRPN補正と同じく無条件に毎回
        // 再送する（`total_pitch_bend_cents`＝bend_cents+tune_cents）。
        engine.set_pitch_bend(id, st.total_pitch_bend_cents());
        for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
            if let Some(f_number) = f {
                engine.set_operator_f_number(id, op_index, *f_number);
            }
        }
    }
}

/// CC/NRPN経路（`handle_control_change`等）から呼ぶ。全128ノートを昇順で走査する
/// （既存挙動を一切変えない、`op505/tools/smf2op505`のapply_liveと同じ役割）。
fn apply_live(engine: &mut Op505Engine, state: &mut MidiState, chi: usize) {
    apply_live_notes(engine, state, chi, 0..MIDI_NOTE_COUNT as u8);
}

/// トレイ起動音色エディタ経路（Step 1以降）から呼ぶ。エディタのノブ操作は毎フレーム
/// 起こり得るため、`chi`で発音中の実ボイスのみへ絞って`apply_live_notes`を呼ぶ
/// （16ch×128ノート全走査は重すぎる）。`ids`は呼び出し側（cpalコールバック）が
/// 使い回すスクラッチバッファ（`slot_buf`と同じ理由でオーディオスレッドの反復
/// ヒープ確保を避ける）。
fn apply_live_active(engine: &mut Op505Engine, state: &mut MidiState, chi: usize, ids: &mut Vec<usize>) {
    engine.collect_active_channels(ids);
    let notes = ids.iter().filter(|id| **id >> 7 == chi).map(|id| (id & 0x7f) as u8);
    apply_live_notes(engine, state, chi, notes);
}

/// 1つのコントロールチェンジを処理する。`op505/tools/smf2op505`のhandle_control_changeを
/// 移植したもの（ドラムキット/複数バンクの`bank`/`drums`引数は無く、`MidiState`が代わりに
/// Program Change選択済みの単一プリセット集合を持つ）。
fn handle_control_change(
    engine: &mut Op505Engine,
    master: &mut MasterSection,
    state: &mut MidiState,
    chi: usize,
    cc: u8,
    val: u8,
) {
    match cc {
        // CC7/CC11: GM2音量。実効ゲイン=(cc7/127)²×(cc11/127)²。発音中へ即時反映。
        7 => {
            state.channels[chi].cc7 = cc_to_u7(val);
            let gain = channel_gain(state.channels[chi].cc7, state.channels[chi].cc11);
            engine.set_channel_volume_group(chi, gain);
        }
        11 => {
            state.channels[chi].cc11 = cc_to_u7(val);
            let gain = channel_gain(state.channels[chi].cc7, state.channels[chi].cc11);
            engine.set_channel_volume_group(chi, gain);
        }
        // CC1/76/77/78: Pitch FG 演奏補正 → 発音中ボイスへ伝播。
        1 => {
            state.channels[chi].pitch_fg_cc1 = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        76 => {
            state.channels[chi].pitch_fg_cc76 = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        77 => {
            state.channels[chi].pitch_fg_cc77 = cc_to_u8(val);
            apply_live(engine, state, chi);
        }
        78 => {
            state.channels[chi].pitch_fg_cc78 = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        // CC92(Tremolo Depth)：Gain FG Depthへの0起点加算（CC77と同型、RPN連動レンジは無い）。
        92 => {
            state.channels[chi].gain_fg_cc92 = cc_to_u8(val);
            apply_live(engine, state, chi);
        }
        // CC2(ブレス)/CC4(フット): Expression Destination（NRPN(0,34)/(0,35)）へ加算 → 発音中ボイスへ伝播。
        2 => {
            state.channels[chi].cc2 = cc_to_u8(val);
            apply_live(engine, state, chi);
        }
        4 => {
            state.channels[chi].cc4 = cc_to_u8(val);
            apply_live(engine, state, chi);
        }
        // CC10(Pan): ボイス単位の左右ゲイン（patchではなくVco::set_channel_pan_group経由、
        // コンスタントパワー則）。CC7/CC11と同じく発音中へ即時反映する。
        10 => {
            state.channels[chi].cc10_pan = cc_to_u7(val);
            engine.set_channel_pan_group(chi, state.channels[chi].pan_gains());
        }
        // CC71(Resonance)/CC72(Release Time)/CC73(Attack Time)/CC74(Brightness)/
        // CC75(Decay Time): `op505_midi::apply_sound_controllers`参照。値を保持し発音中へ伝播する。
        71 => {
            state.channels[chi].cc71_resonance = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        72 => {
            state.channels[chi].cc72_release = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        73 => {
            state.channels[chi].cc73_attack = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        74 => {
            state.channels[chi].cc74_brightness = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        75 => {
            state.channels[chi].cc75_decay = cc_to_u7(val);
            apply_live(engine, state, chi);
        }
        // CC5(Portamento Time)/CC65(Portamento On/Off): 次のnote_onでのグライドに使う
        // だけなので、発音中ボイスへの即時反映（apply_live）は不要。
        5 => {
            state.channels[chi].portamento_time = cc_to_u7(val);
        }
        65 => {
            state.channels[chi].portamento_on = cc_to_u7(val) >= 64;
        }
        // NRPN/RPN 選択（CC99/98=NRPN MSB/LSB、CC101/100=RPN MSB/LSB）
        98 => state.channels[chi].rpn.set_nrpn_lsb(cc_to_u7(val)),
        99 => state.channels[chi].rpn.set_nrpn_msb(cc_to_u7(val)),
        100 => state.channels[chi].rpn.set_rpn_lsb(cc_to_u7(val)),
        101 => state.channels[chi].rpn.set_rpn_msb(cc_to_u7(val)),
        // CC6 Data Entry MSB: 選択中の RPN/NRPN へ値を適用。エフェクト系NRPN(Reverb/Chorus)は
        // op505-midiがsound-core型を扱えないため`DataEntryOutcome::Effect`で返り、ここで
        // 送信チャンネルのeffect_route_slotが指すMasterEffectsへ適用する。
        6 => match state.channels[chi].apply_data_entry(val) {
            DataEntryOutcome::StateChanged { voice_update } => {
                if voice_update {
                    apply_live(engine, state, chi);
                }
            }
            DataEntryOutcome::Effect(slot, target, value) => {
                let fx = master.slot_mut((slot as usize).min(EFFECT_SLOT_COUNT - 1));
                sound_midi::apply_effect_control(fx, target, value);
            }
        },
        // CC38 Data Entry LSB: OP F-Number(NRPN 0,18〜21選択中)の下位7bit。
        38 => {
            if state.channels[chi].apply_data_entry_lsb(val) {
                apply_live(engine, state, chi);
            }
        }
        // CC64 Sustain Pedal（ホールドフラグ方式）。
        64 => {
            let released = state.channels[chi].pedal.cc64(val);
            for note in released_notes(released) {
                engine.note_off(voice_id(chi, note));
            }
        }
        // CC66 Sostenuto: ON時点でkeys_down中のノートのみをlatchし、CC66 OFF（かつ
        // CC64も踏まれていない）までReleaseに入らせない。
        66 => {
            let released = state.channels[chi].pedal.cc66(val);
            for note in released_notes(released) {
                engine.note_off(voice_id(chi, note));
            }
        }
        // CC67 Soft Pedal: 深さを保持するのみ。ON中に新規キーオンしたノートのみへの
        // 適用はnote_on_voice/apply_live側（soft_notesビット）で行う。
        67 => {
            state.channels[chi].pedal.cc67(cc_to_u7(val));
        }
        // CC121 Reset All Controllers: ③ジェスチャー層のみリセットする（②パート状態・
        // ①音色は保持）。program_state（Program Change選択）へは意図的に触れない。
        121 => {
            let released = state.channels[chi].reset_all_controllers();
            for note in released_notes(released) {
                engine.note_off(voice_id(chi, note));
            }
            engine.set_pitch_bend_group(chi, 0.0);
            apply_live(engine, state, chi);
        }
        // CC91/93: エフェクト送りレベル。送信チャンネルのeffect_route_slotが指すスロットへ適用。
        91 => master.slot_mut(state.channels[chi].effect_route_slot as usize).set_reverb_send(cc_to_u8(val)),
        93 => master.slot_mut(state.channels[chi].effect_route_slot as usize).set_chorus_send(cc_to_u8(val)),
        // CC103〜106: Operator Key On/Off（≧64でキーオン/<64でキーオフ、全OP独立）。
        103..=106 => {
            let op_index = (cc - 103) as usize;
            let key_on = cc_to_u7(val) >= 64;
            for note in 0..MIDI_NOTE_COUNT {
                let id = voice_id(chi, note as u8);
                if key_on {
                    engine.note_on_operator(id, op_index);
                } else {
                    engine.note_off_operator(id, op_index);
                }
            }
        }
        // CC120 All Sound Off: リリースを経ず即座に消音する（GM2準拠、CC123のリリース
        // とは区別する）。
        120 => {
            engine.silence_group(chi);
            state.channels[chi].pedal.cc120_reset();
            state.channels[chi].mono.reset();
            state.channels[chi].last_note = None;
        }
        // CC123 All Notes Off: 通常のNote-Off相当（リリースして自然減衰）。
        123 => {
            for note in 0..MIDI_NOTE_COUNT {
                engine.note_off(voice_id(chi, note as u8));
            }
            state.channels[chi].pedal.cc123_reset();
            state.channels[chi].mono.reset();
            state.channels[chi].last_note = None;
        }
        // CC126 Mono Mode On / CC127 Poly Mode On: データバイトの値は無視し（一般的な
        // 音源シンセの実装に倣う）、モード切替＋そのチャンネルの全ノートを一括note_off
        // する（CC123と同じ全ノートオフ処理。Poly→Mono/Mono→Polyのどちらの遷移でも、
        // 遷移前に何が鳴っていたかに関わらずそのチャンネルを一旦静かにしてから始める）。
        // Mono ON中はサステインペダルより優先する（`note_off_voice`参照）。
        126 | 127 => {
            state.channels[chi].mono.set_enabled(cc == 126);
            for note in 0..MIDI_NOTE_COUNT {
                engine.note_off(voice_id(chi, note as u8));
            }
            state.channels[chi].pedal.cc123_reset();
            state.channels[chi].last_note = None;
        }
        // Bank Select（CC0=MSB, CC32=LSB）：これだけでは音色は切り替わらない
        // （次のProgram Changeで確定する）。GM2リズムキット未読み込みのため、MSB=120を
        // 送ってもis_rhythmには実際には入らない（`ChannelProgramState`参照）。
        0 => state.channels[chi].program_state.bank_select_msb(cc_to_u7(val)),
        32 => state.channels[chi].program_state.bank_select_lsb(cc_to_u7(val)),
        _ => {}
    }
}

// MIDI入力ポートへの接続・選択は sources::midir_src（旧connect_midi_input/select_port_index、
// コマンドライン引数+stdin対話）へ移動した。設定ファイル（config.rs）+
// 「ポートが1個だけなら自動選択」方式に置き換え、タスクトレイ化後もstdin無しで動く。

#[cfg(test)]
mod base_patch_for_tests {
    use super::*;

    /// プリセット無し（`Op505PresetBank::default()`）の`MidiState`。Program Change未送信の
    /// 全チャンネルは`ProgramSelection::Melodic { bank: 0, program: 0 }`のまま
    /// （`ChannelProgramState::new`参照）、該当プリセットも無いため`default_patch`へ
    /// フォールバックする。
    fn state_with_default_patch(default_patch: Op505Patch) -> MidiState {
        MidiState::new(Op505PresetBank::default(), default_patch)
    }

    fn patch_with_pitch_fg_depth(depth: u8) -> Op505Patch {
        let mut patch = Op505Patch::default();
        patch.channel.pitch_fg.depth = depth;
        patch
    }

    #[test]
    fn edit_channel_none_uses_program_change_resolution() {
        let default_patch = patch_with_pitch_fg_depth(10);
        let state = state_with_default_patch(default_patch);
        assert_eq!(state.edit_channel, None);
        let resolved = state.base_patch_for(0, 60).expect("no rhythm kit, must resolve to default_patch");
        assert_eq!(resolved.channel.pitch_fg.depth, 10, "edit_channel=Noneでは従来のPC解決のまま");
    }

    #[test]
    fn edit_channel_some_bypasses_program_change_for_that_channel() {
        let default_patch = patch_with_pitch_fg_depth(10);
        let mut state = state_with_default_patch(default_patch);
        state.edit_patch = patch_with_pitch_fg_depth(99);
        state.edit_channel = Some(3);

        let resolved = state.base_patch_for(3, 60).expect("edit_channel=Some(3)はedit_patchを返す");
        assert_eq!(resolved.channel.pitch_fg.depth, 99, "編集対象chはedit_patchで上書きされるはず");
    }

    #[test]
    fn edit_channel_some_does_not_affect_other_channels() {
        let default_patch = patch_with_pitch_fg_depth(10);
        let mut state = state_with_default_patch(default_patch);
        state.edit_patch = patch_with_pitch_fg_depth(99);
        state.edit_channel = Some(3);

        let resolved = state.base_patch_for(4, 60).expect("編集対象外chは従来のPC解決のまま");
        assert_eq!(resolved.channel.pitch_fg.depth, 10, "編集対象外chはedit_patchの影響を受けないはず");
    }
}
