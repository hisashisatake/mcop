/// MIDI CC値（0.0〜1.0正規化）を本プロジェクトの内部表現（0〜255）に変換する。
pub fn cc_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// MIDI CC値（0.0〜1.0正規化）をGM2準拠の7bit値（0〜127）に変換する。
pub fn cc_to_u7(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 127.0).round() as u8
}

/// MIDI/SMFの生CC値（0〜127の7bit整数）を内部表現（0〜255）に変換する（`cc_to_u8`はVST
/// のf32正規化パラメーター向けのため、整数バイト値を受け取るこちらを橋渡しに使う）。
pub fn cc_byte_to_u8(value: u8) -> u8 {
    cc_to_u8(value as f32 / 127.0)
}

/// MIDI/SMFの生CC値（0〜127）をそのまま7bit値として使う（`cc_to_u7`と同じ丸め規則を通す）。
pub fn cc_byte_to_u7(value: u8) -> u8 {
    cc_to_u7(value as f32 / 127.0)
}
