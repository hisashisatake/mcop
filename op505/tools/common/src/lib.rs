//! op505/tools/* 専用の共有クレート。WAV書き出し・ゴールデンテスト照合ヘルパー等、
//! チップ非依存の小道具のみを置く（何を置かないかはCargo.tomlのコメント参照）。

pub mod golden;
pub mod wav;
