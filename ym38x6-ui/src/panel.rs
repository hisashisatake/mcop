use crate::algorithm_diagram::algorithm_diagram;
use crate::eg_preview::{eg_preview, EgAmplitudeMapping};
use crate::knob::{bool_checkbox, knob};
use crate::param_handle::{BoolParamHandle, IntParamHandle};
use crate::selector::{
    enum_selector, CHORUS_TYPE_NAMES, LFO_FADE_MODE_NAMES, LFO_WAVEFORM_NAMES, REVERB_TYPE_NAMES,
};
use crate::waveform::waveform_selector;
use sound_core::eg::EgParams;

/// MUL値(0〜15)→周波数比。`ym38x6_core::mapping::mul_to_ratio`と同じ表（0=0.5倍、1〜15=等倍）だが、
/// `ym38x6-ui`はnice-plug/Tauri非依存の`sound-core`のみに依存する方針のため、
/// OPヘッダの実効比率表示専用にこの極小テーブルをインライン複製する（内部エンジンは無改変）。
fn mul_to_ratio(mul: u8) -> f32 {
    const TABLE: [f32; 16] = [
        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    TABLE[(mul as usize).min(15)]
}

/// op_fine_tune値(0〜255、中心128)→セント。`ym38x6_core::mapping::op_fine_tune_to_cents`と同じ式
/// （中心128で±0、両端±1200セント）。上記`mul_to_ratio`と同じ理由でインライン複製する。
fn op_fine_tune_to_cents(v: u8) -> f32 {
    const OP_FINE_TUNE_RANGE_CENTS: f32 = 1200.0;
    (v as f32 - 128.0) / 128.0 * OP_FINE_TUNE_RANGE_CENTS
}

/// MUL＋FINE（op_fine_tune）による実効周波数比（DT1は除く）。OPヘッダの読み取り表示専用。
fn mul_fine_ratio(mul: u8, op_fine_tune: u8) -> f32 {
    mul_to_ratio(mul) * 2f32.powf(op_fine_tune_to_cents(op_fine_tune) / 1200.0)
}

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
    /// キャリア出力へのベロシティ音量ゲイン深さ（0〜255、既定255＝フル）。
    /// `vel_sens`（明るさ、モジュレーター専用）とは独立・別軸。役割はALGで決まるため、
    /// パネルはそのOPがキャリアかモジュレーターかでVEL/V.GAINを排他的にグレーアウトする。
    pub velocity_gain: Box<dyn IntParamHandle + 'a>,
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
    /// 質感LFOの行き先（0=Pitch/1=Volume/2=TL/3=Cutoff）。パネルではモジュラー風の
    /// パッチケーブルUI（`texture_lfo_patchbay`）で選ぶ。内部は単一enumのまま。
    pub texture_lfo_destination: Box<dyn IntParamHandle + 'a>,
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
    // LOOP/CURVEは縦2段に積んで1スロット分の幅として扱う。
    ui.vertical(|ui| {
        bool_checkbox(ui, &*eg.loop_enabled, "LOOP");
        bool_checkbox(ui, &*eg.curve, "CURVE");
    });
}

/// ノブ等の固定セル幅（各ウィジェットのallocate呼び出しと一致させる、下記`justified_row`用）。
const KNOB_W: f32 = 62.0;
/// bool_checkboxは可変幅（チェックボックス＋ラベル文字幅）。実測値（"CURVE"ラベルの実際の
/// 描画幅、ym38x6-ui/examples/op_slice.rsで目視検証済み）。50では収まらず折り返されていた。
const CHECKBOX_W: f32 = 70.0;
const WAVEFORM_SELECTOR_W: f32 = 130.0;
const ENUM_SELECTOR_W: f32 = 100.0;

/// 子要素の「自然幅」の合計とコンテナの利用可能幅との差を要素間の隙間へ均等配分し、
/// 横一列に敷き詰める（CSSの`justify-content: space-between`相当）。要素自体のサイズは
/// 変えず、余白だけを伸縮させてパネル幅いっぱいに均等割り付けする。
/// `natural_widths`は`add_contents`内で追加する順序・個数と一致させること
/// （knob=`KNOB_W`／bool_checkbox=`CHECKBOX_W`／waveform_selector=`WAVEFORM_SELECTOR_W`／
/// enum_selector=`ENUM_SELECTOR_W`）。
fn justified_row<R>(
    ui: &mut egui::Ui,
    natural_widths: &[f32],
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let available = ui.available_width();
    let total: f32 = natural_widths.iter().sum();
    let n = natural_widths.len();
    let gap = if n > 1 {
        ((available - total) / (n as f32 - 1.0)).max(0.0)
    } else {
        0.0
    };
    let prev_spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = gap;
    let result = ui.horizontal(|ui| add_contents(ui)).inner;
    ui.spacing_mut().item_spacing.x = prev_spacing;
    result
}

/// パラメーターグリッド（OP1〜4 / CHANNEL・TEXTURE LFO・CHIP LFO / PITCH FG / CUTOFF FG・
/// GAIN FG / MASTER EFFECT）を描画する。縦スクロールエリアを内部に含む。
/// PRESETSサイドバー・ウィンドウ枠（ResizableWindow等）・外側のCentralPanelは呼び出し側
/// （ホスト）が用意すること。
pub fn draw_param_panel(ui: &mut egui::Ui, params: &PanelParams) {
    // 質感LFOパッチベイの1フレーム分のジャック配置（出力＋4行き先）。各パネルの描画時に
    // 位置を埋め、末尾の`finish_texture_lfo_patchbay`でケーブルを描く（patchbay.rs参照）。
    let mut tx_jacks = crate::patchbay::JackLayout::new();

    egui::ScrollArea::vertical().show(ui, |ui| {
        // 以降の全パネルが揃える基準幅（OPの幅＝スクロール内容の全幅）。
        let full_width = ui.available_width();
        // CHANNEL-CHIP LFO間の余白と同じ量を、各全幅パネルの右端にも確保する。
        let inter_gap = ui.spacing().item_spacing.x;
        let panel_width = full_width - inter_gap;

        // ---- Operators（各opを横一列で表示し、OP1→OP4を縦に積む。最優先で上に表示） ----
        for (i, op) in params.operators.iter().enumerate() {
            ui.group(|ui| {
                ui.set_width(panel_width);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("OP {}", i + 1)).strong());
                        let ratio = mul_fine_ratio(op.mul.value() as u8, op.op_fine_tune.value() as u8);
                        ui.label(
                            egui::RichText::new(format!("\u{d7}{ratio:.2}"))
                                .size(10.0)
                                .weak(),
                        )
                        .on_hover_text("MUL×FINEの実効周波数比（DT1は含まない）");
                    });
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
                        // このOPがキャリアかどうかでVEL(明るさ・モジュレーター専用)/
                        // V.GAIN(音量・キャリア専用)を排他的にグレーアウトする。
                        // ALGノブを変えるとその場で切り替わる（値自体は不変）。
                        let is_carrier = crate::algorithm_diagram::carriers(
                            params.algorithm.value() as u8,
                        )
                        .contains(&i);
                        // AM/LOOP/CURVEは縦3段に積んで1スロット分の幅として扱う。
                        let widths = [
                            KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W,
                            KNOB_W, KNOB_W, KNOB_W, WAVEFORM_SELECTOR_W, KNOB_W, CHECKBOX_W,
                            KNOB_W, KNOB_W,
                        ];
                        justified_row(ui, &widths, |ui| {
                            knob(ui, &*op.tl, "TL");
                            knob(ui, &*op.ar, "AR");
                            knob(ui, &*op.d1r, "D1R");
                            knob(ui, &*op.d2r, "D2R");
                            knob(ui, &*op.d1l, "D1L");
                            knob(ui, &*op.rr, "RR");
                            knob(ui, &*op.mul, "MUL");
                            knob(ui, &*op.dt1, "DT1");
                            knob(ui, &*op.ksr, "KSR");
                            ui.add_enabled_ui(!is_carrier, |ui| {
                                knob(ui, &*op.vel_sens, "VEL");
                            });
                            ui.add_enabled_ui(is_carrier, |ui| {
                                knob(ui, &*op.velocity_gain, "V.GAIN");
                            });
                            knob(ui, &*op.op_fine_tune, "FINE");
                            waveform_selector(ui, &*op.waveform, i);
                            knob(ui, &*op.floor, "FLOOR");
                            ui.vertical(|ui| {
                                bool_checkbox(ui, &*op.ame, "AM");
                                bool_checkbox(ui, &*op.op_loop, "LOOP");
                                bool_checkbox(ui, &*op.curve, "CURVE");
                            });
                            knob(ui, &*op.eg_shift, "EGSFT");
                            knob(ui, &*op.level_scale, "LEVEL SCALE");
                        });
                    });
                });
            });
        }

        // ---- チャンネル固有 / チップ内LFO（横一列の2列。CHANNEL=1/3・CHIP LFO=2/3幅、
        //      右端にもCHANNEL-CHIP LFO間と同じ余白を残す） ----
        let two_col_usable = full_width - inter_gap * 2.0;
        let channel_width = two_col_usable / 3.0;
        let chip_lfo_width = two_col_usable * 2.0 / 3.0;
        ui.horizontal(|ui| {
            let channel_resp = ui.group(|ui| {
                ui.set_width(channel_width);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("CHANNEL").strong());
                    ui.horizontal(|ui| {
                        algorithm_diagram(ui, params.algorithm.value() as u8);
                        justified_row(ui, &[KNOB_W, KNOB_W], |ui| {
                            knob(ui, &*params.algorithm, "ALG");
                            knob(ui, &*params.feedback, "FB");
                        });
                    });
                    ui.add_space(2.0);
                    crate::patchbay::texture_lfo_dest_jack(
                        ui,
                        &*params.texture_lfo_destination,
                        2,
                        "TX LFO (TL)",
                        &mut tx_jacks,
                    );
                });
            });
            // CHIP LFOの縦幅はCHANNELの実測高さに合わせる（内容量が少なく本来はより低いため）。
            let channel_height = channel_resp.response.rect.height();

            ui.group(|ui| {
                ui.set_width(chip_lfo_width);
                ui.set_min_height(channel_height);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("CHIP LFO").strong());
                    justified_row(ui, &[KNOB_W; 6], |ui| {
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

        // ---- 質感LFO（単独で全幅の1行） ----
        ui.group(|ui| {
            ui.set_width(panel_width);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("TEXTURE LFO").strong());
                justified_row(
                    ui,
                    &[KNOB_W, KNOB_W, KNOB_W, ENUM_SELECTOR_W, ENUM_SELECTOR_W, KNOB_W, KNOB_W],
                    |ui| {
                        knob(ui, &*params.texture_lfo_rate, "TX.RATE");
                        knob(ui, &*params.texture_lfo_depth, "TX.DEP");
                        knob(ui, &*params.texture_lfo_delay, "TX.DLY");
                        enum_selector(ui, &*params.texture_lfo_waveform, "TX.WAVE", &LFO_WAVEFORM_NAMES, 2);
                        enum_selector(ui, &*params.texture_lfo_fade_mode, "TX.FADE", &LFO_FADE_MODE_NAMES, 3);
                        knob(ui, &*params.texture_lfo_fade_time, "TX.F.TM");
                        knob(ui, &*params.texture_lfo_offset, "TX.OFS");
                    },
                );
                // 出力ジャック。行き先（Pitch/Cutoff/Gain各FGパネル・CHANNELパネルのTL）は
                // それぞれのパネルヘッダーにジャックとして配置し、太いパッチケーブルで結ぶ
                // （見た目のみのモジュラーメタファー。内部は従来どおり単一enumを1つ選ぶだけ）。
                ui.add_space(2.0);
                crate::patchbay::texture_lfo_source_jack(ui, &mut tx_jacks);
            });
        });

        // ---- PITCH FG（新規） ----
        ui.group(|ui| {
            ui.set_width(panel_width);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("PITCH FG").strong());
                    crate::patchbay::texture_lfo_dest_jack(
                        ui,
                        &*params.texture_lfo_destination,
                        0,
                        "TX LFO",
                        &mut tx_jacks,
                    );
                });
                ui.horizontal(|ui| {
                    eg_preview(ui, EgAmplitudeMapping::AmplitudeLinear, 255, params.pitch_fg.eg.to_eg_params());
                    // LOOP/CURVEが縦2段の1スロットに統合された分、幅リストは8個(knob)+1個(チェック列)。
                    let widths = [KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, CHECKBOX_W];
                    justified_row(ui, &widths, |ui| {
                        fg_eg_knobs(ui, "P.", &params.pitch_fg.eg, Some(&*params.pitch_fg.depth));
                    });
                });
            });
        });

        // ---- CUTOFF FG（旧Filter EG） ----
        ui.group(|ui| {
            ui.set_width(panel_width);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("CUTOFF FG").strong());
                    crate::patchbay::texture_lfo_dest_jack(
                        ui,
                        &*params.texture_lfo_destination,
                        3,
                        "TX LFO",
                        &mut tx_jacks,
                    );
                });
                ui.horizontal(|ui| {
                    eg_preview(ui, EgAmplitudeMapping::AmplitudeLinear, 255, params.cutoff_fg.eg.to_eg_params());
                    // LOOP/CURVEが縦2段の1スロットに統合された分、幅リストは10個(knob)+1個(チェック列)。
                    let widths = [
                        KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W,
                        KNOB_W, CHECKBOX_W,
                    ];
                    justified_row(ui, &widths, |ui| {
                        knob(ui, &*params.cutoff, "CUT");
                        knob(ui, &*params.resonance, "RES");
                        fg_eg_knobs(ui, "F.", &params.cutoff_fg.eg, Some(&*params.cutoff_fg.depth));
                    });
                });
            });
        });

        // ---- GAIN FG（旧VCA EG） ----
        ui.group(|ui| {
            ui.set_width(panel_width);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("GAIN FG").strong());
                    crate::patchbay::texture_lfo_dest_jack(
                        ui,
                        &*params.texture_lfo_destination,
                        1,
                        "TX LFO",
                        &mut tx_jacks,
                    );
                });
                ui.horizontal(|ui| {
                    eg_preview(ui, EgAmplitudeMapping::AmplitudeLinear, 255, params.gain_fg.to_eg_params());
                    // LOOP/CURVEが縦2段の1スロットに統合された分、幅リストは7個(knob)+1個(チェック列)。
                    let widths = [KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, CHECKBOX_W];
                    justified_row(ui, &widths, |ui| {
                        fg_eg_knobs(ui, "V.", &params.gain_fg, None);
                    });
                });
            });
        });

        // ---- マスターエフェクト ----
        ui.group(|ui| {
            ui.set_width(panel_width);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("MASTER EFFECT").strong());
                let widths = [
                    ENUM_SELECTOR_W, KNOB_W, KNOB_W, ENUM_SELECTOR_W, KNOB_W, KNOB_W, KNOB_W,
                    KNOB_W, KNOB_W,
                ];
                justified_row(ui, &widths, |ui| {
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

        // 質感LFOパッチケーブル：全ジャック位置が出揃ったので、ドラッグ着地判定とケーブル描画を行う。
        crate::patchbay::finish_texture_lfo_patchbay(ui, &*params.texture_lfo_destination, tx_jacks);
    });
}
