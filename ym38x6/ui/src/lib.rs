//! ym38x6エディタの共有描画ロジック。eguiとsound-coreのみに依存し、nice-plug/Tauri/cpalには
//! 依存しない。VST（ym38x6-vst）とgesture-appの両方から、パラメーターアクセスを`IntParamHandle`/
//! `BoolParamHandle`で適合させて利用する想定。
//!
//! knob/eg_preview/layout/param_handle/patchbay/selector/waveform/algorithm_diagramは
//! チップ非依存の汎用部品として`ui-core`クレートへ切り出し済み（op505-ui等、他チップの
//! エディタとも共有する）。ここでは crate root の名前をui-coreの同名モジュールへ
//! エイリアスすることで、`panel.xml`から生成される`panel_generated.rs`（`crate::knob::knob`
//! 等の**裸の名前解決**と、`crate::algorithm_diagram::carriers`等の**明示パス**の両方）を
//! 無改造のまま成立させている（生成コード・ゴールデンテストへの影響ゼロ）。
//!
//! フェーズBのプレビュー用インタープリタ（`interpret.rs`、`tools/xml-panel-dsl`のブラウザ/
//! ネイティブプレビューが使う）はチップ非依存化のため`ui-core`（`preview`フィーチャ配下）へ
//! 移設済み（Step 5）。本クレートは`preview`フィーチャを持たない。
pub(crate) use ui_core::{algorithm_diagram, eg_preview, knob, layout, param_handle, patchbay, selector, waveform};

mod panel;

pub use knob::{knob, spin_control, Knob};
pub use panel::{draw_param_panel, BipolarFgPanelParams, FgEgPanelParams, OperatorPanelParams, PanelParams};
pub use param_handle::{BoolParamHandle, IntParamHandle};
pub use selector::{enum_selector, CHORUS_TYPE_NAMES, REVERB_TYPE_NAMES};
pub use waveform::{waveform_selector, OPZ_VARIANTS, SINE_VARIANTS, SQUARE_VARIANTS, WAVEFORM_CATEGORIES};
