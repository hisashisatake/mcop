//! Windows MME（winmm.dll）のインストーラブルMIDIドライバABI。標準ヘッダ
//! （mmsystem.h/mmddk.h/mmeapi.h/mmiscapi.h）の定義を、外部crateへ依存せず必要最小限だけ
//! 手書き移植したもの（値の出典: mingw-w64/ReactOSのヘッダミラー。DriverProc/modMessageの
//! シグネチャ・MODM_*/MMSYSERR_*/DRV_*の数値は数十年変わっていない安定領域）。

#![allow(non_camel_case_types, dead_code)]

pub type DWORD = u32;
pub type UINT = u32;
pub type WORD = u16;
pub type LRESULT = isize;
pub type LPARAM = isize;
/// DWORD_PTR。ポインタサイズの符号無し整数（x86では4バイト、x64では8バイト）。
pub type DWORD_PTR = usize;
/// HDRVR（ドライバハンドル、不透明値）。中身を解釈しないため生の整数として扱う。
pub type HDRVR = isize;

// --- mmiscapi.h（DriverProc向け、ドライバ自体の有効化/無効化ライフサイクル） ---
pub const DRV_LOAD: UINT = 1;
pub const DRV_ENABLE: UINT = 2;
pub const DRV_OPEN: UINT = 3;
pub const DRV_CLOSE: UINT = 4;
pub const DRV_DISABLE: UINT = 5;
pub const DRV_FREE: UINT = 6;

// --- mmddk.h（modMessage向け、MIDI OUTデバイス単位のメッセージ） ---
pub const MODM_GETNUMDEVS: UINT = 1;
pub const MODM_GETDEVCAPS: UINT = 2;
pub const MODM_OPEN: UINT = 3;
pub const MODM_CLOSE: UINT = 4;
pub const MODM_PREPARE: UINT = 5;
pub const MODM_UNPREPARE: UINT = 6;
pub const MODM_DATA: UINT = 7;
pub const MODM_LONGDATA: UINT = 8;
pub const MODM_RESET: UINT = 9;
pub const MODM_GETVOLUME: UINT = 10;
pub const MODM_SETVOLUME: UINT = 11;
pub const MODM_CACHEPATCHES: UINT = 12;
pub const MODM_CACHEDRUMPATCHES: UINT = 13;
pub const MODM_STRMDATA: UINT = 14;

// --- mmsystem.h（MMSYSERR_BASE=0起点のエラーコード） ---
pub const MMSYSERR_NOERROR: DWORD = 0;
pub const MMSYSERR_ERROR: DWORD = 1;
pub const MMSYSERR_BADDEVICEID: DWORD = 2;
pub const MMSYSERR_NOTENABLED: DWORD = 3;
pub const MMSYSERR_ALLOCATED: DWORD = 4;
pub const MMSYSERR_INVALHANDLE: DWORD = 5;
pub const MMSYSERR_NOMEM: DWORD = 7;
pub const MMSYSERR_NOTSUPPORTED: DWORD = 8;
pub const MMSYSERR_INVALPARAM: DWORD = 11;
pub const MMSYSERR_NODRIVERCB: DWORD = 20;

// --- mmeapi.h（MIDIOUTCAPSW.wTechnology） ---
pub const MOD_SWSYNTH: WORD = 7;

pub const MAXPNAMELEN: usize = 32;

// --- mmsyscom.h（MODM_OPENのdwParam2で渡ってくるコールバック種別。mingw-w64ヘッダミラーで確認） ---
pub const CALLBACK_TYPEMASK: DWORD = 0x0007_0000;
pub const CALLBACK_FUNCTION: DWORD = 0x0003_0000;
pub const CALLBACK_EVENT: DWORD = 0x0005_0000;

// --- mmeapi.h（MOM_*、MIDI OUTドライバがDriverCallbackへ通知するメッセージ種別） ---
pub const MOM_DONE: DWORD = 0x3C9;

// --- mmsystem.h（MIDIHDR.dwFlags） ---
pub const MHDR_DONE: DWORD = 0x0000_0001;
pub const MHDR_PREPARED: DWORD = 0x0000_0002;

/// MIDIOPENDESC（mmddk.h）。クライアント（Domino等）がmidiOutOpenへ渡した情報をwinmmが
/// 詰め替えてMODM_OPENのdwParam1として渡してくる。`hMidi`は「DriverCallbackで通知する際に
/// 使うべきクライアント側ハンドル」とMSDN（learn.microsoft.com/.../ns-mmddk-midiopendesc）に
/// 明記されており、自前で値をでっち上げる必要はない。ストリームオープン専用の`rgIds`は
/// 単一デバイス・非ストリームの本ドライバでは常に`cIds=0`のため読み取らない。
#[repr(C)]
pub struct MidiOpenDesc {
    pub h_midi: DWORD_PTR,
    pub dw_callback: DWORD_PTR,
    pub dw_instance: DWORD_PTR,
    pub dn_dev_node: DWORD_PTR,
    pub c_ids: DWORD,
}

/// MIDIHDR（mmsystem.h）。MODM_LONGDATAのdwParam1が指す、クライアントが
/// `midiOutPrepareHeader`で準備済みのSysExバッファ記述子。
#[repr(C)]
pub struct MidiHdr {
    pub lp_data: *mut u8,
    pub dw_buffer_length: DWORD,
    pub dw_bytes_recorded: DWORD,
    pub dw_user: DWORD_PTR,
    pub dw_flags: DWORD,
    pub lp_next: *mut MidiHdr,
    pub reserved: DWORD_PTR,
    pub dw_offset: DWORD,
    pub dw_reserved: [DWORD_PTR; 8],
}

// winmm.dllが実際にエクスポートしている、インストーラブルドライバ専用のコールバック中継関数
// （winmm.libの一部。宣言はwine-mirror/wineのinclude/mmddk.hで確認済み。x64ではポインタ幅の
// 引数がDWORD_PTRへ拡張されている点が32bit専用の古いMSDN文書の記載と異なるため要注意）。
// このDLLはwinmm.dll自身に読み込まれて動く（modMessage経由で呼ばれる時点でプロセス内に
// winmm.dllが既にロード済み）ため、暗黙リンクで問題ない。
#[link(name = "winmm")]
extern "system" {
    pub fn DriverCallback(
        dw_callback: DWORD_PTR,
        u_flags: DWORD,
        h_dev: HDRVR,
        w_msg: DWORD,
        dw_user: DWORD_PTR,
        dw_param1: DWORD_PTR,
        dw_param2: DWORD_PTR,
    ) -> i32;
}

/// MIDIOUTCAPSW（mmeapi.h）。NT系のMMSYSTEMはドライバとの通信に常にワイド版構造体を使う
/// （ANSI/Wide変換はwinmm.dllのAPI層が担う）。フィールド順・型はヘッダと一致させること。
#[repr(C)]
pub struct MidiOutCapsW {
    pub w_mid: WORD,
    pub w_pid: WORD,
    pub v_driver_version: u32,
    pub sz_pname: [u16; MAXPNAMELEN],
    pub w_technology: WORD,
    pub w_voices: WORD,
    pub w_notes: WORD,
    pub w_channel_mask: WORD,
    pub dw_support: DWORD,
}

/// MIDIステータスバイトから、そのショートメッセージの全体バイト数（1〜3）を求める。
/// 標準的なMIDIステータスバイトの規約（チャンネルボイス/モードメッセージは種別で2〜3バイト、
/// システムメッセージは種別ごとに固定長）に基づく簡易判定。
pub fn short_message_len(status: u8) -> usize {
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 3,
        0xC0 | 0xD0 => 2,
        0xF0 => match status {
            0xF1 | 0xF3 => 2,
            0xF2 => 3,
            _ => 1,
        },
        _ => 1,
    }
}

/// `caps`へ固定のop505デバイス情報を書き込む。
///
/// # Safety
/// `caps`は`size_of::<MidiOutCapsW>()`バイト以上の書き込み可能な有効なポインタであること
/// （呼び出し側でNULLチェック・サイズチェック済みであること）。
pub unsafe fn fill_dev_caps(caps: *mut MidiOutCapsW) {
    let mut sz_pname = [0u16; MAXPNAMELEN];
    for (i, unit) in "op505".encode_utf16().take(MAXPNAMELEN - 1).enumerate() {
        sz_pname[i] = unit;
    }

    caps.write(MidiOutCapsW {
        // 未登録ベンダーの慣例値。実運用上はMIDI OUTデバイス一覧の表示名（szPname）が
        // 識別の要であり、wMid/wPidはほぼ参照されない。
        w_mid: 0xFFFF,
        w_pid: 1,
        v_driver_version: 0x0100,
        sz_pname,
        w_technology: MOD_SWSYNTH,
        // wVoices/wNotesはMOD_SQSYNTH専用フィールド（MSDN）。MOD_SWSYNTHでは0が正しい。
        w_voices: 0,
        w_notes: 0,
        w_channel_mask: 0xFFFF,
        // ボリューム制御等の追加機能は未対応（MIDICAPS_*フラグ無し）。
        dw_support: 0,
    });
}
