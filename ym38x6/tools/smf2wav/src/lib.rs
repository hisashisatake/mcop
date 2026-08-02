//! `.38x6` 音色バンクで標準MIDIファイル（SMF）を再生し、WAV に書き出す汎用ライブラリ。
//!
//! opz2x6 / mucom2x6 / vgm2x6 / psr2x6 など各変換ツールが出力した `.38x6` を
//! そのまま再生できる。変換ツール側からは [`bank::PatchBank::from_patches`] で
//! 変換直後のパッチ列を直接渡して再生することもできる。

pub mod bank;
pub mod fx;
pub mod midi;
pub mod render;
pub mod smf;
pub mod wav;

pub use bank::PatchBank;
pub use fx::{apply_reverb, ReverbConfig};
pub use render::render_smf;
pub use wav::{normalize_peak, write_wav_mono16};
