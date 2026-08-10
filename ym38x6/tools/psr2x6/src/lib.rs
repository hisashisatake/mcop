//! psr2x6 ライブラリ。PSR-70（OPQ/YM3806）ROM2音色パースと OPQ→ym38x6 変換を提供する。
//!
//! バイナリ（`main.rs`）から使うほか、op505直接変換（`psr2op505`）が `conv` の
//! `OpqVoice` / `OpqOperator` / レート写像関数を再利用する。

pub mod conv;
pub mod rom2;
