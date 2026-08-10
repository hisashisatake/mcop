//! vgm2x6 の演奏ロジック共有ライブラリ。
//!
//! VGM/VGZ デコード・OPM/OPNレジスタ状態機・SSG合成・SMF生成・音色バンク管理を
//! bin（vgm2x6本体）と外部クレート（vgm2op505等の直接変換ツール）から共有するための
//! lib クレート。パッチ/エンジン型はここでは `ym38x6-core` の具象型に固定されている
//! （ジェネリクス化は将来の変更で行う）。

pub mod opm;
pub mod opn;
pub mod patch;
pub mod play;
pub mod smf;
pub mod ssg;
pub mod vgm;
