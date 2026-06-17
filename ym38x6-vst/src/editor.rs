use nice_plug::prelude::*;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use std::sync::{Arc, Mutex};
use ym38x6_core::{presets_dir, PresetBank, Ym38x6Patch};

use crate::params::Ym38x6Params;

pub(crate) struct EditorState {
    pub(crate) preset_bank: PresetBank,
    pub(crate) selected_key: Option<(u16, u8)>,
}

/// GUIプリセットブラウザを生成する。
/// - DAW公開パラメーターはParamSetter経由で書き戻す
/// - filter_type/filter_self_oscillation/operator_waveformsはpending_gui_presetでprocess()側へ転送する
pub(crate) fn create_editor(
    egui_state: Arc<EguiState>,
    pending_gui_preset: Arc<Mutex<Option<Ym38x6Patch>>>,
    params: Arc<Ym38x6Params>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        egui_state,
        EditorState {
            preset_bank: PresetBank::load_from_dir(&presets_dir()),
            selected_key: None,
        },
        EguiSettings::default(),
        |_ctx, _queue, _state| {},
        move |ui, setter, _queue, state| {
            egui::ScrollArea::vertical().show(ui, |ui| {
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
                        }
                    }
                }
            });
        },
    )
}
