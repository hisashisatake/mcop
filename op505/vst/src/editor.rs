use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use op505_core::Op505PresetBank;
use op505_editor::panel_source::build_panel_params;
use op505_editor::preset_panel::{draw_presets_panel, EditorPresetState};
use op505_ui::draw_op505_panel;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use crate::param_adapter::VstPanelSource;
use crate::params::Op505VstParams;
use crate::preset_host::VstPresetHost;

pub(crate) struct EditorState {
    pub(crate) presets: EditorPresetState,
}

/// PRESETSサイドバー（固定幅）の幅。Open/Save/Save Asの3ボタンが折り返さず並ぶ幅として200へ
/// 拡張した（旧180px。`.exact_size`側もこの定数を直接使うこと——過去にリテラル直書きと定数が
/// 二重管理になっており、幅変更が`editor_min_size()`にしか反映されない潜在バグがあった）。
const PRESETS_SIDEBAR_WIDTH: f32 = 200.0;
/// ResizableWindow自身と`CentralPanel::default()`（draw_op505_panelを包む方）の
/// `inner_margin(8)`が左右で効く分の安全マージン（実測ではなく余裕を見た概算値）。
const WINDOW_CHROME_SLACK: f32 = 40.0;

/// エディタウィンドウの最小幅。`op505_ui::PANEL_MIN_WIDTH`（panel.xmlから算出した
/// 「ノブ等がtime-eg-editorへ食い込まずに収まる最小幅」）にPRESETSサイドバーぶんを足したもの。
/// panel.xmlの内容が変わればここも自動追従する。
pub(crate) fn editor_min_size() -> (f32, f32) {
    (op505_ui::PANEL_MIN_WIDTH + PRESETS_SIDEBAR_WIDTH + WINDOW_CHROME_SLACK, 480.0)
}

pub(crate) fn create_editor(
    egui_state: Arc<EguiState>,
    params: Arc<Op505VstParams>,
    shared_preset_bank: Arc<RwLock<Op505PresetBank>>,
    preset_bank_dirty: Arc<AtomicBool>,
) -> Option<Box<dyn Editor>> {
    let resize_state = egui_state.clone();
    create_egui_editor(
        egui_state,
        EditorState { presets: EditorPresetState::new() },
        EguiSettings::default(),
        |_ctx, _queue, _state| {},
        move |ui, setter, _queue, state| {
            ResizableWindow::new("op505_resize")
                .min_size(editor_min_size())
                .show(ui, &resize_state, |ui| {
                    // ---- プリセットブラウザ（左サイドバー・縦いっぱい） ----
                    egui::Panel::left("presets_panel")
                        .resizable(false)
                        .exact_size(PRESETS_SIDEBAR_WIDTH)
                        .show_inside(ui, |ui| {
                            let host = VstPresetHost {
                                params: &params,
                                setter,
                                shared_bank: &shared_preset_bank,
                                dirty: &preset_bank_dirty,
                            };
                            draw_presets_panel(ui, &mut state.presets, &host);
                        });

                    // ---- 残りのパラメーター（右側・縦スクロール、op505-uiの共有レイアウト） ----
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        let source = VstPanelSource { params: &params, setter };
                        let panel = build_panel_params(&source);
                        draw_op505_panel(ui, &panel);
                    });
                });
        },
    )
}
