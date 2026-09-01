//! 名前付きパイプ`\\.\pipe\op505.mme.v1`経由でop505-standaloneへ標準MIDIバイト列を送信する。
//! フレーム形式・ライタースレッド構成は`op505/mme-driver/src/client.rs`をそのまま踏襲する
//! （standaloneの`sources/pipe_src.rs`は接続元を区別しないため無改造で受けられる）。
//!
//! gesture-appはジェスチャーをMIDIへ変換してstandaloneへ送るだけのコントローラーであり、
//! エンジン・音声出力は一切持たない（詳細はCLAUDE.md gesture-app節、
//! memory `project_gesture_app_controller_roadmap.md`参照）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Once, OnceLock};
use std::time::Duration;

const PIPE_PATH: &str = r"\\.\pipe\op505.mme.v1";
const CHANNEL_CAPACITY: usize = 256;
const RECONNECT_INTERVAL: Duration = Duration::from_millis(200);

const FRAME_VERSION: u8 = 1;
const FRAME_KIND_SHORT: u8 = 0;
const DEVICE_ID: u8 = 0; // サーバー側は無視する（pipe_src.rsの_device_id）ため固定値で十分。

static SENDER: OnceLock<SyncSender<Vec<u8>>> = OnceLock::new();
static WRITER_THREAD_INIT: Once = Once::new();

fn try_connect() -> Option<File> {
    OpenOptions::new().write(true).open(PIPE_PATH).ok()
}

/// 初回送信前に一度だけ呼ぶ。パイプへ未接続なら、gesture-appと同じワークスペースの
/// `target/<profile>/`に並んでいるはずの`op505-standalone.exe`を起動してみる
/// （単一インスタンスMutexがあるため既に起動済みでも二重起動にはならない）。
/// 見つからない/起動に失敗しても致命的ではない（ライタースレッドが200ms間隔で
/// 再接続を試み続けるだけ）ため、エラーはログのみに留める。
fn ensure_started() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if try_connect().is_some() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let Some(dir) = exe.parent() else { return };
        let standalone = dir.join("op505-standalone.exe");
        if !standalone.exists() {
            eprintln!("midi_out: op505-standalone.exe が見つかりません（{}）。手動で起動してください。", standalone.display());
            return;
        }
        if let Err(e) = std::process::Command::new(&standalone).spawn() {
            eprintln!("midi_out: op505-standalone.exeの起動に失敗しました: {e}");
        }
    });
}

fn ensure_writer_thread() {
    WRITER_THREAD_INIT.call_once(|| {
        let (tx, rx) = sync_channel::<Vec<u8>>(CHANNEL_CAPACITY);
        if SENDER.set(tx).is_err() {
            eprintln!("midi_out: SENDER already set (unexpected)");
        }
        std::thread::spawn(move || writer_loop(rx));
    });
}

fn writer_loop(rx: Receiver<Vec<u8>>) {
    let mut file = try_connect();
    loop {
        match rx.recv_timeout(RECONNECT_INTERVAL) {
            Ok(frame) => {
                if file.is_none() {
                    file = try_connect();
                }
                if let Some(f) = file.as_mut() {
                    if f.write_all(&frame).is_err() {
                        file = None;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if file.is_none() {
                    file = try_connect();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn build_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(FRAME_VERSION);
    frame.push(FRAME_KIND_SHORT);
    frame.push(DEVICE_ID);
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// 1〜3バイトのデコード済みMIDIメッセージを送信キューへ積む。キューが溢れていれば
/// 黙って破棄する（フロントエンドのイベントループをブロックしないため）。
fn send_short(bytes: &[u8]) {
    ensure_started();
    ensure_writer_thread();
    let Some(tx) = SENDER.get() else { return };
    if let Err(TrySendError::Full(_)) = tx.try_send(build_frame(bytes)) {
        eprintln!("midi_out: send queue full, dropping frame");
    }
}

pub fn note_on(channel: u8, note: u8, velocity: u8) {
    send_short(&[0x90 | (channel & 0x0F), note & 0x7F, velocity.clamp(1, 127)]);
}

pub fn note_off(channel: u8, note: u8) {
    send_short(&[0x80 | (channel & 0x0F), note & 0x7F, 0]);
}

pub fn control_change(channel: u8, cc: u8, value: u8) {
    send_short(&[0xB0 | (channel & 0x0F), cc & 0x7F, value & 0x7F]);
}

pub fn program_change(channel: u8, program: u8) {
    send_short(&[0xC0 | (channel & 0x0F), program & 0x7F]);
}

/// CC0(Bank Select MSB) + CC32(Bank Select LSB)。`bank`は14bit(0〜16383)。
pub fn bank_select(channel: u8, bank: u16) {
    let bank = bank.min(16383);
    control_change(channel, 0, (bank >> 7) as u8);
    control_change(channel, 32, (bank & 0x7F) as u8);
}

/// RPN(param_msb, param_lsb)を選択してData Entry MSB(CC6)へ`value`を書き込む
/// （CC101/100/6の3メッセージ）。LSBまで必要な14bit値はCC38(Data Entry LSB)を別途呼ぶこと。
pub fn rpn_data_entry(channel: u8, param_msb: u8, param_lsb: u8, value: u8) {
    control_change(channel, 101, param_msb & 0x7F);
    control_change(channel, 100, param_lsb & 0x7F);
    control_change(channel, 6, value & 0x7F);
}

/// NRPN(param_msb, param_lsb)を選択してData Entry MSB(CC6)へ`value`を書き込む
/// （CC99/98/6の3メッセージ）。
pub fn nrpn_data_entry(channel: u8, param_msb: u8, param_lsb: u8, value: u8) {
    control_change(channel, 99, param_msb & 0x7F);
    control_change(channel, 98, param_lsb & 0x7F);
    control_change(channel, 6, value & 0x7F);
}
