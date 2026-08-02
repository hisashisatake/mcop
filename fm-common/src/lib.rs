//! fm-common: FM合成チップ間（`ym38x6-core`/`op505-core`）で共有する、EG非依存の汎用部品。
//!
//! アルゴリズム結線表・TL/KSR等のパラメーターマッピング・チップ内LFO・波形生成・質感LFOは
//! どのEG方式（レート方式/N点Time-Level方式）を採るFM合成チップにも共通して使える。
//! EGそのもの（`sound_core::Eg`/`TimeEg`）や各チップ固有のオペレーター/チャンネル実装は
//! ここに置かず、各チップのコアクレートに残す。

pub mod algorithm;
pub mod chip_lfo;
pub mod mapping;
pub mod texture_lfo;
pub mod waveform;

pub use texture_lfo::{texture_lfo_to_shape, FmLfoDestination, TextureLfo};
