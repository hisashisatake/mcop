//! MIDIバイト列の供給元を差し替え可能にする薄い抽象化。
//! 消費側（[`crate::drain_midi_queue`]以降）は一切変更しない——境界は既存の
//! `MidiQueue`のままで、生産側だけを複数併存できるようにする
//! （[`crate::sources`]配下が個々の実装。実機MIDIキーボード/loopMIDI経由の
//! [`crate::sources::midir_src`]、`op505-mme-driver`からの名前付きパイプ経由の
//! [`crate::sources::pipe_src`]、将来のWindows MIDI Services経由の供給元も
//! 同じトレイトを実装するだけで足りる設計）。

use crate::MidiQueue;

/// 供給元がMidiQueueへMIDIバイト列を書き込むための薄いハンドル。
/// `MidiQueue`の生の型（Arc<Mutex<VecDeque<Vec<u8>>>>）を各供給元実装から隠す。
#[derive(Clone)]
pub struct MidiSink {
    queue: MidiQueue,
}

impl MidiSink {
    pub fn new(queue: MidiQueue) -> Self {
        Self { queue }
    }

    pub fn push(&self, bytes: Vec<u8>) {
        self.queue.lock().unwrap().push_back(bytes);
    }
}

/// 生きている間、供給元を有効に保つハンドル。Dropで供給元を停止する
/// （[`crate::sources::midir_src::MidirSource`]は`MidiInputConnection`を、
/// [`crate::sources::pipe_src::PipeSource`]は現状Dropでの明示停止を持たず
/// プロセス終了まで動き続ける——停止手段の追加はタスクトレイ化フェーズで行う）。
pub trait MidiSource: Send {
    fn describe(&self) -> String;
}

/// 有効な供給元をまとめて生かしておくだけの入れ物（mainのローカル変数として保持する）。
#[derive(Default)]
pub struct SourceRegistry {
    sources: Vec<Box<dyn MidiSource>>,
}

impl SourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, source: Box<dyn MidiSource>) {
        crate::log::log(&format!("MIDI入力: {}", source.describe()));
        self.sources.push(source);
    }
}
