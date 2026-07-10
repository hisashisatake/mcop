use crate::algorithm_diagram::algorithm_diagram;
use crate::eg_preview::eg_preview;
use crate::knob::knob;
use crate::param_handle::{BoolParamHandle, IntParamHandle};
use crate::selector::{
    enum_selector, CHORUS_TYPE_NAMES, LFO_FADE_MODE_NAMES, LFO_WAVEFORM_NAMES, REVERB_TYPE_NAMES,
};
use crate::waveform::waveform_selector;

/// オペレーター単位パラメーター一式（VST/gesture-app共通のハンドル束）。
pub struct OperatorPanelParams<'a> {
    pub tl: Box<dyn IntParamHandle + 'a>,
    pub ar: Box<dyn IntParamHandle + 'a>,
    pub d1r: Box<dyn IntParamHandle + 'a>,
    pub d2r: Box<dyn IntParamHandle + 'a>,
    pub d1l: Box<dyn IntParamHandle + 'a>,
    pub rr: Box<dyn IntParamHandle + 'a>,
    pub mul: Box<dyn IntParamHandle + 'a>,
    pub dt1: Box<dyn IntParamHandle + 'a>,
    pub ksr: Box<dyn IntParamHandle + 'a>,
    pub vel_sens: Box<dyn IntParamHandle + 'a>,
    pub op_fine_tune: Box<dyn IntParamHandle + 'a>,
    pub ame: Box<dyn BoolParamHandle + 'a>,
    pub waveform: Box<dyn IntParamHandle + 'a>,
}

/// `draw_param_panel`に渡すパラメーター一式。
/// OPERATOR / CHANNEL / PERF LFO / TONE LFO / FILTER / MASTER EFFECTの全グリッドを含む。
pub struct PanelParams<'a> {
    // CHANNEL
    pub algorithm: Box<dyn IntParamHandle + 'a>,
    pub feedback: Box<dyn IntParamHandle + 'a>,
    // PERF LFO
    pub lfo_rate: Box<dyn IntParamHandle + 'a>,
    pub lfo_depth: Box<dyn IntParamHandle + 'a>,
    pub lfo_delay: Box<dyn IntParamHandle + 'a>,
    pub lfo_waveform: Box<dyn IntParamHandle + 'a>,
    pub lfo_fade_mode: Box<dyn IntParamHandle + 'a>,
    pub lfo_fade_time: Box<dyn IntParamHandle + 'a>,
    pub lfo_offset: Box<dyn IntParamHandle + 'a>,
    // TONE LFO
    pub tone_freq: Box<dyn IntParamHandle + 'a>,
    pub tone_pmd: Box<dyn IntParamHandle + 'a>,
    pub tone_amd: Box<dyn IntParamHandle + 'a>,
    pub tone_delay: Box<dyn IntParamHandle + 'a>,
    pub pms: Box<dyn IntParamHandle + 'a>,
    pub ams: Box<dyn IntParamHandle + 'a>,
    // FILTER
    pub cutoff: Box<dyn IntParamHandle + 'a>,
    pub resonance: Box<dyn IntParamHandle + 'a>,
    pub feg_ar: Box<dyn IntParamHandle + 'a>,
    pub feg_d1r: Box<dyn IntParamHandle + 'a>,
    pub feg_d1l: Box<dyn IntParamHandle + 'a>,
    pub feg_d2r: Box<dyn IntParamHandle + 'a>,
    pub feg_rr: Box<dyn IntParamHandle + 'a>,
    pub feg_depth: Box<dyn IntParamHandle + 'a>,
    // VCA (TVAオーバーレイ)
    pub vca_ar: Box<dyn IntParamHandle + 'a>,
    pub vca_d1r: Box<dyn IntParamHandle + 'a>,
    pub vca_d1l: Box<dyn IntParamHandle + 'a>,
    pub vca_d2r: Box<dyn IntParamHandle + 'a>,
    pub vca_rr: Box<dyn IntParamHandle + 'a>,
    // MASTER EFFECT
    pub rev_send: Box<dyn IntParamHandle + 'a>,
    pub reverb_type: Box<dyn IntParamHandle + 'a>,
    pub cho_send: Box<dyn IntParamHandle + 'a>,
    pub chorus_type: Box<dyn IntParamHandle + 'a>,
    pub reverb_time: Box<dyn IntParamHandle + 'a>,
    pub chorus_mod_rate: Box<dyn IntParamHandle + 'a>,
    pub chorus_mod_depth: Box<dyn IntParamHandle + 'a>,
    pub chorus_feedback: Box<dyn IntParamHandle + 'a>,
    pub chorus_send_to_reverb: Box<dyn IntParamHandle + 'a>,
    // OPERATORS
    pub operators: [OperatorPanelParams<'a>; 4],
}

/// パラメーターグリッド（OP1〜4 / CHANNEL・PERF LFO・TONE LFO / FILTER・MASTER EFFECT）を描画する。
/// 縦スクロールエリアを内部に含む。PRESETSサイドバー・ウィンドウ枠（ResizableWindow等）・
/// 外側のCentralPanelは呼び出し側（ホスト）が用意すること。
pub fn draw_param_panel(ui: &mut egui::Ui, params: &PanelParams) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // ---- Operators（各opを横一列で表示し、OP1→OP4を縦に積む。最優先で上に表示） ----
        for (i, op) in params.operators.iter().enumerate() {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(format!("OP {}", i + 1)).strong());
                    ui.horizontal(|ui| {
                        eg_preview(
                            ui,
                            op.tl.value() as u8,
                            op.ar.value() as u8,
                            op.d1r.value() as u8,
                            op.d1l.value() as u8,
                            op.d2r.value() as u8,
                            op.rr.value() as u8,
                        );
                        ui.horizontal_wrapped(|ui| {
                            knob(ui, &*op.tl, "TL");
                            knob(ui, &*op.ar, "AR");
                            knob(ui, &*op.d1r, "D1R");
                            knob(ui, &*op.d2r, "D2R");
                            knob(ui, &*op.d1l, "D1L");
                            knob(ui, &*op.rr, "RR");
                            knob(ui, &*op.mul, "MUL");
                            knob(ui, &*op.dt1, "DT1");
                            knob(ui, &*op.ksr, "KSR");
                            knob(ui, &*op.vel_sens, "VEL");
                            knob(ui, &*op.op_fine_tune, "FINE");
                            let mut am = op.ame.value();
                            if ui.checkbox(&mut am, "AM").changed() {
                                op.ame.begin_edit();
                                op.ame.set(am);
                                op.ame.end_edit();
                            }
                            waveform_selector(ui, &*op.waveform, i);
                        });
                    });
                });
            });
        }

        // ---- チャンネル固有 / パフォーマンスLFO / トーンLFO（横一列） ----
        ui.horizontal(|ui| {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("CHANNEL").strong());
                    ui.horizontal(|ui| {
                        algorithm_diagram(ui, params.algorithm.value() as u8);
                        ui.horizontal_wrapped(|ui| {
                            knob(ui, &*params.algorithm, "ALG");
                            knob(ui, &*params.feedback, "FB");
                        });
                    });
                });
            });

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("PERF LFO").strong());
                    ui.horizontal_wrapped(|ui| {
                        knob(ui, &*params.lfo_rate, "L.RATE");
                        knob(ui, &*params.lfo_depth, "L.DEP");
                        knob(ui, &*params.lfo_delay, "L.DLY");
                        enum_selector(ui, &*params.lfo_waveform, "L.WAVE", &LFO_WAVEFORM_NAMES, 2);
                        enum_selector(ui, &*params.lfo_fade_mode, "L.FADE", &LFO_FADE_MODE_NAMES, 3);
                        knob(ui, &*params.lfo_fade_time, "L.F.TM");
                        knob(ui, &*params.lfo_offset, "L.OFS");
                    });
                });
            });

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("TONE LFO").strong());
                    ui.horizontal_wrapped(|ui| {
                        knob(ui, &*params.tone_freq, "T.FRQ");
                        knob(ui, &*params.tone_pmd, "T.PMD");
                        knob(ui, &*params.tone_amd, "T.AMD");
                        knob(ui, &*params.tone_delay, "T.DLY");
                        knob(ui, &*params.pms, "PMS");
                        knob(ui, &*params.ams, "AMS");
                    });
                });
            });
        });

        // ---- フィルター ----
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("FILTER").strong());
                ui.horizontal(|ui| {
                    eg_preview(
                        ui,
                        255,
                        params.feg_ar.value() as u8,
                        params.feg_d1r.value() as u8,
                        params.feg_d1l.value() as u8,
                        params.feg_d2r.value() as u8,
                        params.feg_rr.value() as u8,
                    );
                    ui.horizontal_wrapped(|ui| {
                        knob(ui, &*params.cutoff, "CUT");
                        knob(ui, &*params.resonance, "RES");
                        knob(ui, &*params.feg_ar, "F.AR");
                        knob(ui, &*params.feg_d1r, "F.D1R");
                        knob(ui, &*params.feg_d1l, "F.D1L");
                        knob(ui, &*params.feg_d2r, "F.D2R");
                        knob(ui, &*params.feg_rr, "F.RR");
                        knob(ui, &*params.feg_depth, "F.DEP");
                    });
                });
            });
        });

        // ---- VCA（TVAオーバーレイ） ----
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("VCA").strong());
                ui.horizontal(|ui| {
                    eg_preview(
                        ui,
                        255,
                        params.vca_ar.value() as u8,
                        params.vca_d1r.value() as u8,
                        params.vca_d1l.value() as u8,
                        params.vca_d2r.value() as u8,
                        params.vca_rr.value() as u8,
                    );
                    ui.horizontal_wrapped(|ui| {
                        knob(ui, &*params.vca_ar, "V.AR");
                        knob(ui, &*params.vca_d1r, "V.D1R");
                        knob(ui, &*params.vca_d1l, "V.D1L");
                        knob(ui, &*params.vca_d2r, "V.D2R");
                        knob(ui, &*params.vca_rr, "V.RR");
                    });
                });
            });
        });

        // ---- マスターエフェクト ----
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("MASTER EFFECT").strong());
                ui.horizontal_wrapped(|ui| {
                    enum_selector(ui, &*params.reverb_type, "REV TYPE", &REVERB_TYPE_NAMES, 0);
                    knob(ui, &*params.rev_send, "REV");
                    knob(ui, &*params.reverb_time, "R.TIME");
                    enum_selector(ui, &*params.chorus_type, "CHO TYPE", &CHORUS_TYPE_NAMES, 1);
                    knob(ui, &*params.cho_send, "CHO");
                    knob(ui, &*params.chorus_mod_rate, "C.RATE");
                    knob(ui, &*params.chorus_mod_depth, "C.DEP");
                    knob(ui, &*params.chorus_feedback, "C.FB");
                    knob(ui, &*params.chorus_send_to_reverb, "C>R");
                });
            });
        });
    });
}
