//! op505-mme-driver（Domino等のプロセスに住むMMEドライバDLL）からの名前付きパイプ受信サーバー。
//! （旧`mme_pipe`モジュールを`sources`配下へ移動したもの。フレーム形式・FFI設計は不変）
//!
//! フレーム形式（op505/mme-driver/src/client.rsのbuild_frameと対になる。変更時は両方揃える）:
//! `[u8 version=1][u8 kind][u8 device_id][u16 len（リトルエンディアン）][bytes; len]`
//! kind: 0=Short（デコード済み1〜3バイトのMIDIメッセージ）, 1=Long（SysEx。DLL側は
//! `DriverCallback`のMOM_DONE通知までバッファライフサイクルを正しく完結させる。
//! payloadをそのままMidiQueueへ積み、GM2 Universal SysEx（Master Volume等）の解釈は
//! `handle_midi_message`側で行う）,
//! 2=Reset（payload無し。全16chへAll Sound Off(CC120)を合成して送る）,
//! 3=OpenEditor（payload無し。トレイ起動音色エディタを開く/フォーカスする。MIDIメッセージ
//! ではないためMidiQueueを経由せず、直接`EditorHandle::show()`を呼ぶ。`gesture-app`の
//! Eキー押下専用で、`op505-mme-driver`（Domino等のクライアント）は送らない）。
//!
//! 名前付きパイプの「サーバー」役はRust標準ライブラリに無いAPI（CreateNamedPipeW /
//! ConnectNamedPipe）が必要なため、その2関数だけをkernel32.dllから直接FFI宣言する
//! （windows-sys等の外部crateは導入しない）。接続確立後の読み取り・クローズは
//! `std::fs::File`（`FromRawHandle`経由）に委ね、生のReadFile/CloseHandle呼び出しは書かない。
//!
//! acceptループの明示的な停止手段は現状無く、プロセス終了まで動き続ける
//! （タスクトレイ化フェーズで「終了」メニューの実装時に追加する）。
//!
//! パイプにはSDDL`S:(ML;;NW;;;LW)`（Low整合性レベルのプロセスからの書き込みも許可する
//! Mandatory Label）を明示的に付与する。既定のセキュリティ記述子はMedium整合性レベル
//! 相当で、Windowsの「No Write-Up」原則によりLow整合性レベルで動くクライアント
//! （サンドボックス化されたアプリ等）からは書き込めない。DACL自体はSDDLで指定せず
//! （呼び出し元プロセスの既定DACルールに委ねる）、整合性レベルのみを緩和する。
//! `ConvertStringSecurityDescriptorToSecurityDescriptorW`（advapi32.dll）が返す
//! セキュリティ記述子はプロセス終了まで使い続けるため意図的にリークする
//! （`Box::leak`。毎回のCreateNamedPipeW呼び出しで同じ記述子を再利用する）。

use std::fs::File;
use std::io::Read;
use std::os::windows::io::FromRawHandle;
use std::sync::{Arc, OnceLock};

use crate::editor::EditorHandle;
use crate::midi_source::{MidiSink, MidiSource};
use crate::tempo_clock::TempoClock;

const PIPE_PATH: &str = r"\\.\pipe\op505.mme.v1";
const PIPE_SDDL: &str = "S:(ML;;NW;;;LW)";

const FRAME_VERSION: u8 = 1;
const FRAME_KIND_SHORT: u8 = 0;
const FRAME_KIND_LONG: u8 = 1;
const FRAME_KIND_RESET: u8 = 2;
const FRAME_KIND_OPEN_EDITOR: u8 = 3;

#[allow(non_snake_case, non_camel_case_types, dead_code)]
mod ffi {
    use std::ffi::c_void;

    pub type HANDLE = *mut c_void;
    pub type BOOL = i32;
    pub type DWORD = u32;
    pub type LPVOID = *mut c_void;

    pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

    pub const PIPE_ACCESS_INBOUND: DWORD = 0x0000_0001;
    pub const PIPE_TYPE_MESSAGE: DWORD = 0x0000_0004;
    pub const PIPE_READMODE_MESSAGE: DWORD = 0x0000_0002;
    pub const PIPE_WAIT: DWORD = 0x0000_0000;
    pub const PIPE_UNLIMITED_INSTANCES: DWORD = 255;

    pub const SDDL_REVISION_1: DWORD = 1;

    /// SECURITY_ATTRIBUTES（winbase.h）。CreateNamedPipeWのlpSecurityAttributesへ渡す。
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

    // SDDL文字列 -> セキュリティ記述子の変換はadvapi32.dllが提供する（sddl.h/aclapi.h）。
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

/// `PIPE_SDDL`から生成したSECURITY_ATTRIBUTESをプロセス内で使い回す。生成に失敗したら
/// `None`を返し、呼び出し側はNULL（既定のセキュリティ記述子）へフォールバックする。
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
            "パイプのSDDLセキュリティ記述子生成に失敗しました（{}）。既定のACLで動作します。",
            std::io::Error::last_os_error()
        ));
        return None;
    }

    // プロセス終了まで使い続けるため意図的にリークする（accept_loopが繰り返し
    // 参照するプロセス全体で1個の記述子。都度生成/解放するとACLの一貫性を保つ理由がない）。
    let sa = Box::leak(Box::new(ffi::SecurityAttributes {
        n_length: std::mem::size_of::<ffi::SecurityAttributes>() as ffi::DWORD,
        lp_security_descriptor: descriptor,
        b_inherit_handle: 0,
    }));
    Some(sa as *const _ as usize)
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
///
/// `editor`はOpenEditorフレーム（kind=3）受信時に`EditorHandle::show()`を呼ぶために持つ
/// （`main`側で音色エディタスレッド起動後に渡される、モジュールdoc参照）。
/// `tempo`はMIDI Clock（0xF8/0xFA/0xFB/0xFC）を検出するために持つ（[`crate::tempo_clock`]の
/// モジュールdoc参照）。gesture-appのタップテンポ等、このパイプ経由で届くクロックにも対応する。
pub fn spawn(sink: MidiSink, editor: EditorHandle, tempo: Arc<TempoClock>) -> PipeSource {
    std::thread::spawn(move || accept_loop(sink, editor, tempo));
    PipeSource
}

fn accept_loop(sink: MidiSink, editor: EditorHandle, tempo: Arc<TempoClock>) {
    loop {
        let Some(handle) = create_pipe_instance() else {
            crate::log::log(&format!(
                "MMEパイプサーバーの作成に失敗しました（{}）。MMEドライバ経由の入力は無効です。",
                std::io::Error::last_os_error()
            ));
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
        let editor_for_client = editor.clone();
        let tempo_for_client = tempo.clone();
        std::thread::spawn(move || serve_client(file, sink_for_client, editor_for_client, tempo_for_client));
        // ループ先頭へ戻り、次のクライアント用に新しいインスタンスを作る
        // （複数のWinMMホストアプリが同時に接続してくる可能性があるため）。
    }
}

fn create_pipe_instance() -> Option<ffi::HANDLE> {
    let name = pipe_name_wide();
    // SDDL生成に失敗した場合はNULL（既定のセキュリティ記述子）へフォールバックする。
    let sa_ptr: *mut std::ffi::c_void = security_attributes()
        .map(|sa| sa as *const ffi::SecurityAttributes as *mut std::ffi::c_void)
        .unwrap_or(std::ptr::null_mut());
    let handle = unsafe {
        ffi::CreateNamedPipeW(
            name.as_ptr(),
            ffi::PIPE_ACCESS_INBOUND,
            ffi::PIPE_TYPE_MESSAGE | ffi::PIPE_READMODE_MESSAGE | ffi::PIPE_WAIT,
            ffi::PIPE_UNLIMITED_INSTANCES,
            0,    // out buffer size（受信専用なので0で可）
            4096, // in buffer size（フレームは高々8バイト程度なので十分すぎるほど余裕がある）
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

fn serve_client(mut file: File, sink: MidiSink, editor: EditorHandle, tempo: Arc<TempoClock>) {
    let mut buf = [0u8; 4096];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break, // クライアントが切断
            Ok(n) => {
                let raw = &buf[..n];
                if is_open_editor_frame(raw) {
                    editor.show();
                    continue;
                }
                for message in decode_frame(raw) {
                    match message.first() {
                        Some(0xF8) => tempo.on_clock_pulse(),
                        Some(0xFA | 0xFB | 0xFC) => tempo.on_transport_reset(),
                        _ => sink.push(message),
                    }
                }
            }
            Err(_) => break,
        }
    }
    // fileのDropでCloseHandleされる（DisconnectNamedPipeは呼ばない。このインスタンスは
    // 使い捨てで、次のクライアントには別インスタンスをaccept_loopが新規作成するため）。
}

/// OpenEditorフレーム（kind=3、payload無し）かどうかを判定する。MIDIバイト列ではなく
/// UIへの直接操作要求のため、`decode_frame`のVec<Vec<u8>>（MidiQueueへ積む想定の戻り値）
/// とは別経路で扱う。
fn is_open_editor_frame(raw: &[u8]) -> bool {
    raw.len() >= 5 && raw[0] == FRAME_VERSION && raw[1] == FRAME_KIND_OPEN_EDITOR
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
        // SysEx（0xF0開始・0xF7終端の完全なメッセージ）をそのままMidiQueueへ積む。
        // 解釈はhandle_midi_message側（GM2 Universal SysEx Master Volume等）が行う。
        FRAME_KIND_LONG => vec![payload.to_vec()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_frame(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![FRAME_VERSION, kind, 0u8];
        frame.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn short_frame_passes_payload_through() {
        let frame = build_frame(FRAME_KIND_SHORT, &[0x90, 60, 100]);
        assert_eq!(decode_frame(&frame), vec![vec![0x90, 60, 100]]);
    }

    #[test]
    fn reset_frame_expands_to_all_sound_off_on_16_channels() {
        let frame = build_frame(FRAME_KIND_RESET, &[]);
        let messages = decode_frame(&frame);
        assert_eq!(messages.len(), 16);
        assert_eq!(messages[0], vec![0xB0, 120, 0]);
        assert_eq!(messages[15], vec![0xBF, 120, 0]);
    }

    /// Long（SysEx）フレームはpayloadがそのままキューへ渡される
    /// （以前は未解釈のため破棄していたが、GM2 Master Volume対応で素通しに変更した）。
    #[test]
    fn long_frame_passes_sysex_payload_through() {
        let sysex = vec![0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x7F, 0xF7];
        let frame = build_frame(FRAME_KIND_LONG, &sysex);
        assert_eq!(decode_frame(&frame), vec![sysex]);
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let mut frame = build_frame(FRAME_KIND_SHORT, &[0x90, 60, 100]);
        frame.truncate(frame.len() - 1); // 宣言長よりpayloadが短い
        assert!(decode_frame(&frame).is_empty());
    }

    #[test]
    fn too_short_frame_is_rejected() {
        assert!(decode_frame(&[FRAME_VERSION, FRAME_KIND_SHORT]).is_empty());
    }

    #[test]
    fn wrong_version_is_rejected() {
        let mut frame = build_frame(FRAME_KIND_SHORT, &[0x90, 60, 100]);
        frame[0] = FRAME_VERSION + 1;
        assert!(decode_frame(&frame).is_empty());
    }

    #[test]
    fn unknown_kind_is_ignored() {
        let frame = build_frame(0xFF, &[0x01, 0x02]);
        assert!(decode_frame(&frame).is_empty());
    }
}
