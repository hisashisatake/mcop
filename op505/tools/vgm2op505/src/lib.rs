//! vgm2op505のライブラリ部分。演奏ロジック（VGM逐次デコード・OPM/OPN二系統・
//! SSGコアレス処理・PatchBank等）を`tests/`から利用可能にする（ゴールデンテスト用）。
//! CLI本体（引数パース・バンク書き出し等）は`src/main.rs`に残す。
//!
//! ym38x6/tools/vgm2x6からデフォーク（ym38x6依存排除）した独立実装。各モジュール冒頭に
//! 由来コミットを記載している（fork-on-write、以後は独立に進化させる）。

pub mod convert;
pub mod opm;
pub mod opn;
pub mod patch;
pub mod play;
pub mod smf;
pub mod ssg;
pub mod vgm;
