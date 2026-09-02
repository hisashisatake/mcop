use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use op505_core::Op505PresetBank;
use op505_editor::layout::{editor_min_size, PRESETS_SIDEBAR_WIDTH};
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

/// VSTエディタウィンドウの最小高さ。`ResizableWindow`内はスクロールするため小さめでよい
/// （standaloneの固定サイズ720pxとは異なる事情、詳細は`op505_editor::layout`のdoc参照）。
const MIN_HEIGHT: f32 = 480.0;

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
                .min_size(editor_min_size(MIN_HEIGHT))
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
