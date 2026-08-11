//! vgm2x6 の演奏ロジック共有ライブラリ。
//!
//! VGM/VGZ デコード・OPM/OPNレジスタ状態機・SSG合成・SMF生成・音色バンク管理を
//! bin（vgm2x6本体）から使うための lib クレート。パッチ/エンジン型はここでは
//! `ym38x6-core` の具象型に固定されている。
//! `op505/tools/vgm2op505`はop505デフォーク（2026-08-11）でこのlibから独立複製したため、
//! 現在は再利用していない（fork-on-write。詳細はspec-fm.md 8章）。

pub mod opm;
pub mod opn;
pub mod patch;
pub mod play;
pub mod smf;
pub mod ssg;
pub mod vgm;
