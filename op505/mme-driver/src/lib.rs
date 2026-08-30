//! op505をWindowsのMIDI OUTデバイス一覧に「op505」として登録する、ユーザーモードMMEドライバの
//! 実現可能性スパイク（フェーズ0）。DriverProc/modMessageをエクスポートし、winmm.dllから
//! 正しく認識されるか、MODM_DATAで実際にMIDIバイト列が届くかを検証する。
//!
//! この段階ではop505-standaloneへの配線は行わない。受信したMIDIバイト列は一時ログ
//! （[`log`]モジュール、`%TEMP%\op505mme-spike.log`）へ書き出すだけで、
//! 「データが実際に流れるか」の証明に専念する（名前付きパイプでの本配線は後続フェーズ）。
//!
//! この.dllは相手アプリ（Domino等）のプロセス空間にロードされる。パニックでホストごと
//! 落とさないよう、エクスポート関数の本体は必ず[`std::panic::catch_unwind`]で包む
//! （ワークスペースの`panic="abort"`から本クレートを除外している理由と対になる防御）。

mod log;
mod mm;

use std::panic::catch_unwind;

use mm::*;

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
            if dw_user == 0 {
                return MMSYSERR_INVALPARAM;
            }
            // このメッセージに限り dw_user は DWORD_PTR* （出力先）。以降のメッセージでは
            // ここに書いた値がそのまま dw_user として渡ってくる。スパイクでは区別不要のため
            // 固定値を書くだけ。
            let out = dw_user as *mut usize;
            unsafe { *out = 1 };
            MMSYSERR_NOERROR
        }

        MODM_CLOSE => {
            log::log(&format!("modMessage CLOSE device={u_device_id}"));
            MMSYSERR_NOERROR
        }

        MODM_DATA => {
            // dw_param1はステータス/データ1/データ2をリトルエンディアンでパックした短いMIDI
            // メッセージ（下位1バイトがステータス）。正確な長さ判定はop505-midi側の役割なので
            // ここでは行わず、届いた3バイトをそのままログに残す（データ到達の証明が目的）。
            let status = (dw_param1 & 0xFF) as u8;
            let data1 = ((dw_param1 >> 8) & 0xFF) as u8;
            let data2 = ((dw_param1 >> 16) & 0xFF) as u8;
            log::log(&format!(
                "modMessage DATA device={u_device_id} bytes=[{status:#04X} {data1:#04X} {data2:#04X}]"
            ));
            MMSYSERR_NOERROR
        }

        MODM_LONGDATA => {
            log::log(&format!("modMessage LONGDATA device={u_device_id}（SysEx、フェーズ0では未実装）"));
            MMSYSERR_NOTSUPPORTED
        }

        MODM_RESET => {
            log::log(&format!("modMessage RESET device={u_device_id}"));
            MMSYSERR_NOERROR
        }

        MODM_PREPARE | MODM_UNPREPARE => MMSYSERR_NOERROR,

        MODM_GETVOLUME | MODM_SETVOLUME | MODM_CACHEPATCHES | MODM_CACHEDRUMPATCHES | MODM_STRMDATA => {
            MMSYSERR_NOTSUPPORTED
        }

        other => {
            log::log(&format!("modMessage 未知のメッセージ msg=0x{other:X} device={u_device_id}"));
            MMSYSERR_NOTSUPPORTED
        }
    }
}
