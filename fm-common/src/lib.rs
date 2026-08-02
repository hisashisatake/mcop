//! fm-common: FM合成チップ間（`ym38x6-core`/`op505-core`）で共有する、EG非依存の汎用部品。
//!
//! アルゴリズム結線表・TL/KSR等のパラメーターマッピング・チップ内LFO・波形生成は
//! どのEG方式（レート方式/N点Time-Level方式）を採るFM合成チップにも共通して使える。
//! EGそのもの（`sound_core::Eg`/`TimeEg`）や各チップ固有のオペレーター/チャンネル実装は
//! ここに置かず、各チップのコアクレートに残す。
//!
//! 質感LFO本体（`TextureLfo`/`texture_lfo_to_shape`）はVCO実装に依存しないため
//! `sound-core`に属し、ここでは同名で再エクスポートするのみ（既存の参照パスを保つため）。
//! FM合成チップ固有の適用先解釈（`FmLfoDestination`）だけが`texture_lfo`モジュールに残る。

pub mod algorithm;
pub mod chip_lfo;
pub mod mapping;
pub mod texture_lfo;
pub mod waveform;

pub use sound_core::{texture_lfo_to_shape, TextureLfo};
pub use texture_lfo::FmLfoDestination;
