//! gesture-appの音色名表示向けクエリ専用パイプ`\\.\pipe\op505.query.v1`のサーバー。
//!
//! `sources/pipe_src.rs`（MIDIバイト列の一方向転送、Domino等がクライアント）とは別チャネル。
//! あちらは受信専用（`PIPE_ACCESS_INBOUND`）で返事を書けないため、双方向化はせず素直に
//! 別パイプを立てる（既存のMIDI転送路には一切触れない）。
//!
//! このサーバーは監視スレッド・差分検知・ブロードキャストを持たない。クライアント
//! （gesture-app）が1リクエストを送るたびに、その場で1レスポンスを返すだけ
//! （更新方式は「gesture-appがProgram Change送信直後・ウィンドウフォーカス復帰時に
//! 問い合わせる」というクライアント側の設計に委ねている。詳細はCLAUDE.md gesture-app節、
//! memory `project_gesture_app_program_name_standalone_query.md`参照）。
//!
//! フレーム形式（`gesture-app/src-tauri/src/query_client.rs`と対になる。変更時は両方揃える）:
//! `[u8 version=1][u8 kind][u8 reserved][u16 len（リトルエンディアン）][bytes; len]`
//! - リクエスト kind=0（QueryProgram）: payload = `[channel]`（0〜15）
//! - レスポンス kind=0x80（ProgramInfo）: payload =
//!   `[status][bank_lo][bank_hi][program][name_len][name UTF-8 bytes; name_len]`
//!   statusは`shared::ProgramStatus`と対応（0=Resolved, 1=NotFound, 2=Rhythm, 3=Editing）。
//!
//! FFI宣言・SDDL・acceptループの構成は`sources/pipe_src.rs`を踏襲する（コメントは重複を
//! 避けるため簡略化している。詳細な設計理由はそちらを参照）。

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::FromRawHandle;
use std::sync::{Arc, OnceLock};

use crate::shared::{ProgramInfo, ProgramStatus, SharedEditState};

const PIPE_PATH: &str = r"\\.\pipe\op505.query.v1";
const PIPE_SDDL: &str = "S:(ML;;NW;;;LW)";

const FRAME_VERSION: u8 = 1;
const REQUEST_KIND_QUERY_PROGRAM: u8 = 0;
const RESPONSE_KIND_PROGRAM_INFO: u8 = 0x80;

#[allow(non_snake_case, non_camel_case_types, dead_code)]
mod ffi {
    use std::ffi::c_void;

    pub type HANDLE = *mut c_void;
    pub type BOOL = i32;
    pub type DWORD = u32;
    pub type LPVOID = *mut c_void;

    pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

    pub const PIPE_ACCESS_DUPLEX: DWORD = 0x0000_0003;
    pub const PIPE_TYPE_MESSAGE: DWORD = 0x0000_0004;
    pub const PIPE_READMODE_MESSAGE: DWORD = 0x0000_0002;
    pub const PIPE_WAIT: DWORD = 0x0000_0000;
    pub const PIPE_UNLIMITED_INSTANCES: DWORD = 255;

    pub const SDDL_REVISION_1: DWORD = 1;

    #[repr(C)]
    pub struct SecurityAttributes {
        pub n_length: DWORD,
        pub lp_security_descriptor: LPVOID,
        pub b_inherit_handle: BOOL,
    }

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

    #[link(name = "advapi32")]
    extern "system" {
        pub fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            StringSecurityDescriptor: *const u16,
            StringSDRevision: DWORD,
            SecurityDescriptor: *mut LPVOID,
            SecurityDescriptorSize: *mut DWORD,
        ) -> BOOL;
    }
}

/// `PIPE_SDDL`から生成したSECURITY_ATTRIBUTESをプロセス内で使い回す（`pipe_src.rs`と同じ
/// パターン。2枚のパイプで別々のSDDL文字列を都度変換するのは無駄なため、意図的に複製する
/// ——1関数に共通化すると`pipe_src.rs`側の変更影響がこちらへ及ぶため、独立性を優先した）。
fn security_attributes() -> Option<&'static ffi::SecurityAttributes> {
    static SA_ADDR: OnceLock<Option<usize>> = OnceLock::new();
    SA_ADDR
        .get_or_init(build_security_attributes)
        .map(|addr| unsafe { &*(addr as *const ffi::SecurityAttributes) })
}

fn build_security_attributes() -> Option<usize> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    let sddl: Vec<u16> = OsStr::new(PIPE_SDDL).encode_wide().chain(std::iter::once(0)).collect();

    let mut descriptor: ffi::LPVOID = std::ptr::null_mut();
    let ok = unsafe {
        ffi::ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            ffi::SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || descriptor.is_null() {
        crate::log::log(&format!(
            "クエリパイプのSDDLセキュリティ記述子生成に失敗しました（{}）。既定のACLで動作します。",
            std::io::Error::last_os_error()
        ));
        return None;
    }

    let sa = Box::leak(Box::new(ffi::SecurityAttributes {
        n_length: std::mem::size_of::<ffi::SecurityAttributes>() as ffi::DWORD,
        lp_security_descriptor: descriptor,
        b_inherit_handle: 0,
    }));
    Some(sa as *const _ as usize)
}

/// クエリパイプサーバーをバックグラウンドスレッドで起動する（呼び出しはブロックしない）。
/// 失敗時は標準エラー相当のログのみに留める（音色名表示が使えなくなるだけで、演奏用の
/// MIDI転送路には影響しない）。
pub fn spawn(shared: Arc<SharedEditState>) {
    std::thread::spawn(move || accept_loop(shared));
}

fn accept_loop(shared: Arc<SharedEditState>) {
    loop {
        let Some(handle) = create_pipe_instance() else {
            crate::log::log(&format!(
                "クエリパイプサーバーの作成に失敗しました（{}）。音色名クエリは無効です。",
                std::io::Error::last_os_error()
            ));
            return;
        };

        let connected = unsafe { ffi::ConnectNamedPipe(handle, std::ptr::null_mut()) };
        const ERROR_PIPE_CONNECTED: i32 = 535;
        let ok = connected != 0 || std::io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED);

        // SAFETY: handleはCreateNamedPipeWが返した有効なHANDLEで、以降このFileが唯一の所有者になる。
        let file = unsafe { File::from_raw_handle(handle) };
        if !ok {
            drop(file);
            continue;
        }

        let shared_for_client = Arc::clone(&shared);
        std::thread::spawn(move || serve_client(file, shared_for_client));
    }
}

fn create_pipe_instance() -> Option<ffi::HANDLE> {
    let name = pipe_name_wide();
    let sa_ptr: *mut std::ffi::c_void = security_attributes()
        .map(|sa| sa as *const ffi::SecurityAttributes as *mut std::ffi::c_void)
        .unwrap_or(std::ptr::null_mut());
    let handle = unsafe {
        ffi::CreateNamedPipeW(
            name.as_ptr(),
            ffi::PIPE_ACCESS_DUPLEX,
            ffi::PIPE_TYPE_MESSAGE | ffi::PIPE_READMODE_MESSAGE | ffi::PIPE_WAIT,
            ffi::PIPE_UNLIMITED_INSTANCES,
            4096, // out buffer size（レスポンスは高々300バイト弱なので十分）
            4096, // in buffer size（リクエストは高々5バイト）
            0,    // default timeout（0はシステム既定値=50ms）
            sa_ptr,
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

/// 1クライアント（gesture-appの1回の接続）を、切断されるまでリクエスト/レスポンスの
/// 繰り返しで処理する。メッセージモードのため1回の`read`は常に1リクエスト境界と一致する。
fn serve_client(mut file: File, shared: Arc<SharedEditState>) {
    let mut buf = [0u8; 4096];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break, // クライアントが切断
            Ok(n) => n,
            Err(_) => break,
        };
        let Some(channel) = decode_query_request(&buf[..n]) else { continue };
        let info = shared.resolve_program_name(channel as usize);
        let response = encode_program_info_response(&info);
        if file.write_all(&response).is_err() {
            break;
        }
    }
}

fn decode_query_request(raw: &[u8]) -> Option<u8> {
    if raw.len() < 6 {
        return None;
    }
    let version = raw[0];
    let kind = raw[1];
    let len = u16::from_le_bytes([raw[3], raw[4]]) as usize;
    if version != FRAME_VERSION || kind != REQUEST_KIND_QUERY_PROGRAM || len < 1 || raw.len() < 5 + len {
        return None;
    }
    Some(raw[5])
}

fn status_byte(status: ProgramStatus) -> u8 {
    match status {
        ProgramStatus::Resolved => 0,
        ProgramStatus::NotFound => 1,
        ProgramStatus::Rhythm => 2,
        ProgramStatus::Editing => 3,
    }
}

fn encode_program_info_response(info: &ProgramInfo) -> Vec<u8> {
    let name_bytes = info.name.as_bytes();
    // 名前欄は1バイト長で表現するため255バイトに切り詰める（実運用の音色名はこれより十分短い）。
    let name_len = name_bytes.len().min(255);
    let name_bytes = &name_bytes[..name_len];

    let bank_bytes = info.bank.to_le_bytes();
    let mut payload = Vec::with_capacity(5 + name_len);
    payload.push(status_byte(info.status));
    payload.push(bank_bytes[0]);
    payload.push(bank_bytes[1]);
    payload.push(info.program);
    payload.push(name_len as u8);
    payload.extend_from_slice(name_bytes);

    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(FRAME_VERSION);
    frame.push(RESPONSE_KIND_PROGRAM_INFO);
    frame.push(0); // reserved
    frame.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    frame.extend_from_slice(&payload);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_request_frame(channel: u8) -> Vec<u8> {
        let mut frame = vec![FRAME_VERSION, REQUEST_KIND_QUERY_PROGRAM, 0u8];
        frame.extend_from_slice(&1u16.to_le_bytes());
        frame.push(channel);
        frame
    }

    #[test]
    fn decode_query_request_reads_channel() {
        let frame = build_request_frame(9);
        assert_eq!(decode_query_request(&frame), Some(9));
    }

    #[test]
    fn decode_query_request_rejects_wrong_version() {
        let mut frame = build_request_frame(0);
        frame[0] = FRAME_VERSION + 1;
        assert_eq!(decode_query_request(&frame), None);
    }

    #[test]
    fn decode_query_request_rejects_wrong_kind() {
        let mut frame = build_request_frame(0);
        frame[1] = RESPONSE_KIND_PROGRAM_INFO;
        assert_eq!(decode_query_request(&frame), None);
    }

    #[test]
    fn decode_query_request_rejects_truncated_frame() {
        let mut frame = build_request_frame(3);
        frame.truncate(frame.len() - 1);
        assert_eq!(decode_query_request(&frame), None);
    }

    #[test]
    fn decode_query_request_rejects_too_short_frame() {
        assert_eq!(decode_query_request(&[FRAME_VERSION, REQUEST_KIND_QUERY_PROGRAM]), None);
    }

    #[test]
    fn encode_program_info_response_round_trips_fields() {
        let info = ProgramInfo { bank: 300, program: 42, name: "Test Voice".to_string(), status: ProgramStatus::Resolved };
        let frame = encode_program_info_response(&info);

        assert_eq!(frame[0], FRAME_VERSION);
        assert_eq!(frame[1], RESPONSE_KIND_PROGRAM_INFO);
        let len = u16::from_le_bytes([frame[3], frame[4]]) as usize;
        let payload = &frame[5..5 + len];

        assert_eq!(payload[0], 0); // Resolved
        let bank = u16::from_le_bytes([payload[1], payload[2]]);
        assert_eq!(bank, 300);
        assert_eq!(payload[3], 42);
        let name_len = payload[4] as usize;
        assert_eq!(&payload[5..5 + name_len], b"Test Voice");
    }

    #[test]
    fn encode_program_info_response_truncates_long_name() {
        let long_name = "a".repeat(300);
        let info = ProgramInfo { bank: 0, program: 0, name: long_name, status: ProgramStatus::Resolved };
        let frame = encode_program_info_response(&info);
        let len = u16::from_le_bytes([frame[3], frame[4]]) as usize;
        let payload = &frame[5..5 + len];
        let name_len = payload[4] as usize;
        assert_eq!(name_len, 255);
    }
}
