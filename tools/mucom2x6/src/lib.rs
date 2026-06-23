//! mucom2x6 ライブラリ。MUCOM88 バイナリ音色バンクのパースと OPN→ym38x6 変換を提供する。
//!
//! バイナリ（`main.rs`）から使うほか、OPN系VGM変換（`vgm2x6`）が `conv` の
//! `OpnVoice` / `OpnOperator` / `to_ym38x6_patch()` を再利用する。

pub mod conv;
pub mod mucom88;
