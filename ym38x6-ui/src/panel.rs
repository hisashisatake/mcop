use crate::algorithm_diagram::algorithm_diagram;
use crate::eg_preview::{eg_preview, EgAmplitudeMapping};
use crate::knob::{bool_checkbox, knob};
use crate::param_handle::{BoolParamHandle, IntParamHandle};
use crate::selector::{
    enum_selector, CHORUS_TYPE_NAMES, LFO_FADE_MODE_NAMES, LFO_WAVEFORM_NAMES, REVERB_TYPE_NAMES,
};
use crate::waveform::waveform_selector;
use sound_core::eg::EgParams;

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
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。`op_loop`がOFFの間は無効。
    pub floor: Box<dyn IntParamHandle + 'a>,
    /// OP単位ループEG（VCF/VCAのFGと同じLoop/Floor/Curve機構をFMオペレーターEGに開放したもの）。
    pub op_loop: Box<dyn BoolParamHandle + 'a>,
    /// 0=線形（角の立つ三角）/1=サイン風（レイズドコサインで角を丸める）。
    pub curve: Box<dyn BoolParamHandle + 'a>,
    /// EGSFT（TX81Z EG Shift）。EGの減衰レンジ(dB)を圧縮する（0〜255、既定0＝96dBフルレンジ）。
    pub eg_shift: Box<dyn IntParamHandle + 'a>,
    /// Level Scaling（ノート依存の出力レベル減衰、OPL系KSL相当）。0〜255、既定0＝スケーリングなし。
    pub level_scale: Box<dyn IntParamHandle + 'a>,
}

/// ファンクションジェネレーター（Pitch/Cutoff/Gain FG共通）のループ可能EGパラメーター一式。
/// `sound_core::eg::EgParams`と1:1対応する（新規の別部品を作らない設計、spec-sound.md参照）。
pub struct FgEgPanelParams<'a> {
    pub ar: Box<dyn IntParamHandle + 'a>,
    pub d1r: Box<dyn IntParamHandle + 'a>,
    pub d1l: Box<dyn IntParamHandle + 'a>,
    pub d2r: Box<dyn IntParamHandle + 'a>,
    pub rr: Box<dyn IntParamHandle + 'a>,
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。
    pub floor: Box<dyn IntParamHandle + 'a>,
    /// キーオンからAR開始までの遅延(0〜255、既定0＝遅延なし)。
    pub delay: Box<dyn IntParamHandle + 'a>,
    /// 0=ワンショット／1=ループ。
    pub loop_enabled: Box<dyn BoolParamHandle + 'a>,
    /// 0=線形（角の立つ三角）／1=サイン風（レイズドコサインで角を丸める）。
    pub curve: Box<dyn BoolParamHandle + 'a>,
}

impl FgEgPanelParams<'_> {
    fn to_eg_params(&self) -> EgParams {
        EgParams {
            ar: self.ar.value() as u8,
            d1r: self.d1r.value() as u8,
            d1l: self.d1l.value() as u8,
            d2r: self.d2r.value() as u8,
            rr: self.rr.value() as u8,
            floor: self.floor.value() as u8,
            loop_enabled: self.loop_enabled.value() as u8,
            curve: self.curve.value() as u8,
            delay: self.delay.value() as u8,
        }
    }
}

/// Pitch FG（新規）／Cutoff FG（旧Filter EG）に共通の「共通EG＋バイポーラDepth」一式。
pub struct BipolarFgPanelParams<'a> {
    pub eg: FgEgPanelParams<'a>,
    /// バイポーラDepth（0〜255、中心128＝変調なし、128超＝＋方向、128未満＝−方向）。
    pub depth: Box<dyn IntParamHandle + 'a>,
}

/// `draw_param_panel`に渡すパラメーター一式。
/// OPERATOR / CHANNEL / TEXTURE LFO / CHIP LFO / PITCH FG / CUTOFF FG / GAIN FG / MASTER EFFECT
/// の全グリッドを含む。
pub struct PanelParams<'a> {
    // CHANNEL
    pub algorithm: Box<dyn IntParamHandle + 'a>,
    pub feedback: Box<dyn IntParamHandle + 'a>,
    // TEXTURE LFO（旧PERF LFO。5波形専用・焼き込み専用）
    pub texture_lfo_rate: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_depth: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_delay: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_waveform: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_fade_mode: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_fade_time: Box<dyn IntParamHandle + 'a>,
    pub texture_lfo_offset: Box<dyn IntParamHandle + 'a>,
    // CHIP LFO（旧TONE LFO。チップ内LFO、VCO差し替えで消えるレイヤー）
    pub chip_lfo_freq: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_pmd: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_amd: Box<dyn IntParamHandle + 'a>,
    pub chip_lfo_delay: Box<dyn IntParamHandle + 'a>,
    pub pms: Box<dyn IntParamHandle + 'a>,
    pub ams: Box<dyn IntParamHandle + 'a>,
    // PITCH FG（新規）
    pub pitch_fg: BipolarFgPanelParams<'a>,
    // FILTER（CUTOFF FG、旧Filter EG）
    pub cutoff: Box<dyn IntParamHandle + 'a>,
    pub resonance: Box<dyn IntParamHandle + 'a>,
    pub cutoff_fg: BipolarFgPanelParams<'a>,
    // VCA（GAIN FG、旧VCA EG。音量に負値は無いためDepthを持たずFloorが深さ役）
    pub gain_fg: FgEgPanelParams<'a>,
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

/// FG共通EG（AR/D1R/D1L/D2R/RR/FLOOR/DLY + LOOP/CURVEチェック）をノブ列として描く。
/// `depth`が`Some`ならバイポーラDepthノブ（`label`+"DEP"、中心128±方向）も並べる
/// （Pitch/Cutoff FG向け。Gain FGはDepthを持たないため`None`で呼ぶ）。
fn fg_eg_knobs(ui: &mut egui::Ui, prefix: &str, eg: &FgEgPanelParams, depth: Option<&dyn IntParamHandle>) {
    knob(ui, &*eg.ar, &format!("{prefix}AR"));
    knob(ui, &*eg.d1r, &format!("{prefix}D1R"));
    knob(ui, &*eg.d1l, &format!("{prefix}D1L"));
    knob(ui, &*eg.d2r, &format!("{prefix}D2R"));
    knob(ui, &*eg.rr, &format!("{prefix}RR"));
    if let Some(depth) = depth {
        knob(ui, depth, &format!("{prefix}DEP\u{00b1}"));
    }
    knob(ui, &*eg.floor, &format!("{prefix}FLOOR"));
    knob(ui, &*eg.delay, &format!("{prefix}DLY"));
    bool_checkbox(ui, &*eg.loop_enabled, "LOOP");
    bool_checkbox(ui, &*eg.curve, "CURVE");
}

/// パラメーターグリッド（OP1〜4 / CHANNEL・TEXTURE LFO・CHIP LFO / PITCH FG / CUTOFF FG・
/// GAIN FG / MASTER EFFECT）を描画する。縦スクロールエリアを内部に含む。
/// PRESETSサイドバー・ウィンドウ枠（ResizableWindow等）・外側のCentralPanelは呼び出し側
/// （ホスト）が用意すること。
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
                            EgAmplitudeMapping::DbLinear,
                            op.tl.value() as u8,
                            EgParams {
                                ar: op.ar.value() as u8,
                                d1r: op.d1r.value() as u8,
                                d1l: op.d1l.value() as u8,
                                d2r: op.d2r.value() as u8,
                                rr: op.rr.value() as u8,
                                floor: op.floor.value() as u8,
                                loop_enabled: op.op_loop.value() as u8,
                                curve: op.curve.value() as u8,
                                delay: 0,
                            },
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
                            bool_checkbox(ui, &*op.ame, "AM");
                            waveform_selector(ui, &*op.waveform, i);
                            knob(ui, &*op.floor, "FLOOR");
                            bool_checkbox(ui, &*op.op_loop, "LOOP");
                            bool_checkbox(ui, &*op.curve, "CURVE");
                            knob(ui, &*op.eg_shift, "EGSFT");
                            knob(ui, &*op.level_scale, "LEVEL SCALE");
                        });
                    });
                });
            });
        }

        // ---- チャンネル固有 / 質感LFO / チップ内LFO（横一列） ----
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
                    ui.label(egui::RichText::new("TEXTURE LFO").strong());
                    ui.horizontal_wrapped(|ui| {
                        knob(ui, &*params.texture_lfo_rate, "TX.RATE");
                        knob(ui, &*params.texture_lfo_depth, "TX.DEP");
                        knob(ui, &*params.texture_lfo_delay, "TX.DLY");
                        enum_selector(ui, &*params.texture_lfo_waveform, "TX.WAVE", &LFO_WAVEFORM_NAMES, 2);
                        enum_selector(ui, &*params.texture_lfo_fade_mode, "TX.FADE", &LFO_FADE_MODE_NAMES, 3);
                        knob(ui, &*params.texture_lfo_fade_time, "TX.F.TM");
                        knob(ui, &*params.texture_lfo_offset, "TX.OFS");
                    });
                });
            });

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("CHIP LFO").strong());
                    ui.horizontal_wrapped(|ui| {
                        knob(ui, &*params.chip_lfo_freq, "CH.FRQ");
                        knob(ui, &*params.chip_lfo_pmd, "CH.PMD");
                        knob(ui, &*params.chip_lfo_amd, "CH.AMD");
                        knob(ui, &*params.chip_lfo_delay, "CH.DLY");
                        knob(ui, &*params.pms, "PMS");
                        knob(ui, &*params.ams, "AMS");
                    });
                });
            });
        });

        // ---- PITCH FG（新規） ----
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("PITCH FG").strong());
                ui.horizontal(|ui| {
                    eg_preview(ui, EgAmplitudeMapping::AmplitudeLinear, 255, params.pitch_fg.eg.to_eg_params());
                    ui.horizontal_wrapped(|ui| {
                        fg_eg_knobs(ui, "P.", &params.pitch_fg.eg, Some(&*params.pitch_fg.depth));
                    });
                });
            });
        });

        // ---- CUTOFF FG（旧Filter EG） ----
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("CUTOFF FG").strong());
                ui.horizontal(|ui| {
                    eg_preview(ui, EgAmplitudeMapping::AmplitudeLinear, 255, params.cutoff_fg.eg.to_eg_params());
                    ui.horizontal_wrapped(|ui| {
                        knob(ui, &*params.cutoff, "CUT");
                        knob(ui, &*params.resonance, "RES");
                        fg_eg_knobs(ui, "F.", &params.cutoff_fg.eg, Some(&*params.cutoff_fg.depth));
                    });
                });
            });
        });

        // ---- GAIN FG（旧VCA EG） ----
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("GAIN FG").strong());
                ui.horizontal(|ui| {
                    eg_preview(ui, EgAmplitudeMapping::AmplitudeLinear, 255, params.gain_fg.to_eg_params());
                    ui.horizontal_wrapped(|ui| {
                        fg_eg_knobs(ui, "V.", &params.gain_fg, None);
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
