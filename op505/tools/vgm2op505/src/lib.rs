//! vgm2op505のライブラリ部分。`Op505Converter`/`Engine505`/`AttackMode`を`tests/`から
//! 利用可能にする（`vgm2x6::play::opm_to_smf`等を通したゴールデンテスト用）。
//! CLI本体（引数パース・バンク書き出し等）は`src/main.rs`に残す。

pub mod convert;
