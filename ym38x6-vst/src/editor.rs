use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use std::sync::{Arc, Mutex};
use ym38x6_core::{presets_dir, PresetBank, Ym38x6Patch};
use ym38x6_ui::{draw_param_panel, BipolarFgPanelParams, FgEgPanelParams, OperatorPanelParams, PanelParams};

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
        floor: vi(&op.floor, setter),
        op_loop: vb(&op.op_loop, setter),
        curve: vb(&op.curve, setter),
        eg_shift: vi(&op.eg_shift, setter),
    }
}

fn build_panel_params<'a>(
    params: &'a Ym38x6Params,
    setter: &'a ParamSetter<'a>,
) -> PanelParams<'a> {
    PanelParams {
        algorithm: vi(&params.algorithm, setter),
        feedback: vi(&params.feedback, setter),
        texture_lfo_rate: vi(&params.texture_lfo_rate, setter),
        texture_lfo_depth: vi(&params.texture_lfo_depth, setter),
        texture_lfo_delay: vi(&params.texture_lfo_delay, setter),
        // 質感LFOの5波形パレット(0〜4)へ直接対応する（ym38x6-ui::LFO_WAVEFORM_NAMESも
        // 5波形に修正済み。旧8波形経由の変換は不要）。
        texture_lfo_waveform: vi(&params.texture_lfo_waveform, setter),
        texture_lfo_fade_mode: vi(&params.texture_lfo_fade_mode, setter),
        texture_lfo_fade_time: vi(&params.texture_lfo_fade_time, setter),
        texture_lfo_offset: vi(&params.texture_lfo_offset, setter),
        chip_lfo_freq: vi(&params.chip_lfo_freq, setter),
        chip_lfo_pmd: vi(&params.chip_lfo_pmd, setter),
        chip_lfo_amd: vi(&params.chip_lfo_amd, setter),
        chip_lfo_delay: vi(&params.chip_lfo_delay, setter),
        pms: vi(&params.pms, setter),
        ams: vi(&params.ams, setter),
        pitch_fg: BipolarFgPanelParams {
            eg: FgEgPanelParams {
                ar: vi(&params.pitch_fg_ar, setter),
                d1r: vi(&params.pitch_fg_d1r, setter),
                d1l: vi(&params.pitch_fg_d1l, setter),
                d2r: vi(&params.pitch_fg_d2r, setter),
                rr: vi(&params.pitch_fg_rr, setter),
                floor: vi(&params.pitch_fg_floor, setter),
                delay: vi(&params.pitch_fg_delay, setter),
                loop_enabled: vb(&params.pitch_fg_loop, setter),
                curve: vb(&params.pitch_fg_curve, setter),
            },
            depth: vi(&params.pitch_fg_depth, setter),
        },
        cutoff: vi(&params.cutoff, setter),
        resonance: vi(&params.resonance, setter),
        cutoff_fg: BipolarFgPanelParams {
            eg: FgEgPanelParams {
                ar: vi(&params.cutoff_fg_ar, setter),
                d1r: vi(&params.cutoff_fg_d1r, setter),
                d1l: vi(&params.cutoff_fg_d1l, setter),
                d2r: vi(&params.cutoff_fg_d2r, setter),
                rr: vi(&params.cutoff_fg_rr, setter),
                floor: vi(&params.cutoff_fg_floor, setter),
                delay: vi(&params.cutoff_fg_delay, setter),
                loop_enabled: vb(&params.cutoff_fg_loop, setter),
                curve: vb(&params.cutoff_fg_curve, setter),
            },
            depth: vi(&params.cutoff_fg_depth, setter),
        },
        gain_fg: FgEgPanelParams {
            ar: vi(&params.gain_fg_ar, setter),
            d1r: vi(&params.gain_fg_d1r, setter),
            d1l: vi(&params.gain_fg_d1l, setter),
            d2r: vi(&params.gain_fg_d2r, setter),
            rr: vi(&params.gain_fg_rr, setter),
            floor: vi(&params.gain_fg_floor, setter),
            delay: vi(&params.gain_fg_delay, setter),
            loop_enabled: vb(&params.gain_fg_loop, setter),
            curve: vb(&params.gain_fg_curve, setter),
        },
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
                                            // 質感LFO: 5波形パレット(0〜4)へ直接対応するため変換不要。
                                            set!(params.texture_lfo_waveform, ch.texture_lfo.waveform as i32);
                                            // Destination(NRPN(0,0)専用)はDAWパラメーターに存在しないため反映しない。
                                            set!(params.texture_lfo_rate, ch.texture_lfo.rate as i32);
                                            set!(params.texture_lfo_depth, ch.texture_lfo.depth as i32);
                                            set!(params.texture_lfo_delay, ch.texture_lfo.delay as i32);
                                            set!(params.texture_lfo_fade_mode, ch.texture_lfo.fade_mode as i32);
                                            set!(params.texture_lfo_fade_time, ch.texture_lfo.fade_time as i32);
                                            set!(params.texture_lfo_offset, ch.texture_lfo.offset as i32);
                                            set!(params.chip_lfo_freq, ch.chip_lfo_freq as i32);
                                            set!(params.chip_lfo_pmd, ch.chip_lfo_pmd as i32);
                                            set!(params.chip_lfo_amd, ch.chip_lfo_amd as i32);
                                            set!(params.chip_lfo_delay, ch.chip_lfo_delay as i32);
                                            set!(params.pms, ch.pms as i32);
                                            set!(params.ams, ch.ams as i32);
                                            set!(params.cutoff, ch.filter_cutoff as i32);
                                            set!(params.resonance, ch.filter_resonance as i32);
                                            // Pitch FG（②③層CC補正のCC状態自体はプリセット選択で変わらないため
                                            // ①パッチ由来の基準値のみDAWパラメーターへ反映する）。
                                            set!(params.pitch_fg_ar, ch.pitch_fg.eg.ar as i32);
                                            set!(params.pitch_fg_d1r, ch.pitch_fg.eg.d1r as i32);
                                            set!(params.pitch_fg_d1l, ch.pitch_fg.eg.d1l as i32);
                                            set!(params.pitch_fg_d2r, ch.pitch_fg.eg.d2r as i32);
                                            set!(params.pitch_fg_rr, ch.pitch_fg.eg.rr as i32);
                                            set!(params.pitch_fg_depth, ch.pitch_fg.depth as i32);
                                            set!(params.pitch_fg_floor, ch.pitch_fg.eg.floor as i32);
                                            set!(params.pitch_fg_delay, ch.pitch_fg.eg.delay as i32);
                                            set!(params.pitch_fg_loop, ch.pitch_fg.eg.loop_enabled != 0);
                                            set!(params.pitch_fg_curve, ch.pitch_fg.eg.curve != 0);
                                            set!(params.cutoff_fg_ar, ch.cutoff_fg.eg.ar as i32);
                                            set!(params.cutoff_fg_d1r, ch.cutoff_fg.eg.d1r as i32);
                                            set!(params.cutoff_fg_d1l, ch.cutoff_fg.eg.d1l as i32);
                                            set!(params.cutoff_fg_d2r, ch.cutoff_fg.eg.d2r as i32);
                                            set!(params.cutoff_fg_rr, ch.cutoff_fg.eg.rr as i32);
                                            set!(params.cutoff_fg_depth, ch.cutoff_fg.depth as i32);
                                            set!(params.cutoff_fg_floor, ch.cutoff_fg.eg.floor as i32);
                                            set!(params.cutoff_fg_delay, ch.cutoff_fg.eg.delay as i32);
                                            set!(params.cutoff_fg_loop, ch.cutoff_fg.eg.loop_enabled != 0);
                                            set!(params.cutoff_fg_curve, ch.cutoff_fg.eg.curve != 0);
                                            set!(params.gain_fg_ar, ch.gain_fg.ar as i32);
                                            set!(params.gain_fg_d1r, ch.gain_fg.d1r as i32);
                                            set!(params.gain_fg_d1l, ch.gain_fg.d1l as i32);
                                            set!(params.gain_fg_d2r, ch.gain_fg.d2r as i32);
                                            set!(params.gain_fg_rr, ch.gain_fg.rr as i32);
                                            set!(params.gain_fg_floor, ch.gain_fg.floor as i32);
                                            set!(params.gain_fg_delay, ch.gain_fg.delay as i32);
                                            set!(params.gain_fg_loop, ch.gain_fg.loop_enabled != 0);
                                            set!(params.gain_fg_curve, ch.gain_fg.curve != 0);
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
