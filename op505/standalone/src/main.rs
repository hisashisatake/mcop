//! op505をDomino等のMME専用MIDIシーケンサから直接鳴らすための常駐アプリ（Step 1）。
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

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use midir::{MidiInput, MidiInputConnection};
use op505_core::{op505_presets_dir, Op505Engine, Op505Patch, Op505PresetBank};
use sound_core::{AudioProcessor, MasterEffects, Vco};

type MidiQueue = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// MIDIチャンネル(0〜15)とノート番号からVco::note_on/note_offの`channel`引数(ボイスID)を作る。
/// `op505/tools/smf2op505`のrender.rsと同じ規約（`channel_index * 128 + note`）に揃えている。
fn voice_id(channel: u8, note: u8) -> usize {
    channel as usize * 128 + note as usize
}

/// MIDIノート番号(0〜127)を周波数(Hz)に変換する。A4=69=440Hzの平均律。
fn note_to_frequency(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
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
    engine.set_patch(default_patch());

    let stream = device
        .build_output_stream::<f32, _, _>(
            &stream_config,
            move |output: &mut [f32], _| {
                drain_midi_queue(&midi_queue, &mut engine);
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

/// 起動時の既定パッチ。`op505_presets_dir()`にある最初のプリセットを使う
/// （Step 1にはプログラムチェンジ対応が無く、常に単一パッチで鳴らす）。
/// `Op505Patch::default()`は全段level=0/time=0のEGで無音になるため、
/// プリセットが1つも見つからない場合は警告を出す。
fn default_patch() -> Op505Patch {
    let dir = op505_presets_dir();
    let presets = Op505PresetBank::load_from_dir(&dir);
    match presets.sorted_entries().into_iter().next() {
        Some(((bank, program), preset)) => {
            println!("既定パッチ: {} (bank={bank}, program={program})", preset.name);
            preset.patch
        }
        None => {
            eprintln!("警告: {} にプリセットが見つかりません。無音のデフォルトパッチのまま起動します。", dir.display());
            Op505Patch::default()
        }
    }
}

/// キューに溜まったMIDIメッセージを全て取り出し、エンジンへ適用する。
fn drain_midi_queue(queue: &MidiQueue, engine: &mut Op505Engine) {
    let mut pending = queue.lock().unwrap();
    while let Some(bytes) = pending.pop_front() {
        handle_midi_message(engine, &bytes);
    }
}

/// Note On/Note Off（Note On velocity=0によるNote Off含む）のみを解釈する。
/// CC/NRPN/プログラムチェンジ等はStep 3で`op505-midi`と繋ぎ込む。
fn handle_midi_message(engine: &mut Op505Engine, bytes: &[u8]) {
    let [status, data1, data2, ..] = *bytes else { return };
    let channel = status & 0x0F;

    match status & 0xF0 {
        0x90 if data2 == 0 => {
            println!("note off: ch{channel} note={data1}");
            engine.note_off(voice_id(channel, data1));
        }
        0x90 => {
            println!("note on:  ch{channel} note={data1} vel={data2}");
            engine.note_on(voice_id(channel, data1), note_to_frequency(data1), data2);
        }
        0x80 => {
            println!("note off: ch{channel} note={data1}");
            engine.note_off(voice_id(channel, data1));
        }
        _ => {}
    }
}

/// 最初に見つかったMIDI入力ポートへ接続する。複数ポートからの選択はStep 2で対応する。
fn connect_midi_input(queue: MidiQueue) -> MidiInputConnection<()> {
    let input = MidiInput::new("op505-standalone").expect("failed to init MIDI input");
    let ports = input.ports();
    if ports.is_empty() {
        panic!("MIDI入力ポートが見つかりません。loopMIDI等で仮想ポートを作成してから起動してください。");
    }

    println!("利用可能なMIDI入力ポート:");
    for (i, port) in ports.iter().enumerate() {
        let name = input.port_name(port).unwrap_or_else(|_| "(不明)".to_string());
        println!("  [{i}] {name}");
    }

    let port = &ports[0];
    println!("接続: {}", input.port_name(port).unwrap_or_default());

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
