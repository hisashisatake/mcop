use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use op505_core::Op505PresetBank;
use op505_ui::{draw_op505_panel, Op505BipolarFgPanelParams, Op505OperatorPanelParams, Op505PanelParams};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use crate::param_adapter::{vb, vi, vt};
use crate::params::{Op505EgBank, Op505VstParams, OperatorVstParams};
use crate::preset_host::VstPresetHost;
use op505_editor::preset_panel::{draw_presets_panel, EditorPresetState};

pub(crate) struct EditorState {
    pub(crate) presets: EditorPresetState,
}

/// オペレーター単位パネルパラメーターを組み立てる（`I`＝オペレーター番号0〜3）。
/// EGのget/setは`Op505EgBank.operators[I]`（配列要素そのものがTimeEgParams）を指す
/// 非キャプチャ関数ポインタ（`gesture-app/editor-wasm/src/op505_state.rs`の
/// `operator_panel_params::<const I: usize>`と同型）。
fn operator_panel<'a, const I: usize>(
    op: &'a OperatorVstParams,
    egs: &'a Arc<RwLock<Op505EgBank>>,
    setter: &'a ParamSetter<'a>,
) -> Op505OperatorPanelParams<'a> {
    let eg_name: &'static str = match I {
        0 => "OP1 EG",
        1 => "OP2 EG",
        2 => "OP3 EG",
        3 => "OP4 EG",
        _ => "OP EG",
    };
    Op505OperatorPanelParams {
        tl: vi(&op.tl, setter),
        eg: vt(egs, |b: &Op505EgBank| b.operators[I], |b: &mut Op505EgBank, v| b.operators[I] = v, eg_name),
        mul: vi(&op.mul, setter),
        dt1: vi(&op.dt1, setter),
        ksr: vi(&op.ksr, setter),
        vel_sens: vi(&op.vel_sens, setter),
        op_fine_tune: vi(&op.op_fine_tune, setter),
        ame: vb(&op.ame, setter),
        waveform: vi(&op.waveform, setter),
        eg_shift: vi(&op.eg_shift, setter),
        level_scale: vi(&op.level_scale, setter),
        velocity_gain: vi(&op.velocity_gain, setter),
    }
}

fn build_panel_params<'a>(
    params: &'a Op505VstParams,
    setter: &'a ParamSetter<'a>,
) -> Op505PanelParams<'a> {
    let egs = &params.egs;
    Op505PanelParams {
        algorithm: vi(&params.algorithm, setter),
        feedback: vi(&params.feedback, setter),
        fixed_note_enable: vb(&params.fixed_note_enable, setter),
        fixed_note: vi(&params.fixed_note, setter),
        fixed_note_fine: vi(&params.fixed_note_fine, setter),
        cutoff: vi(&params.cutoff, setter),
        resonance: vi(&params.resonance, setter),
        filter_type: vi(&params.filter_type, setter),
        filter_self_oscillation: vb(&params.filter_self_oscillation, setter),
        pitch_fg: Op505BipolarFgPanelParams {
            eg: vt(egs, |b: &Op505EgBank| b.pitch_fg, |b: &mut Op505EgBank, v| b.pitch_fg = v, "PITCH FG"),
            depth: vi(&params.pitch_fg_depth, setter),
        },
        cutoff_fg: Op505BipolarFgPanelParams {
            eg: vt(egs, |b: &Op505EgBank| b.cutoff_fg, |b: &mut Op505EgBank, v| b.cutoff_fg = v, "CUTOFF FG"),
            depth: vi(&params.cutoff_fg_depth, setter),
        },
        gain_fg: Op505BipolarFgPanelParams {
            eg: vt(egs, |b: &Op505EgBank| b.gain_fg, |b: &mut Op505EgBank, v| b.gain_fg = v, "GAIN FG"),
            depth: vi(&params.gain_fg_depth, setter),
        },
        gain_fg_to_master: vb(&params.gain_fg_to_master, setter),
        gain_fg_to_operators: vb(&params.gain_fg_to_operators, setter),
        rev_send: vi(&params.rev_send, setter),
        reverb_type: vi(&params.reverb_type, setter),
        reverb_time: vi(&params.reverb_time, setter),
        cho_send: vi(&params.cho_send, setter),
        chorus_type: vi(&params.chorus_type, setter),
        chorus_mod_rate: vi(&params.chorus_mod_rate, setter),
        chorus_mod_depth: vi(&params.chorus_mod_depth, setter),
        chorus_feedback: vi(&params.chorus_feedback, setter),
        chorus_send_to_reverb: vi(&params.chorus_send_to_reverb, setter),
        operators: [
            operator_panel::<0>(&params.operators[0], egs, setter),
            operator_panel::<1>(&params.operators[1], egs, setter),
            operator_panel::<2>(&params.operators[2], egs, setter),
            operator_panel::<3>(&params.operators[3], egs, setter),
        ],
    }
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
                        let panel = build_panel_params(&params, setter);
                        draw_op505_panel(ui, &panel);
                    });
                });
        },
    )
}
