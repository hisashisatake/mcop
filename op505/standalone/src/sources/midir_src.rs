//! midir経由のMIDI入力（実機MIDIキーボード・loopMIDI等の既存仮想ポート）。
//! 旧`connect_midi_input`/`select_port_index`（コマンドライン引数+stdin対話）を置き換える。
//! ポート選択はコマンドライン引数・stdin対話を廃止し、[`crate::config`]に保存された
//! ポート名、または「ポートがちょうど1個だけ」の場合の自動選択のみを行う
//! （タスクトレイ化後はstdinが使えなくなるため）。

use midir::{MidiInput, MidiInputConnection};

use crate::midi_source::{MidiSink, MidiSource};

pub struct MidirSource {
    port_name: String,
    _connection: MidiInputConnection<()>,
}

impl MidiSource for MidirSource {
    fn describe(&self) -> String {
        format!("MIDIキーボード/仮想ポート: {}", self.port_name)
    }
}

impl MidirSource {
    /// 接続中のポート名（トレイメニューの初期チェック状態を合わせるために使う）。
    pub fn connected_port_name(&self) -> &str {
        &self.port_name
    }
}

/// 利用可能な入力ポート名の一覧を返す（列挙のみ、接続はしない）。
pub fn list_input_ports() -> Vec<String> {
    let Ok(input) = MidiInput::new("op505-standalone-list") else {
        return Vec::new();
    };
    input
        .ports()
        .iter()
        .map(|port| input.port_name(port).unwrap_or_else(|_| "(不明)".to_string()))
        .collect()
}

/// `preferred_name`と完全一致するポートへ接続する。指定が無い、または見つからない場合は
/// 「ポートがちょうど1個だけ」ならそれへ自動接続する。
///
/// ポートが1個も無い場合、または複数あって自動選択できない場合は`None`を返す
/// （[`crate::sources::pipe_src`]経由の入力だけで使う構成、複数ポートで設定未定の構成は
/// どちらも正当にあり得るため、警告に留めpanicしない）。
pub fn connect(sink: MidiSink, preferred_name: Option<&str>) -> Option<MidirSource> {
    let input = MidiInput::new("op505-standalone").ok()?;
    let ports = input.ports();
    if ports.is_empty() {
        crate::log::log("MIDI入力ポートが見つかりません（loopMIDI等が未起動）。");
        return None;
    }

    let named: Vec<(midir::MidiInputPort, String)> = ports
        .into_iter()
        .map(|port| {
            let name = input.port_name(&port).unwrap_or_else(|_| "(不明)".to_string());
            (port, name)
        })
        .collect();

    let chosen = preferred_name
        .and_then(|want| named.iter().find(|(_, name)| name == want))
        .or_else(|| if named.len() == 1 { named.first() } else { None });

    let Some((port, port_name)) = chosen else {
        let available: Vec<&String> = named.iter().map(|(_, name)| name).collect();
        crate::log::log(&format!(
            "MIDI入力ポートが複数見つかりましたが、設定（standalone.jsonのmidi_in_port）が\
             未指定のため自動接続しません。利用可能なポート: {available:?}"
        ));
        return None;
    };

    let port = port.clone();
    let port_name = port_name.clone();

    let connection = input
        .connect(
            &port,
            "op505-standalone-input",
            move |_stamp, message, _| sink.push(message.to_vec()),
            (),
        )
        .ok()?;

    Some(MidirSource { port_name, _connection: connection })
}
