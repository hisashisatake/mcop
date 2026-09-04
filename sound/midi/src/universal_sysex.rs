// ---------------------------------------------------------------------------
// GM2 Universal SysExパーサ
// ---------------------------------------------------------------------------
//
// MIDI規格のUniversal Real Time / Non-Real Time SysExのうち、GM2で意味が
// 固定されているDevice Controlメッセージを解釈する。将来Master Balance
// (F0 7F <dev> 04 02 ..)・GM System On (F0 7E <dev> 09 01 F7)等を足すときは
// UniversalSysExへバリアントを追加し、parse_universal_sysex()のmatchへ
// 分岐を足すだけでよい形にしてある。

/// パース済みのUniversal SysExメッセージ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UniversalSysEx {
    /// Universal Real Time, Device Control, Master Volume
    /// （`F0 7F <device_id> 04 01 <LSB> <MSB> F7`）。`value14`は0〜16383。
    MasterVolume { device_id: u8, value14: u16 },
}

/// 完全な1個のSysExメッセージ（先頭`0xF0`・末尾`0xF7`を含む）をパースする。
/// 対応外の形式（未知のsub-id、長さ不正、データバイト範囲外等）は`None`を返す。
pub fn parse_universal_sysex(bytes: &[u8]) -> Option<UniversalSysEx> {
    match *bytes {
        [0xF0, 0x7F, device_id, 0x04, 0x01, lsb, msb, 0xF7] if lsb < 0x80 && msb < 0x80 => {
            let value14 = ((msb as u16) << 7) | (lsb as u16);
            Some(UniversalSysEx::MasterVolume { device_id, value14 })
        }
        _ => None,
    }
}

/// GM2の14bit値（0〜16383）を、このプロジェクト共通の0〜255表現へ写像する。
/// `value14 >> 6`により16383は255へちょうど届く（上位8bitを取り出す形）。
pub fn value14_to_u8(value14: u16) -> u8 {
    (value14 >> 6).min(255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_master_volume() {
        let bytes = [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x7F, 0x7F, 0xF7];
        let result = parse_universal_sysex(&bytes);
        assert_eq!(result, Some(UniversalSysEx::MasterVolume { device_id: 0x7F, value14: 16383 }));
    }

    #[test]
    fn parses_zero_volume() {
        let bytes = [0xF0, 0x7F, 0x00, 0x04, 0x01, 0x00, 0x00, 0xF7];
        let result = parse_universal_sysex(&bytes);
        assert_eq!(result, Some(UniversalSysEx::MasterVolume { device_id: 0x00, value14: 0 }));
    }

    #[test]
    fn device_id_is_preserved() {
        let bytes = [0xF0, 0x7F, 0x05, 0x04, 0x01, 0x10, 0x20, 0xF7];
        let result = parse_universal_sysex(&bytes);
        assert_eq!(result, Some(UniversalSysEx::MasterVolume { device_id: 0x05, value14: (0x20 << 7) | 0x10 }));
    }

    #[test]
    fn rejects_wrong_start_byte() {
        let bytes = [0xF1, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x7F, 0xF7];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn rejects_non_realtime_category() {
        // F0 7E は Universal Non-Real Time（GM System On等）で Master Volume ではない。
        let bytes = [0xF0, 0x7E, 0x7F, 0x04, 0x01, 0x00, 0x7F, 0xF7];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn rejects_wrong_sub_id1() {
        let bytes = [0xF0, 0x7F, 0x7F, 0x05, 0x01, 0x00, 0x7F, 0xF7];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn rejects_wrong_sub_id2() {
        // sub-id2=02はMaster Balance（未対応）。
        let bytes = [0xF0, 0x7F, 0x7F, 0x04, 0x02, 0x00, 0x7F, 0xF7];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn rejects_missing_terminator() {
        let bytes = [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x7F];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn rejects_too_short() {
        let bytes = [0xF0, 0x7F, 0x7F, 0x04, 0x01];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn rejects_too_long() {
        let bytes = [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x7F, 0x00, 0xF7];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn rejects_empty_slice() {
        assert_eq!(parse_universal_sysex(&[]), None);
    }

    #[test]
    fn rejects_data_byte_out_of_range() {
        // MSB/LSBはデータバイト（0〜127）である必要がある。
        let bytes = [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x80, 0x7F, 0xF7];
        assert_eq!(parse_universal_sysex(&bytes), None);
    }

    #[test]
    fn value14_to_u8_maps_full_range() {
        assert_eq!(value14_to_u8(0), 0);
        assert_eq!(value14_to_u8(16383), 255);
        assert_eq!(value14_to_u8(8192), 128);
    }
}
