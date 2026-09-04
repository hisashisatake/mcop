//! OP505（N点Time/Level方式EG）のエディタパネル描画ロジック。eguiとsound-coreのみに依存し、
//! nice-plug/Tauri/cpalには依存しない（`ym38x6-ui`のOP505版に相当）。
//!
//! `ym38x6-ui`と同じく`panel.xml`（正本）→`ui-codegen`→`build.rs`生成方式（Step 4）。
//! 各EG（OP×4のオペレーターEG＋PITCH/CUTOFF/GAIN FG）は`<time-eg-editor>`要素経由で
//! `ui_core::time_eg_editor`（折れ線ドラッグ編集/ノブ編集をタブで切り替えるハイブリッドエディタ、
//! Step 2で固定枠＋内部スクロール化済み）へ委譲する。
//!
//! knob/layout/param_handle/patchbay/selector/waveform/algorithm_diagram/time_eg_editorは
//! チップ非依存の汎用部品として`ui-core`クレートへ切り出し済み（ym38x6-ui等、他チップの
//! エディタとも共有する）。ここでは crate root の名前をui-coreの同名モジュールへ
//! エイリアスすることで、`panel.xml`から生成される`panel_generated.rs`（`crate::knob::knob`
//! 等の**裸の名前解決**と、`crate::algorithm_diagram::carriers`等の**明示パス**の両方）を
//! 無改造のまま成立させている（`ym38x6-ui/src/lib.rs`と同じ設計）。
// `patchbay`は現在panel.xmlにjack要素が無いため未使用（質感LFO退役でパッチベイの利用先が
// 消えたため）。将来OP/フィードバック等への配線でジャックを復活させる布石として、
// 描画コード自体は`ui-core`に温存する方針（memory `project_texture_lfo_retirement.md`参照）。
#[allow(unused_imports)]
pub(crate) use ui_core::{algorithm_diagram, knob, layout, level_meter, param_handle, patchbay, selector, time_eg_editor, waveform};

mod panel;

// editor-wasm(op505_state.rs/app.rs)がハンドル実装・パネル描画で使うため再エクスポートする。
// PANEL_MIN_WIDTHはpanel.xmlから算出した「重ならずに収まる最小幅」（ui-codegen生成）。
// ホスト側（op505-vst/gesture-app）がウィンドウ最小サイズを決めるのに使う。
pub use panel::{draw_op505_panel, Op505BipolarFgPanelParams, Op505OperatorPanelParams, Op505PanelParams, PANEL_MIN_WIDTH};
pub use param_handle::{BoolParamHandle, IntParamHandle, MeterHandle, TimeEgHandle};
