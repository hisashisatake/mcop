//! ym38x6エディタの共有描画ロジック。eguiとsound-coreのみに依存し、nice-plug/Tauri/cpalには
//! 依存しない。VST（ym38x6-vst）とgesture-appの両方から、パラメーターアクセスを`IntParamHandle`/
//! `BoolParamHandle`で適合させて利用する想定。
//!
//! knob/eg_preview/layout/param_handle/patchbay/selector/waveform/algorithm_diagramは
//! チップ非依存の汎用部品として`ui-common`クレートへ切り出し済み（op505-ui等、他チップの
//! エディタとも共有する）。ここでは crate root の名前をui-commonの同名モジュールへ
//! エイリアスすることで、`panel.xml`から生成される`panel_generated.rs`（`crate::knob::knob`
//! 等の**裸の名前解決**と、`crate::algorithm_diagram::carriers`等の**明示パス**の両方）と
//! `interpret.rs`を無改造のまま成立させている（生成コード・ゴールデンテストへの影響ゼロ）。
pub(crate) use ui_common::{algorithm_diagram, eg_preview, knob, layout, param_handle, patchbay, selector, waveform};

#[cfg(feature = "preview")]
pub mod interpret; // フェーズB: panel.xmlのIRをランタイム解釈して実egui描画するプレビュー用インタープリタ。
mod panel;

pub use knob::{knob, spin_control, Knob};
pub use panel::{draw_param_panel, BipolarFgPanelParams, FgEgPanelParams, OperatorPanelParams, PanelParams};
pub use param_handle::{BoolParamHandle, IntParamHandle};
pub use selector::{enum_selector, CHORUS_TYPE_NAMES, REVERB_TYPE_NAMES};
pub use waveform::{waveform_selector, OPZ_VARIANTS, SINE_VARIANTS, SQUARE_VARIANTS, WAVEFORM_CATEGORIES};
