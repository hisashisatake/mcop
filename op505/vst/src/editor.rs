use nice_plug::prelude::*;
use nice_plug_egui::resizable_window::ResizableWindow;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use op505_core::{op505_presets_dir, Op505PresetBank};
use op505_ui::{draw_op505_panel, Op505BipolarFgPanelParams, Op505OperatorPanelParams, Op505PanelParams};
use std::sync::{Arc, RwLock};

use crate::param_adapter::{vb, vi, vt};
use crate::params::{Op505EgBank, Op505VstParams, OperatorVstParams};

pub(crate) struct EditorState {
    pub(crate) preset_bank: Op505PresetBank,
    pub(crate) selected_key: Option<(u16, u8)>,
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
        chip_lfo_freq: vi(&params.chip_lfo_freq, setter),
        chip_lfo_pmd: vi(&params.chip_lfo_pmd, setter),
        chip_lfo_amd: vi(&params.chip_lfo_amd, setter),
        chip_lfo_delay: vi(&params.chip_lfo_delay, setter),
        pms: vi(&params.pms, setter),
        ams: vi(&params.ams, setter),
        texture_lfo_rate: vi(&params.texture_lfo_rate, setter),
        texture_lfo_depth: vi(&params.texture_lfo_depth, setter),
        texture_lfo_delay: vi(&params.texture_lfo_delay, setter),
        texture_lfo_waveform: vi(&params.texture_lfo_waveform, setter),
        texture_lfo_fade_mode: vi(&params.texture_lfo_fade_mode, setter),
        texture_lfo_fade_time: vi(&params.texture_lfo_fade_time, setter),
        texture_lfo_offset: vi(&params.texture_lfo_offset, setter),
        texture_lfo_destination: vi(&params.texture_lfo_destination, setter),
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
        gain_fg: vt(egs, |b: &Op505EgBank| b.gain_fg, |b: &mut Op505EgBank, v| b.gain_fg = v, "GAIN FG"),
        operators: [
            operator_panel::<0>(&params.operators[0], egs, setter),
            operator_panel::<1>(&params.operators[1], egs, setter),
            operator_panel::<2>(&params.operators[2], egs, setter),
            operator_panel::<3>(&params.operators[3], egs, setter),
        ],
    }
}

pub(crate) fn create_editor(
    egui_state: Arc<EguiState>,
    params: Arc<Op505VstParams>,
) -> Option<Box<dyn Editor>> {
    let resize_state = egui_state.clone();
    create_egui_editor(
        egui_state,
        EditorState {
            preset_bank: Op505PresetBank::load_from_dir(&op505_presets_dir()),
            selected_key: None,
        },
        EguiSettings::default(),
        |_ctx, _queue, _state| {},
        move |ui, setter, _queue, state| {
            ResizableWindow::new("op505_resize")
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
                                        let label = format!("[{bank:04X}:{program:03}] {}", preset.name);
                                        let selected = state.selected_key == Some((bank, program));
                                        if ui.selectable_label(selected, &label).clicked() {
                                            state.selected_key = Some((bank, program));
                                            let patch = preset.patch;
                                            macro_rules! set {
                                                ($p:expr, $v:expr) => {
                                                    setter.begin_set_parameter(&$p);
                                                    setter.set_parameter(&$p, $v);
                                                    setter.end_set_parameter(&$p);
                                                };
                                            }
                                            let ch = &patch.channel;
                                            set!(params.algorithm, ch.algorithm as i32);
                                            set!(params.feedback, ch.feedback as i32);
                                            set!(params.chip_lfo_freq, ch.chip_lfo_freq as i32);
                                            set!(params.chip_lfo_pmd, ch.chip_lfo_pmd as i32);
                                            set!(params.chip_lfo_amd, ch.chip_lfo_amd as i32);
                                            set!(params.chip_lfo_delay, ch.chip_lfo_delay as i32);
                                            set!(params.pms, ch.pms as i32);
                                            set!(params.ams, ch.ams as i32);
                                            set!(params.cutoff, ch.filter_cutoff as i32);
                                            set!(params.resonance, ch.filter_resonance as i32);
                                            set!(params.filter_type, ch.filter_type as i32);
                                            set!(params.filter_self_oscillation, ch.filter_self_oscillation);
                                            set!(params.pitch_fg_depth, ch.pitch_fg.depth as i32);
                                            set!(params.cutoff_fg_depth, ch.cutoff_fg.depth as i32);
                                            set!(params.texture_lfo_rate, ch.texture_lfo.rate as i32);
                                            set!(params.texture_lfo_depth, ch.texture_lfo.depth as i32);
                                            set!(params.texture_lfo_delay, ch.texture_lfo.delay as i32);
                                            set!(params.texture_lfo_waveform, ch.texture_lfo.waveform as i32);
                                            set!(params.texture_lfo_fade_mode, ch.texture_lfo.fade_mode as i32);
                                            set!(params.texture_lfo_fade_time, ch.texture_lfo.fade_time as i32);
                                            set!(params.texture_lfo_offset, ch.texture_lfo.offset as i32);
                                            set!(params.texture_lfo_destination, ch.texture_lfo.destination as i32);
                                            for (i, op) in patch.operators.iter().enumerate() {
                                                let op_p = &params.operators[i];
                                                set!(op_p.tl, op.tl as i32);
                                                set!(op_p.mul, op.mul as i32);
                                                set!(op_p.dt1, op.dt1 as i32);
                                                set!(op_p.ksr, op.ksr as i32);
                                                set!(op_p.ame, op.am_enable);
                                                set!(op_p.vel_sens, op.velocity_sensitivity as i32);
                                                set!(op_p.op_fine_tune, op.op_fine_tune as i32);
                                                set!(op_p.waveform, op.waveform as i32);
                                                set!(op_p.eg_shift, op.eg_shift as i32);
                                                set!(op_p.level_scale, op.level_scale as i32);
                                                set!(op_p.velocity_gain, op.velocity_gain as i32);
                                            }
                                            // TimeEg 7本（persist状態）はDAWパラメーターではないため
                                            // ParamSetterを経由せず直接書き込む。
                                            let mut egs = params.egs.write().expect("Poisoned RwLock on write");
                                            for (i, op) in patch.operators.iter().enumerate() {
                                                egs.operators[i] = op.eg;
                                            }
                                            egs.pitch_fg = ch.pitch_fg.eg;
                                            egs.cutoff_fg = ch.cutoff_fg.eg;
                                            egs.gain_fg = ch.gain_fg;
                                        }
                                    }
                                });
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
