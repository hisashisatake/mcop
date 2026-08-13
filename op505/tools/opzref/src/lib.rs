//! opzref のレジスタ計算ロジック（`main.rs`のバイナリ本体から分離し、ゴールデンテストから
//! 参照できるようにする）。
//!
//! 由来: ym38x6/tools/opzref4x6/src/main.rs（コミット b61ba7a 時点の複製、2026-08-13）。
//! デフォーク後のop505ツール群向け複製（fork-on-write）。
//! ym38x6/tools/opzref4x6側の修正は自動では反映されない。

pub mod attls;
pub mod regs;
