//! op505をWindowsのMIDI OUTデバイス一覧に「op505」として登録する、ユーザーモードMMEドライバ。
//! DriverProc/modMessageをエクスポートし、winmm.dll経由でDomino等のレガシーWinMM
//! シーケンサから直接発音できるようにする（フェーズ0でDriverProc/modMessageの疎通を
//! x64/x86両方で実機確認済み。フェーズ1で[`client`]モジュールによる名前付きパイプ配線を追加）。
//!
//! 受信した短いMIDIメッセージ（MODM_DATA）・リセット（MODM_RESET）・SysEx（MODM_LONGDATA）は
//! いずれも[`client::send_frame`]でop505-standaloneへ転送する。MODM_LONGDATAはMIDIHDRの
//! バッファライフサイクル（`DriverCallback`によるMOM_DONE通知）まで正しく完結させるが、
//! SysExの中身自体はop505側で一切解釈せず破棄する（配線の堅牢化のみがフェーズ4のスコープ）。
//! デバッグ用の生ログは引き続き[`log`]モジュール（`%TEMP%\op505mme-spike.log`）へ残す。
//!
//! この.dllは相手アプリ（Domino等）のプロセス空間にロードされる。パニックでホストごと
//! 落とさないよう、エクスポート関数の本体は必ず[`std::panic::catch_unwind`]で包む
//! （ワークスペースの`panic="abort"`から本クレートを除外している理由と対になる防御）。

mod client;
mod log;
mod mm;

use std::panic::catch_unwind;
use std::sync::Mutex;

use mm::*;

/// MODM_OPENで受け取ったMIDIOPENDESC相当の情報。MODM_LONGDATA完了時にDriverCallbackへ
/// MOM_DONEを通知するために必要（`h_midi`はMSDN記載の通りDriverCallbackのhDev引数へ
/// そのまま渡すべき値、`dcb_flags`はCALLBACK_TYPEMASKでマスクした呼び出し種別）。
/// 単一デバイス・単一クライアント接続のみを想定し1個の静的スロットで保持する
/// （複数クライアント同時接続対応は別項目、フェーズ4の別タスク）。
struct OpenState {
    h_midi: DWORD_PTR,
    dw_callback: DWORD_PTR,
    dw_instance: DWORD_PTR,
    dcb_flags: DWORD,
}

static OPEN_STATE: Mutex<Option<OpenState>> = Mutex::new(None);

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DriverProc(
    _dw_driver_id: DWORD_PTR,
    _hdrvr: HDRVR,
    msg: UINT,
    _l_param1: LPARAM,
    _l_param2: LPARAM,
) -> LRESULT {
    catch_unwind(|| driver_proc_inner(msg)).unwrap_or(0)
}

fn driver_proc_inner(msg: UINT) -> LRESULT {
    log::log(&format!("DriverProc msg=0x{msg:X}"));
    match msg {
        DRV_LOAD | DRV_ENABLE | DRV_OPEN | DRV_CLOSE | DRV_DISABLE | DRV_FREE => 1,
        _ => 0,
    }
}

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn modMessage(
    u_device_id: UINT,
    u_msg: UINT,
    dw_user: DWORD_PTR,
    dw_param1: DWORD_PTR,
    dw_param2: DWORD_PTR,
) -> DWORD {
    catch_unwind(|| mod_message_inner(u_device_id, u_msg, dw_user, dw_param1, dw_param2)).unwrap_or(MMSYSERR_ERROR)
}

/// MODM_LONGDATA（SysEx）。中身は解釈せずパイプへ転送するだけの配線に留める
/// （op505側にSysEx解釈は無い。ここでの責務はMIDIHDRのバッファライフサイクルを正しく完結させ、
/// ホストアプリ（Domino等）が`midiOutUnprepareHeader`でハングしないようにすることのみ）。
fn mod_message_longdata(u_device_id: UINT, dw_param1: DWORD_PTR) -> DWORD {
    if dw_param1 == 0 {
        return MMSYSERR_INVALPARAM;
    }
    let hdr = dw_param1 as *mut MidiHdr;
    let (len, data) = unsafe { ((*hdr).dw_buffer_length, (*hdr).lp_data) };
    if data.is_null() {
        return MMSYSERR_INVALPARAM;
    }
    // フレーム形式のペイロード長はu16（build_frame参照）。実運用のSysExダンプがこれを
    // 超えることは稀だが、超えた場合はバッファを受理せず同期エラーを返す
    // （MHDR_DONEを立てずMOM_DONEも呼ばない＝クライアントは「即時失敗」として扱う、
    // 既存のMMSYSERR_NOTSUPPORTED応答と同じ安全な失敗経路）。
    if len > u16::MAX as u32 {
        log::log(&format!("modMessage LONGDATA device={u_device_id} len={len} が上限を超えるため拒否します"));
        return MMSYSERR_NOTSUPPORTED;
    }

    let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };
    log::log(&format!("modMessage LONGDATA device={u_device_id} len={len}（内容は転送のみ、op505側では未解釈）"));
    client::send_frame(client::build_frame(client::FRAME_KIND_LONG, u_device_id as u8, bytes));

    unsafe { (*hdr).dw_flags |= MHDR_DONE };

    if let Some(open) = OPEN_STATE.lock().unwrap().as_ref() {
        unsafe {
            DriverCallback(open.dw_callback, open.dcb_flags, open.h_midi as HDRVR, MOM_DONE, open.dw_instance, dw_param1, 0);
        }
    } else {
        log::log("modMessage LONGDATA: OPEN状態が無くDriverCallbackを呼べません（想定外のシーケンス）");
    }

    MMSYSERR_NOERROR
}

fn mod_message_inner(u_device_id: UINT, u_msg: UINT, dw_user: DWORD_PTR, dw_param1: DWORD_PTR, dw_param2: DWORD_PTR) -> DWORD {
    match u_msg {
        // 唯一、戻り値が「デバイス数」そのものになるメッセージ（MMSYSERR_*ではない）。
        MODM_GETNUMDEVS => {
            log::log("modMessage GETNUMDEVS -> 1");
            1
        }

        MODM_GETDEVCAPS => {
            log::log(&format!("modMessage GETDEVCAPS device={u_device_id} cb={dw_param2}"));
            if dw_param1 == 0 {
                return MMSYSERR_INVALPARAM;
            }
            if dw_param2 < core::mem::size_of::<MidiOutCapsW>() {
                return MMSYSERR_INVALPARAM;
            }
            let caps = dw_param1 as *mut MidiOutCapsW;
            unsafe { fill_dev_caps(caps) };
            MMSYSERR_NOERROR
        }

        MODM_OPEN => {
            log::log(&format!("modMessage OPEN device={u_device_id}"));
            if dw_user == 0 || dw_param1 == 0 {
                return MMSYSERR_INVALPARAM;
            }
            if !client::open() {
                log::log("modMessage OPEN failed: op505-standaloneのパイプサーバーに接続できません");
                return MMSYSERR_NOTENABLED;
            }
            // dw_param1はMIDIOPENDESC*、dw_param2はmidiOutOpen呼び出し元が指定したコールバック
            // 種別フラグ（CALLBACK_FUNCTION等）。MODM_LONGDATA完了時のDriverCallback呼び出しに
            // 必要なため保存しておく（フェーズ4 SysEx対応）。
            let desc = dw_param1 as *const MidiOpenDesc;
            let (h_midi, dw_callback, dw_instance) = unsafe { ((*desc).h_midi, (*desc).dw_callback, (*desc).dw_instance) };
            // DriverCallbackのuFlags引数はCALLBACK_*そのものではなくDCB_*（mmddk.hのマスク値の
            // 上位ワードぶんだけ右シフトした値。DCB_FUNCTION=3=HIWORD(CALLBACK_FUNCTION=0x30000)、
            // learn.microsoft.com/.../ms708182のDriverCallback解説で確認済み）。
            let dcb_flags = ((dw_param2 as DWORD) & CALLBACK_TYPEMASK) >> 16;
            *OPEN_STATE.lock().unwrap() = Some(OpenState { h_midi, dw_callback, dw_instance, dcb_flags });

            // このメッセージに限り dw_user は DWORD_PTR* （出力先）。以降のメッセージでは
            // ここに書いた値がそのまま dw_user として渡ってくる。デバイス1個のみのため
            // 区別不要で固定値を書くだけ。
            let out = dw_user as *mut usize;
            unsafe { *out = 1 };
            MMSYSERR_NOERROR
        }

        MODM_CLOSE => {
            log::log(&format!("modMessage CLOSE device={u_device_id}"));
            *OPEN_STATE.lock().unwrap() = None;
            MMSYSERR_NOERROR
        }

        MODM_DATA => {
            // dw_param1はステータス/データ1/データ2をリトルエンディアンでパックした短いMIDI
            // メッセージ（下位1バイトがステータス）。ステータスバイトから実際のメッセージ長
            // （1〜3バイト）を求め、その分だけをフレームへ詰めて送る
            // （op505-standalone側はop505-midiで正規の長さ判定をするため、ここでの判定は
            // 「送るバイト数」を決めるためだけの簡易実装でよい）。
            let status = (dw_param1 & 0xFF) as u8;
            let data1 = ((dw_param1 >> 8) & 0xFF) as u8;
            let data2 = ((dw_param1 >> 16) & 0xFF) as u8;
            let bytes: &[u8] = match short_message_len(status) {
                1 => &[status],
                2 => &[status, data1][..],
                _ => &[status, data1, data2][..],
            };
            log::log(&format!("modMessage DATA device={u_device_id} bytes={bytes:02X?}"));
            client::send_frame(client::build_frame(client::FRAME_KIND_SHORT, u_device_id as u8, bytes));
            MMSYSERR_NOERROR
        }

        MODM_LONGDATA => mod_message_longdata(u_device_id, dw_param1),

        MODM_RESET => {
            log::log(&format!("modMessage RESET device={u_device_id}"));
            client::send_frame(client::build_frame(client::FRAME_KIND_RESET, u_device_id as u8, &[]));
            MMSYSERR_NOERROR
        }

        // MHDR_PREPARED/MHDR_DONEはwinmm自身ではなくドライバが立てる/降ろす責務
        // （mmddk.hのMODM_PREPARE/MODM_UNPREPARE規約）。ここを素通りにしていたのが
        // 「MODM_LONGDATAがMIDIERR_UNPREPAREDで即座に失敗する」不具合の原因だった
        // （winmmのmidiOutLongMsgクライアント側チェックがdw_param1の指すMIDIHDRの
        // dwFlagsを直接見ており、ドライバへ到達する前に弾いていた）。
        MODM_PREPARE => {
            if dw_param1 == 0 {
                return MMSYSERR_INVALPARAM;
            }
            let hdr = dw_param1 as *mut MidiHdr;
            unsafe { (*hdr).dw_flags |= MHDR_PREPARED };
            MMSYSERR_NOERROR
        }

        MODM_UNPREPARE => {
            if dw_param1 == 0 {
                return MMSYSERR_INVALPARAM;
            }
            let hdr = dw_param1 as *mut MidiHdr;
            unsafe { (*hdr).dw_flags &= !MHDR_PREPARED };
            MMSYSERR_NOERROR
        }

        MODM_GETVOLUME | MODM_SETVOLUME | MODM_CACHEPATCHES | MODM_CACHEDRUMPATCHES | MODM_STRMDATA => {
            MMSYSERR_NOTSUPPORTED
        }

        other => {
            log::log(&format!("modMessage 未知のメッセージ msg=0x{other:X} device={u_device_id}"));
            MMSYSERR_NOTSUPPORTED
        }
    }
}
