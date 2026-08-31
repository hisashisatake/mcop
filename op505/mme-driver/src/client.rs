//! 名前付きパイプ`\\.\pipe\op505.mme.v1`経由でop505-standaloneへMIDIバイト列を転送する
//! クライアント側実装。std::fs::Fileでパイプへ`OpenOptions::write(true)`するだけで接続できる
//! （名前付きパイプのクライアント接続はCreateFileWと等価で、Rust標準ライブラリの範囲で完結する。
//! サーバー側のCreateNamedPipeWのようなWin32固有APIはクライアントには不要）。
//!
//! フレーム形式（op505/standalone/src/mme_pipe.rsのdecode_frameと対になる。変更時は両方揃える）:
//! `[u8 version=1][u8 kind][u8 device_id][u16 len（リトルエンディアン）][bytes; len]`
//! kind: 0=Short（デコード済み1〜3バイトのMIDIメッセージ）, 1=Long（SysEx。フェーズ4で配線、
//! op505側では内容を解釈せず破棄する。ペイロード長がu16の範囲を超える場合はこの層まで
//! 到達しない——lib.rsのMODM_LONGDATA側で同期エラーとして弾く）, 2=Reset（payload無し）。
//!
//! MODM_DATA呼び出しスレッド（winmm経由でDominoのMIDIスレッド）をブロックしないよう、
//! 実際の書き込みは専用ライタースレッドが担う。フレームは有界チャネル（256件）経由で
//! 渡し、溢れたら黙って破棄する（R4対策：op505-standalone側が詰まってもホストは止まらない）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Once, OnceLock};
use std::time::Duration;

use crate::log;

const PIPE_PATH: &str = r"\\.\pipe\op505.mme.v1";
const CHANNEL_CAPACITY: usize = 256;
const RECONNECT_INTERVAL: Duration = Duration::from_millis(200);

pub const FRAME_VERSION: u8 = 1;
pub const FRAME_KIND_SHORT: u8 = 0;
pub const FRAME_KIND_LONG: u8 = 1;
pub const FRAME_KIND_RESET: u8 = 2;

static SENDER: OnceLock<SyncSender<Vec<u8>>> = OnceLock::new();
static WRITER_THREAD_INIT: Once = Once::new();

fn try_connect() -> Option<File> {
    OpenOptions::new().write(true).open(PIPE_PATH).ok()
}

/// パイプへの最初の接続を同期的に試みる。成功・失敗にかかわらずライタースレッドを
/// （未起動なら）起動する。戻り値はMODM_OPENの成否判定に使う
/// （サーバー未起動なら`false`を返し、呼び出し側がMMSYSERR_NOTENABLEDを返せるようにする）。
pub fn open() -> bool {
    let file = try_connect();
    let connected = file.is_some();
    ensure_writer_thread(file);
    connected
}

/// ライタースレッドをプロセス内で一度だけ起動する。2回目以降のopen()呼び出しで得た
/// Fileは（既にスレッドが起動済みなら）使われず破棄されるが、ライタースレッド自身が
/// 未接続時に自力で再接続を試みるため実害はない。
fn ensure_writer_thread(initial: Option<File>) {
    WRITER_THREAD_INIT.call_once(|| {
        let (tx, rx) = sync_channel::<Vec<u8>>(CHANNEL_CAPACITY);
        if SENDER.set(tx).is_err() {
            log::log("client: SENDER already set (unexpected)");
        }
        std::thread::spawn(move || writer_loop(rx, initial));
    });
}

fn writer_loop(rx: Receiver<Vec<u8>>, initial: Option<File>) {
    let mut file = initial;
    loop {
        match rx.recv_timeout(RECONNECT_INTERVAL) {
            Ok(frame) => {
                if file.is_none() {
                    file = try_connect();
                }
                if let Some(f) = file.as_mut() {
                    if f.write_all(&frame).is_err() {
                        log::log("client: pipe write failed, will reconnect");
                        file = None;
                    }
                }
                // fileが無い（未接続）場合、このフレームは破棄する。
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

pub fn build_frame(kind: u8, device_id: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(FRAME_VERSION);
    frame.push(kind);
    frame.push(device_id);
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// フレームを送信キューへ積む。キューが溢れていれば黙って破棄する
/// （modMessage呼び出しスレッドをブロックしないため）。
pub fn send_frame(frame: Vec<u8>) {
    let Some(tx) = SENDER.get() else {
        // open()がまだ一度も呼ばれていない（通常はMODM_OPEN前にMODM_DATAは来ない）。
        return;
    };
    if let Err(TrySendError::Full(_)) = tx.try_send(frame) {
        log::log("client: send queue full, dropping frame");
    }
}
