//! FM合成チップのエディタ間で共有するegui描画部品（ノブ・EGプレビュー・レイアウト・
//! アルゴリズム結線図・波形/enumセレクタ・パラメーターハンドルトレイト等）。
//! eguiとsound-coreのみに依存し、nice-plug/Tauri/cpalには依存しない
//! （`fm-common`のUI版に相当。ym38x6-ui/op505-ui双方が依存する）。

pub mod algorithm_diagram;
pub mod eg_preview;
#[cfg(feature = "preview")]
pub mod interpret; // フェーズB: panel.xmlのIRをランタイム解釈して実egui描画するプレビュー用インタープリタ。
pub mod knob;
pub mod layout;
pub mod level_meter;
pub mod mapping;
pub mod param_handle;
pub mod patchbay;
pub mod selector;
pub mod time_eg_editor;
pub mod time_eg_preview;
pub mod waveform;

// algorithm_diagram/eg_preview/knobは関数名がモジュール名と同一のため、crate root へ
// 再エクスポートすると（モジュールエイリアスと合わせて）型/値の両名前空間で二重定義になる
// （ym38x6-ui側で`use ui_core::{algorithm_diagram, eg_preview, knob, ...}`とモジュール
// エイリアスする際に顕在化する）。この3つはモジュールパス越し（`ui_core::knob::knob`等）
// でのみ公開し、crate root では公開しない。
pub use eg_preview::EgAmplitudeMapping;
pub use knob::{bool_checkbox, spin_control, Knob};
pub use param_handle::{BipolarHandle, BoolParamHandle, IntParamHandle, MeterHandle, TimeEgHandle, BIPOLAR_CENTER};
pub use selector::{
    enum_selector, sync_rate_display, sync_rate_selector, CHORUS_TYPE_NAMES, DELAY_SYNC_NAMES, FILTER_TYPE_NAMES,
    REVERB_TYPE_NAMES, SYNC_NOTE_NAMES,
};
// time_eg_previewも同様の理由でモジュールパス越し（`ui_core::time_eg_preview::time_eg_preview`）
// でのみ公開する。`TimeEgGeometry`/`time_eg_layout`は名前が衝突しないため再エクスポートする。
pub use time_eg_editor::TimeEgProfile;
pub use time_eg_preview::{time_eg_editor_layout, time_eg_layout, TimeEgGeometry};
pub use waveform::{waveform_selector, OPZ_VARIANTS, SINE_VARIANTS, SQUARE_VARIANTS, WAVEFORM_CATEGORIES};
