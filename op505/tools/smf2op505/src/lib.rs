//! `.op505` 音色バンクで標準MIDIファイル（SMF）を再生し、WAV に書き出す汎用ライブラリ。
//!
//! 各変換ツール（opz2op505 / mucom2op505 / vgm2op505 / psr2op505 等）が出力した `.op505` を
//! そのまま再生できる。変換ツール側からは [`bank::PatchBank::from_patches`] で
//! 変換直後のパッチ列を直接渡して再生することもできる。
//! `ym38x6/tools/smf2wav`のop505向け複製（fork-on-write）。

pub mod bank;
pub mod render;
pub mod smf;

pub use bank::PatchBank;
pub use op505_tools::fx::{apply_reverb, ReverbConfig};
pub use op505_tools::wav::{normalize_peak, write_wav_mono16};
pub use render::render_smf;
