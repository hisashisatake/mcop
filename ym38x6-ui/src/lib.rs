//! ym38x6エディタの共有描画ロジック。eguiとsound-coreのみに依存し、nice-plug/Tauri/cpalには
//! 依存しない。VST（ym38x6-vst）とgesture-appの両方から、パラメーターアクセスを`IntParamHandle`/
//! `BoolParamHandle`で適合させて利用する想定。

mod algorithm_diagram;
mod eg_preview;
mod knob;
mod panel;
mod param_handle;
mod selector;
mod waveform;

pub use knob::{knob, spin_control, Knob};
pub use panel::{draw_param_panel, BipolarFgPanelParams, FgEgPanelParams, OperatorPanelParams, PanelParams};
pub use param_handle::{BoolParamHandle, IntParamHandle};
pub use selector::{enum_selector, CHORUS_TYPE_NAMES, REVERB_TYPE_NAMES};
pub use waveform::{waveform_selector, OPZ_VARIANTS, SINE_VARIANTS, SQUARE_VARIANTS, WAVEFORM_CATEGORIES};
