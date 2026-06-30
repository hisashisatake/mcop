use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use std::sync::{Arc, Mutex};
use ym38x6_core::{presets_dir, PresetBank, Ym38x6Patch};
use ym38x6_ui::{draw_param_panel, OperatorPanelParams, PanelParams};

use crate::param_adapter::{vb, vi};
use crate::params::{OperatorVstParams, Ym38x6Params};

pub(crate) struct EditorState {
    pub(crate) preset_bank: PresetBank,
    pub(crate) selected_key: Option<(u16, u8)>,
}

fn operator_panel<'a>(
    op: &'a OperatorVstParams,
    setter: &'a ParamSetter<'a>,
) -> OperatorPanelParams<'a> {
    OperatorPanelParams {
        tl: vi(&op.tl, setter),
        ar: vi(&op.ar, setter),
        d1r: vi(&op.d1r, setter),
        d2r: vi(&op.d2r, setter),
        d1l: vi(&op.d1l, setter),
        rr: vi(&op.rr, setter),
        mul: vi(&op.mul, setter),
        dt1: vi(&op.dt1, setter),
        ksr: vi(&op.ksr, setter),
        vel_sens: vi(&op.vel_sens, setter),
        op_fine_tune: vi(&op.op_fine_tune, setter),
        ame: vb(&op.ame, setter),
        waveform: vi(&op.waveform, setter),
    }
}

fn build_panel_params<'a>(
    params: &'a Ym38x6Params,
    setter: &'a ParamSetter<'a>,
) -> PanelParams<'a> {
    PanelParams {
        algorithm: vi(&params.algorithm, setter),
        feedback: vi(&params.feedback, setter),
        lfo_rate: vi(&params.lfo_rate, setter),
        lfo_depth: vi(&params.lfo_depth, setter),
        lfo_delay: vi(&params.lfo_delay, setter),
        tone_freq: vi(&params.tone_freq, setter),
        tone_pmd: vi(&params.tone_pmd, setter),
        tone_amd: vi(&params.tone_amd, setter),
        tone_delay: vi(&params.tone_delay, setter),
        pms: vi(&params.pms, setter),
        ams: vi(&params.ams, setter),
        cutoff: vi(&params.cutoff, setter),
        resonance: vi(&params.resonance, setter),
        feg_a: vi(&params.feg_a, setter),
        feg_d: vi(&params.feg_d, setter),
        feg_s: vi(&params.feg_s, setter),
        feg_r: vi(&params.feg_r, setter),
        feg_depth: vi(&params.feg_depth, setter),
        rev_send: vi(&params.rev_send, setter),
        reverb_type: vi(&params.reverb_type, setter),
        cho_send: vi(&params.cho_send, setter),
        chorus_type: vi(&params.chorus_type, setter),
        reverb_time: vi(&params.reverb_time, setter),
        chorus_mod_rate: vi(&params.chorus_mod_rate, setter),
        chorus_mod_depth: vi(&params.chorus_mod_depth, setter),
        chorus_feedback: vi(&params.chorus_feedback, setter),
        chorus_send_to_reverb: vi(&params.chorus_send_to_reverb, setter),
        operators: [
            operator_panel(&params.operators[0], setter),
            operator_panel(&params.operators[1], setter),
            operator_panel(&params.operators[2], setter),
            operator_panel(&params.operators[3], setter),
        ],
    }
}

pub(crate) fn create_editor(
    egui_state: Arc<EguiState>,
    pending_gui_preset: Arc<Mutex<Option<Ym38x6Patch>>>,
    params: Arc<Ym38x6Params>,
) -> Option<Box<dyn Editor>> {
    let resize_state = egui_state.clone();
    create_egui_editor(
        egui_state,
        EditorState {
            preset_bank: PresetBank::load_from_dir(&presets_dir()),
            selected_key: None,
        },
        EguiSettings::default(),
        |_ctx, _queue, _state| {},
        move |ui, setter, _queue, state| {
            ResizableWindow::new("ym38x6_resize")
                .min_size((640.0, 480.0))
                .show(ui, &resize_state, |ui| {
                    // ---- プリセットブラウザ（左サイドバー・縦いっぱい） ----
                    egui::Panel::left("presets_panel")
                        .resizable(false)
                        .exact_size(180.0)
                        .show_inside(ui, |ui| {
                            ui.label(egui::RichText::new("PRESETS").strong());
                            egui::ScrollArea::vertical()
                                .id_salt("presets")
                                .show(ui, |ui| {
                                    for ((bank, program), preset) in state.preset_bank.sorted_entries() {
                                        let label = if bank == 0 {
                                            format!("{program:03} {}", preset.name)
                                        } else {
                                            format!("[{bank:04X}:{program:03}] {}", preset.name)
                                        };
                                        let selected = state.selected_key == Some((bank, program));
                                        if ui.selectable_label(selected, &label).clicked() {
                                            state.selected_key = Some((bank, program));
                                            *pending_gui_preset.lock().unwrap() = Some(preset.patch);
                                            macro_rules! set {
                                                ($p:expr, $v:expr) => {
                                                    setter.begin_set_parameter(&$p);
                                                    setter.set_parameter(&$p, $v);
                                                    setter.end_set_parameter(&$p);
                                                };
                                            }
                                            let ch = &preset.patch.channel;
                                            set!(params.algorithm, ch.algorithm as i32);
                                            set!(params.feedback, ch.feedback as i32);
                                            set!(params.tone_freq, ch.tone_lfo_freq as i32);
                                            set!(params.tone_pmd, ch.tone_lfo_pmd as i32);
                                            set!(params.tone_amd, ch.tone_lfo_amd as i32);
                                            set!(params.tone_delay, ch.tone_lfo_delay as i32);
                                            set!(params.pms, ch.pms as i32);
                                            set!(params.ams, ch.ams as i32);
                                            set!(params.cutoff, ch.filter_cutoff as i32);
                                            set!(params.resonance, ch.filter_resonance as i32);
                                            set!(params.feg_a, ch.filter_eg_attack as i32);
                                            set!(params.feg_d, ch.filter_eg_decay as i32);
                                            set!(params.feg_s, ch.filter_eg_sustain as i32);
                                            set!(params.feg_r, ch.filter_eg_release as i32);
                                            set!(params.feg_depth, ch.filter_eg_depth as i32);
                                            for (i, op) in preset.patch.operators.iter().enumerate() {
                                                let op_p = &params.operators[i];
                                                set!(op_p.tl, op.tl as i32);
                                                set!(op_p.ar, op.ar as i32);
                                                set!(op_p.d1r, op.d1r as i32);
                                                set!(op_p.d2r, op.d2r as i32);
                                                set!(op_p.d1l, op.d1l as i32);
                                                set!(op_p.rr, op.rr as i32);
                                                set!(op_p.mul, op.mul as i32);
                                                set!(op_p.dt1, op.dt1 as i32);
                                                set!(op_p.ksr, op.ksr as i32);
                                                set!(op_p.ame, op.am_enable);
                                                set!(op_p.vel_sens, op.velocity_sensitivity as i32);
                                                set!(op_p.op_fine_tune, op.op_fine_tune as i32);
                                                set!(op_p.waveform, op.waveform as i32);
                                            }
                                        }
                                    }
                                });
                        });

                    // ---- 残りのパラメーター（右側・縦スクロール、ym38x6-uiの共有レイアウト） ----
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        let panel = build_panel_params(&params, setter);
                        draw_param_panel(ui, &panel);
                    });
                });
        },
    )
}
