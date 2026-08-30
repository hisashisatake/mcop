//! op505-mme-driver（Domino等のプロセスに住むMMEドライバDLL）からの名前付きパイプ受信サーバー。
//! （旧`mme_pipe`モジュールを`sources`配下へ移動したもの。フレーム形式・FFI設計は不変）
//!
//! フレーム形式（op505/mme-driver/src/client.rsのbuild_frameと対になる。変更時は両方揃える）:
//! `[u8 version=1][u8 kind][u8 device_id][u16 len（リトルエンディアン）][bytes; len]`
//! kind: 0=Short（デコード済み1〜3バイトのMIDIメッセージ）, 1=Long（SysEx、未対応）,
//! 2=Reset（payload無し。全16chへAll Sound Off(CC120)を合成して送る）。
//!
//! 名前付きパイプの「サーバー」役はRust標準ライブラリに無いAPI（CreateNamedPipeW /
//! ConnectNamedPipe）が必要なため、その2関数だけをkernel32.dllから直接FFI宣言する
//! （windows-sys等の外部crateは導入しない）。接続確立後の読み取り・クローズは
//! `std::fs::File`（`FromRawHandle`経由）に委ね、生のReadFile/CloseHandle呼び出しは書かない。
//!
//! acceptループの明示的な停止手段は現状無く、プロセス終了まで動き続ける
//! （タスクトレイ化フェーズで「終了」メニューの実装時に追加する）。

use std::fs::File;
use std::io::Read;
use std::os::windows::io::FromRawHandle;

use crate::midi_source::{MidiSink, MidiSource};

const PIPE_PATH: &str = r"\\.\pipe\op505.mme.v1";

const FRAME_VERSION: u8 = 1;
const FRAME_KIND_SHORT: u8 = 0;
const FRAME_KIND_RESET: u8 = 2;

#[allow(non_snake_case, non_camel_case_types, dead_code)]
mod ffi {
    use std::ffi::c_void;

    pub type HANDLE = *mut c_void;
    pub type BOOL = i32;
    pub type DWORD = u32;

    pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

    pub const PIPE_ACCESS_INBOUND: DWORD = 0x0000_0001;
    pub const PIPE_TYPE_MESSAGE: DWORD = 0x0000_0004;
    pub const PIPE_READMODE_MESSAGE: DWORD = 0x0000_0002;
    pub const PIPE_WAIT: DWORD = 0x0000_0000;
    pub const PIPE_UNLIMITED_INSTANCES: DWORD = 255;

    extern "system" {
        pub fn CreateNamedPipeW(
            lpName: *const u16,
            dwOpenMode: DWORD,
            dwPipeMode: DWORD,
            nMaxInstances: DWORD,
            nOutBufferSize: DWORD,
            nInBufferSize: DWORD,
            nDefaultTimeOut: DWORD,
            lpSecurityAttributes: *mut c_void,
        ) -> HANDLE;

        pub fn ConnectNamedPipe(hNamedPipe: HANDLE, lpOverlapped: *mut c_void) -> BOOL;
    }
}

/// パイプサーバーが生きていることを示すだけのハンドル（実体はacceptループのスレッド）。
pub struct PipeSource;

impl MidiSource for PipeSource {
    fn describe(&self) -> String {
        format!("MMEドライバ (名前付きパイプ {PIPE_PATH})")
    }
}

/// パイプ受信サーバーをバックグラウンドスレッドで起動する（呼び出しはブロックしない）。
/// 失敗時は標準エラーへ警告を出して諦める（MMEドライバ経由の入力が無効になるだけで、
/// midir経由の既存入力は影響を受けない）。
pub fn spawn(sink: MidiSink) -> PipeSource {
    std::thread::spawn(move || accept_loop(sink));
    PipeSource
}

fn accept_loop(sink: MidiSink) {
    loop {
        let Some(handle) = create_pipe_instance() else {
            eprintln!(
                "op505-standalone: MMEパイプサーバーの作成に失敗しました（{}）。MMEドライバ経由の入力は無効です。",
                std::io::Error::last_os_error()
            );
            return;
        };

        // ConnectNamedPipeはクライアント接続まで待機する。CreateNamedPipeWとの間の競合で
        // クライアントが先に接続済みだった場合はFALSE+ERROR_PIPE_CONNECTEDが返るが、これも成功扱い。
        let connected = unsafe { ffi::ConnectNamedPipe(handle, std::ptr::null_mut()) };
        const ERROR_PIPE_CONNECTED: i32 = 535;
        let ok = connected != 0 || std::io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED);

        // SAFETY: handleはCreateNamedPipeWが返した有効なHANDLEで、以降このFileが唯一の所有者になる。
        let file = unsafe { File::from_raw_handle(handle) };
        if !ok {
            drop(file);
            continue;
        }

        let sink_for_client = sink.clone();
        std::thread::spawn(move || serve_client(file, sink_for_client));
        // ループ先頭へ戻り、次のクライアント用に新しいインスタンスを作る
        // （複数のWinMMホストアプリが同時に接続してくる可能性があるため）。
    }
}

fn create_pipe_instance() -> Option<ffi::HANDLE> {
    let name = pipe_name_wide();
    let handle = unsafe {
        ffi::CreateNamedPipeW(
            name.as_ptr(),
            ffi::PIPE_ACCESS_INBOUND,
            ffi::PIPE_TYPE_MESSAGE | ffi::PIPE_READMODE_MESSAGE | ffi::PIPE_WAIT,
            ffi::PIPE_UNLIMITED_INSTANCES,
            0,    // out buffer size（受信専用なので0で可）
            4096, // in buffer size（フレームは高々8バイト程度なので十分すぎるほど余裕がある）
            0,    // default timeout（0はシステム既定値=50ms）
            std::ptr::null_mut(),
        )
    };
    if handle == ffi::INVALID_HANDLE_VALUE {
        None
    } else {
        Some(handle)
    }
}

fn pipe_name_wide() -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(PIPE_PATH).encode_wide().chain(std::iter::once(0)).collect()
}

fn serve_client(mut file: File, sink: MidiSink) {
    let mut buf = [0u8; 4096];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break, // クライアントが切断
            Ok(n) => {
                for message in decode_frame(&buf[..n]) {
                    sink.push(message);
                }
            }
            Err(_) => break,
        }
    }
    // fileのDropでCloseHandleされる（DisconnectNamedPipeは呼ばない。このインスタンスは
    // 使い捨てで、次のクライアントには別インスタンスをaccept_loopが新規作成するため）。
}

/// 1回の`ReadFile`（メッセージモードのため常に1メッセージ境界と一致する）から
/// 実際にキューへ積むべきMIDIバイト列（0〜複数個）を取り出す。
fn decode_frame(raw: &[u8]) -> Vec<Vec<u8>> {
    if raw.len() < 5 {
        return Vec::new();
    }
    let version = raw[0];
    let kind = raw[1];
    let _device_id = raw[2];
    let len = u16::from_le_bytes([raw[3], raw[4]]) as usize;
    if version != FRAME_VERSION || raw.len() < 5 + len {
        return Vec::new();
    }
    let payload = &raw[5..5 + len];

    match kind {
        FRAME_KIND_SHORT => vec![payload.to_vec()],
        // All Sound Off(CC120)を全16chへ合成する。既存のhandle_control_changeが
        // 対応済みのCCなので、専用の処理を新設せずに済む。
        FRAME_KIND_RESET => (0u8..16).map(|ch| vec![0xB0 | ch, 120, 0]).collect(),
        _ => Vec::new(), // Long(SysEx)等、現状未対応
    }
}
