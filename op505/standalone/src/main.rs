//! op505をDomino等のMME専用MIDIシーケンサから直接鳴らすための常駐アプリ。
//!
//! このアプリ自身は新規の仮想MIDIポートを作らない。loopMIDI等が作った既存の仮想MIDI
//! ポート（または実機のUSB-MIDIキーボード）をmidir経由で開き、受け取ったMIDIバイト列を
//! op505-coreへ渡してcpalでWASAPI直出しする。
//!
//! MIDI入力はmidirのコールバックスレッドから届き、オーディオ処理はcpalのコールバック
//! スレッドで行われる。両者はキュー（Mutex<VecDeque<Vec<u8>>>）でのみやり取りし、
//! Op505Engine/MasterEffectsはオーディオスレッドが単独で所有する
//! （gesture-appのArc<Mutex<Op505Engine>>と違い、UIスレッドからの同時アクセスが
//! 無いため排他制御そのものを避けられる）。
//!
//! CC/NRPN・プログラムチェンジの解釈は`op505-midi`の[`ChannelState`]を使う
//! （`op505-vst`・`op505/tools/smf2op505`に続く3本目の利用側、詳細はspec-fm.md 8章）。
//! GM2リズムチャンネルに対応する（`op505_presets_dir()`配下に`bank=15360+キット番号`の
//! ドラムキットバンクが1つでもあれば、Bank Select MSB(CC0)=120 + Program Changeで該当
//! MIDIチャンネルがリズムチャンネルになる。専用のCLIオプションは無く、プリセット
//! ディレクトリに置くだけでよい。判定・アドレス解決は`op505_midi::ChannelProgramState`参照）。
//! 対応イベント: Note On/Off・Program Change・Pitch Bend・Channel/Poly Pressure・
//! CC0/1/2/4/7/11/32/64/66/67/76/77/78/91/93/98〜101/103〜106/120/121/123、RPN(0,0)/(0,5)、
//! NRPN(0,2)〜(0,21)/(0,34)/(0,35)。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use midir::{MidiInput, MidiInputConnection};
use op505_core::{op505_presets_dir, Op505Engine, Op505Patch, Op505PresetBank};
use op505_midi::{
    cc_byte_to_u7 as cc_to_u7, cc_byte_to_u8 as cc_to_u8, released_notes, ChannelState, DataEntryOutcome,
    EffectControlTarget, ProgramSelection, RHYTHM_BANK_RANGE,
};
use sound_core::{cc76_to_rate_scale, AudioProcessor, ChorusType, MasterEffects, ReverbType, Vco};

type MidiQueue = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// MIDIノート番号の総数（0〜127）。発音中ボイス走査ループの上限に使う
/// （`op505-vst`/`op505/tools/smf2op505`の`MIDI_NOTE_COUNT`と同じ）。
const MIDI_NOTE_COUNT: usize = 128;

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
}

impl MidiState {
    fn new(presets: Op505PresetBank, default_patch: Op505Patch) -> Self {
        // `presets`（`op505_presets_dir()`配下の全`.op505`）にリズムキットバンク
        // （bank=15360+キット番号）が1件でもあれば、ch10は起動時からドラムモードONで始まる
        // （`ChannelProgramState::new`参照。専用CLIオプションは無く、プリセットディレクトリに
        // 置くだけで有効になる）。
        let rhythm_kits_available = presets.has_bank_in(RHYTHM_BANK_RANGE);
        if rhythm_kits_available {
            println!("GM2リズムキットを検出。ch10は起動時からドラムモードで開始します。");
        }
        let channels = (0..16).map(|chi| ChannelState::new(chi, rhythm_kits_available)).collect();
        Self { channels, presets, default_patch }
    }

    /// このチャンネル・ノートの現在のProgram Change選択が指すベースパッチ。
    /// - 旋律: `presets`に該当(bank,program)が無ければ`default_patch`にフォールバック。
    /// - リズム: `presets`のkit内に該当ノートが無ければkit0へフォールバックし、それも無ければ
    ///   `None`（このノートは発音しない。GM2実機でキット内に未定義のノートが無音になるのと同じ、
    ///   `op505/tools/smf2op505`の`base_patch_for`と同じ意味論）。
    fn base_patch_for(&self, chi: usize, note: u8) -> Option<Op505Patch> {
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
    let _midi_connection = connect_midi_input(Arc::clone(&midi_queue));

    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    let supported = device.default_output_config().expect("no default output config");
    let num_channels = supported.channels() as usize;
    let sample_rate = supported.sample_rate().0 as f32;
    let stream_config: cpal::StreamConfig = supported.into();

    let mut engine = Op505Engine::new(sample_rate);
    let mut effects = MasterEffects::new(sample_rate);
    let (presets, default_patch) = load_presets();
    engine.set_patch(default_patch);
    let mut state = MidiState::new(presets, default_patch);

    let stream = device
        .build_output_stream::<f32, _, _>(
            &stream_config,
            move |output: &mut [f32], _| {
                drain_midi_queue(&midi_queue, &mut engine, &mut effects, &mut state);
                output.fill(0.0);
                engine.render(output, num_channels);
                effects.process(output, num_channels);
            },
            |err| eprintln!("audio error: {err}"),
            None,
        )
        .expect("failed to build output stream");

    stream.play().expect("failed to start audio stream");

    println!("op505-standalone: 再生中。Ctrl+Cで終了します。");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// `op505_presets_dir()`から全プリセットを読み込み、その先頭を起動時の既定パッチとして返す。
/// `Op505Patch::default()`は全段level=0/time=0のEGで無音になるため、プリセットが1つも
/// 見つからない場合は警告を出す。
fn load_presets() -> (Op505PresetBank, Op505Patch) {
    let dir = op505_presets_dir();
    let presets = Op505PresetBank::load_from_dir(&dir);
    match presets.sorted_entries().into_iter().next() {
        Some(((bank, program), preset)) => {
            println!("既定パッチ: {} (bank={bank}, program={program})", preset.name);
            (presets.clone(), preset.patch)
        }
        None => {
            eprintln!("警告: {} にプリセットが見つかりません。無音のデフォルトパッチのまま起動します。", dir.display());
            (presets, Op505Patch::default())
        }
    }
}

/// キューに溜まったMIDIメッセージを全て取り出し、エンジンへ適用する。
fn drain_midi_queue(queue: &MidiQueue, engine: &mut Op505Engine, effects: &mut MasterEffects, state: &mut MidiState) {
    let mut pending = queue.lock().unwrap();
    while let Some(bytes) = pending.pop_front() {
        handle_midi_message(engine, effects, state, &bytes);
    }
}

/// 1つのMIDIメッセージを解釈する。メッセージ長はステータスバイトごとに異なる
/// （Note On/Off・Pitch Bend・CCは3バイト、Program Change・Channel Pressureは2バイト）ため、
/// 固定長スライスパターンではなくステータス別に必要な長さを都度チェックする。
fn handle_midi_message(engine: &mut Op505Engine, effects: &mut MasterEffects, state: &mut MidiState, bytes: &[u8]) {
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
            println!("cc: ch{chi} cc={cc} val={val}");
            handle_control_change(engine, effects, state, chi, cc, val);
        }
        0xC0 => {
            let &[_, program] = bytes else { return };
            println!("program change: ch{chi} program={program}");
            state.channels[chi].program_state.program_change(program & 0x7f);
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

/// 1ボイスのノートオン。現在のProgram Changeが指す音色へ、NRPN上書き・CC2/CC4/AT/
/// Pitch FG演奏補正/Soft Pedalを重ねた実効パッチで発音する（`op505-vst`/`smf2op505`の
/// note_on適用と同型）。`base_patch_for`がNoneなら発音しない（リズムキット内に未定義のノート）。
fn note_on_voice(engine: &mut Op505Engine, state: &mut MidiState, chi: usize, note: u8, vel: u8) {
    state.channels[chi].pedal.note_on(note);
    let Some(base) = state.base_patch_for(chi, note) else { return };
    let st = &state.channels[chi];
    let mut eff = st.build_effective_patch(&base);
    st.apply_note_post_processing(&mut eff, note);
    let id = voice_id(chi, note);
    println!("note on:  ch{chi} note={note} vel={vel}");
    engine.set_patch(eff);
    engine.note_on(id, note_to_frequency(note), vel);
    engine.set_channel_volume(id, channel_gain(st.cc7, st.cc11));
    engine.set_pitch_bend(id, st.bend_cents);
    engine.set_pitch_fg_rate_scale(id, cc76_to_rate_scale(st.pitch_fg_cc76));
    for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
        if let Some(f_number) = f {
            engine.set_operator_f_number(id, op_index, *f_number);
        }
    }
}

/// ペダル保持中でなければ即座にNote Offする（保持中なら`pending_release`へ回るのみ）。
fn note_off_voice(engine: &mut Op505Engine, state: &mut MidiState, chi: usize, note: u8) {
    println!("note off: ch{chi} note={note}");
    state.channels[chi].poly_pressure[note as usize] = 0;
    if state.channels[chi].pedal.note_off(note) {
        engine.note_off(voice_id(chi, note));
    }
}

/// CC/NRPNで変わった実効パッチ・CC76 rate_scale・AT・OP F-Numberを、そのチャンネルの
/// 発音中ボイス全てへ伝播する（`op505/tools/smf2op505`のapply_liveと同じ役割）。
///
/// リズムチャンネルはノートごとに音色が違うため、`base_patch_for`をノートごとに呼ぶ
/// （旋律のようにチャンネル全体で1回だけ計算する最適化はできない）。キット内に未定義の
/// ノート（`base_patch_for`がNone）はスキップし、既存の発音中パラメーターをそのまま維持する。
fn apply_live(engine: &mut Op505Engine, state: &mut MidiState, chi: usize) {
    let rate_scale = cc76_to_rate_scale(state.channels[chi].pitch_fg_cc76);
    for note in 0..MIDI_NOTE_COUNT {
        let note_u8 = note as u8;
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
        for (op_index, f) in st.operator_f_number_override.iter().enumerate() {
            if let Some(f_number) = f {
                engine.set_operator_f_number(id, op_index, *f_number);
            }
        }
    }
}

/// 1つのコントロールチェンジを処理する。`op505/tools/smf2op505`のhandle_control_changeを
/// 移植したもの（ドラムキット/複数バンクの`bank`/`drums`引数は無く、`MidiState`が代わりに
/// Program Change選択済みの単一プリセット集合を持つ）。
fn handle_control_change(
    engine: &mut Op505Engine,
    effects: &mut MasterEffects,
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
        // CC2(ブレス)/CC4(フット): Expression Destination（NRPN(0,34)/(0,35)）へ加算 → 発音中ボイスへ伝播。
        2 => {
            state.channels[chi].cc2 = cc_to_u8(val);
            apply_live(engine, state, chi);
        }
        4 => {
            state.channels[chi].cc4 = cc_to_u8(val);
            apply_live(engine, state, chi);
        }
        // NRPN/RPN 選択（CC99/98=NRPN MSB/LSB、CC101/100=RPN MSB/LSB）
        98 => state.channels[chi].rpn.set_nrpn_lsb(cc_to_u7(val)),
        99 => state.channels[chi].rpn.set_nrpn_msb(cc_to_u7(val)),
        100 => state.channels[chi].rpn.set_rpn_lsb(cc_to_u7(val)),
        101 => state.channels[chi].rpn.set_rpn_msb(cc_to_u7(val)),
        // CC6 Data Entry MSB: 選択中の RPN/NRPN へ値を適用。エフェクト系NRPN(Reverb/Chorus)は
        // op505-midiがsound-core型を扱えないため`DataEntryOutcome::Effect`で返り、ここで
        // MasterEffectsへ適用する。
        6 => match state.channels[chi].apply_data_entry(val) {
            DataEntryOutcome::StateChanged { voice_update } => {
                if voice_update {
                    apply_live(engine, state, chi);
                }
            }
            DataEntryOutcome::Effect(target, value) => match target {
                EffectControlTarget::ReverbType => effects.set_reverb_type(ReverbType::from_u8(value)),
                EffectControlTarget::ChorusType => effects.set_chorus_type(ChorusType::from_u8(value)),
                EffectControlTarget::ReverbTime => effects.set_reverb_time(value),
                EffectControlTarget::ChorusModRate => effects.set_chorus_mod_rate(value),
                EffectControlTarget::ChorusModDepth => effects.set_chorus_mod_depth(value),
                EffectControlTarget::ChorusFeedback => effects.set_chorus_feedback(value),
                EffectControlTarget::ChorusSendToReverb => effects.set_chorus_send_to_reverb(value),
            },
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
        // CC91/93: マスターエフェクト送りレベル（master）。
        91 => effects.set_reverb_send(cc_to_u8(val)),
        93 => effects.set_chorus_send(cc_to_u8(val)),
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
        }
        // CC123 All Notes Off: 通常のNote-Off相当（リリースして自然減衰）。
        123 => {
            for note in 0..MIDI_NOTE_COUNT {
                engine.note_off(voice_id(chi, note as u8));
            }
            state.channels[chi].pedal.cc123_reset();
        }
        // Bank Select（CC0=MSB, CC32=LSB）：これだけでは音色は切り替わらない
        // （次のProgram Changeで確定する）。GM2リズムキット未読み込みのため、MSB=120を
        // 送ってもis_rhythmには実際には入らない（`ChannelProgramState`参照）。
        0 => state.channels[chi].program_state.bank_select_msb(cc_to_u7(val)),
        32 => state.channels[chi].program_state.bank_select_lsb(cc_to_u7(val)),
        _ => {}
    }
}

/// MIDI入力ポートへ接続する（Step 2: 複数ポートからの選択に対応）。
/// 選択順は「起動時コマンドライン引数（数値）」→「ポートが1つのみなら自動選択」→
/// 「複数かつ引数無しなら標準入力で対話選択」。
fn connect_midi_input(queue: MidiQueue) -> MidiInputConnection<()> {
    let input = MidiInput::new("op505-standalone").expect("failed to init MIDI input");
    let ports = input.ports();
    if ports.is_empty() {
        panic!("MIDI入力ポートが見つかりません。loopMIDI等で仮想ポートを作成してから起動してください。");
    }

    println!("利用可能なMIDI入力ポート:");
    let port_names: Vec<String> = ports
        .iter()
        .map(|port| input.port_name(port).unwrap_or_else(|_| "(不明)".to_string()))
        .collect();
    for (i, name) in port_names.iter().enumerate() {
        println!("  [{i}] {name}");
    }

    let index = select_port_index(&port_names);
    let port = &ports[index];
    println!("接続: {}", port_names[index]);

    input
        .connect(
            port,
            "op505-standalone-input",
            move |_stamp, message, _| {
                queue.lock().unwrap().push_back(message.to_vec());
            },
            (),
        )
        .expect("failed to connect to MIDI input port")
}

/// 接続するポートのインデックスを決める。
/// 1. 起動時コマンドライン引数（`op505-standalone.exe 1`のように数値で指定）があれば最優先。
/// 2. ポートが1つしかなければ選ぶ余地が無いので自動選択。
/// 3. それ以外は標準入力で対話選択（不正な入力は再入力を促す）。
fn select_port_index(port_names: &[String]) -> usize {
    if let Some(arg) = std::env::args().nth(1) {
        match arg.parse::<usize>() {
            Ok(index) if index < port_names.len() => return index,
            _ => eprintln!(
                "警告: 引数 '{arg}' は有効なポート番号(0〜{})ではありません。無視します。",
                port_names.len() - 1
            ),
        }
    }

    if port_names.len() == 1 {
        return 0;
    }

    use std::io::Write;
    loop {
        print!("接続するポート番号を入力してください (0〜{}): ", port_names.len() - 1);
        std::io::stdout().flush().ok();

        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            eprintln!("標準入力の読み取りに失敗しました。ポート0を使用します。");
            return 0;
        }

        match line.trim().parse::<usize>() {
            Ok(index) if index < port_names.len() => return index,
            _ => println!("0〜{}の番号を入力してください。", port_names.len() - 1),
        }
    }
}
