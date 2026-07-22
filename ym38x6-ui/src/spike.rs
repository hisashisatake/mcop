//! 【一時・検証用】XML UIレイアウトDSLの垂直スライス検証。
//!
//! OPパネル1枚（eg_preview＋17ウィジェットの均等割り付け＋checkstack、入れ子レイアウトの代表）
//! について、現行の手書きレイアウト（[`draw_op_current`]、panel.rsからの忠実な複製）と、
//! 「XMLが生成すべきtaffyベースのコード」（[`draw_op_taffy`]）を並べ、同一描画になるかを
//! 目視検証するための使い捨てモジュール。設計方針が固まったら削除する。
//!
//! 検証exampleは`examples/op_slice.rs`。

use crate::algorithm_diagram::carriers;
use crate::eg_preview::{eg_preview, EgAmplitudeMapping};
use crate::knob::{bool_checkbox, knob};
use crate::layout::{self, leaf, row, row_grow, Justify};
use crate::panel::OperatorPanelParams;
use crate::waveform::waveform_selector;
use sound_core::eg::EgParams;

const KNOB_W: f32 = 62.0;
const CHECKBOX_W: f32 = 50.0;
const WAVEFORM_SELECTOR_W: f32 = 130.0;
const EG_PREVIEW_W: f32 = 84.0;
const ROW_H: f32 = 66.0;

fn mul_to_ratio(mul: u8) -> f32 {
    const TABLE: [f32; 16] = [
        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    TABLE[(mul as usize).min(15)]
}
fn op_fine_tune_to_cents(v: u8) -> f32 {
    (v as f32 - 128.0) / 128.0 * 1200.0
}
fn mul_fine_ratio(mul: u8, op_fine_tune: u8) -> f32 {
    mul_to_ratio(mul) * 2f32.powf(op_fine_tune_to_cents(op_fine_tune) / 1200.0)
}

/// ノブ等の自然幅の合計とコンテナ幅の差を要素間へ均等配分する（現行`justified_row`の複製）。
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

/// OPパネルのヘッダ行（"OP n" ＋ MUL×FINE実効比率）。両パスで共通に使う（`<header>`相当）。
fn op_header(ui: &mut egui::Ui, i: usize, op: &OperatorPanelParams) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("OP {}", i + 1)).strong());
        let ratio = mul_fine_ratio(op.mul.value() as u8, op.op_fine_tune.value() as u8);
        ui.label(egui::RichText::new(format!("\u{d7}{ratio:.2}")).size(10.0).weak())
            .on_hover_text("MUL×FINEの実効周波数比（DT1は含まない）");
    });
}

fn op_eg_params(op: &OperatorPanelParams) -> EgParams {
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
    }
}

// ===========================================================================
//  現行パス（panel.rsからの忠実な複製）
// ===========================================================================
pub fn draw_op_current(ui: &mut egui::Ui, panel_width: f32, i: usize, op: &OperatorPanelParams, algorithm: u8) {
    ui.group(|ui| {
        ui.set_width(panel_width);
        ui.vertical(|ui| {
            op_header(ui, i, op);
            ui.horizontal(|ui| {
                eg_preview(ui, EgAmplitudeMapping::DbLinear, op.tl.value() as u8, op_eg_params(op));
                let is_carrier = carriers(algorithm).contains(&i);
                let widths = [
                    KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W, KNOB_W,
                    KNOB_W, KNOB_W, WAVEFORM_SELECTOR_W, KNOB_W, CHECKBOX_W, KNOB_W, KNOB_W,
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
                    ui.add_enabled_ui(!is_carrier, |ui| knob(ui, &*op.vel_sens, "VEL"));
                    ui.add_enabled_ui(is_carrier, |ui| knob(ui, &*op.velocity_gain, "V.GAIN"));
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

// ===========================================================================
//  taffyパス（XMLが生成すべきコードの姿）
// ===========================================================================
//  対応XML（設計案）:
//    <panel repeat="operators" as="op" index="i">
//      <header expr='format!("OP {}", i + 1)'/>            ← op_header（<raw>相当）
//      <row justify="start">
//        <eg-preview mapping="DbLinear" level="tl" .../>
//        <row grow justify="between">
//          <knob label="TL" handle="tl"/> ... <knob label="KSR" handle="ksr"/>
//          <knob label="VEL"    handle="vel_sens"      enabled-if="!is_carrier"/>
//          <knob label="V.GAIN" handle="velocity_gain" enabled-if="is_carrier"/>
//          <knob label="FINE" handle="op_fine_tune"/>
//          <waveform handle="waveform" index="i"/>
//          <knob label="FLOOR" handle="floor"/>
//          <checkbox-stack>
//            <checkbox label="AM" handle="ame"/> <checkbox label="LOOP" handle="op_loop"/>
//            <checkbox label="CURVE" handle="curve"/>
//          </checkbox-stack>
//          <knob label="EGSFT" handle="eg_shift"/>
//          <knob label="LEVEL SCALE" handle="level_scale"/>
//        </row>
//      </row>
//    </panel>
pub fn draw_op_taffy(ui: &mut egui::Ui, panel_width: f32, i: usize, op: &OperatorPanelParams, algorithm: u8) {
    ui.group(|ui| {
        ui.set_width(panel_width);
        ui.vertical(|ui| {
            op_header(ui, i, op);

            let is_carrier = carriers(algorithm).contains(&i);
            let outer_gap = ui.spacing().item_spacing.x;
            let avail = ui.available_width();

            // レイアウト木（XMLの<row>/<eg-preview>/<knob>...に1:1対応）。
            let k = |()| leaf(KNOB_W, ROW_H);
            let tree = row(
                Justify::Start,
                outer_gap,
                vec![
                    leaf(EG_PREVIEW_W, ROW_H),
                    row_grow(
                        Justify::Between,
                        0.0,
                        vec![
                            k(()), k(()), k(()), k(()), k(()), k(()), k(()), k(()), k(()), // TL..KSR
                            k(()), k(()),                                                   // VEL, V.GAIN
                            k(()),                                                          // FINE
                            leaf(WAVEFORM_SELECTOR_W, ROW_H),                               // waveform
                            k(()),                                                          // FLOOR
                            leaf(70.0, ROW_H),                                              // checkstack(実測値70。CHECKBOX_W=50はLOOP/CURVEラベルに対し過小)
                            k(()), k(()),                                                   // EGSFT, LEVEL SCALE
                        ],
                    ),
                ],
            );
            let rects = layout::solve(egui::vec2(avail, ROW_H), &tree);

            // 描画原点を確保し、行の高さぶんカーソルを進める（後続パネルと重ならないように）。
            let origin = ui.cursor().min;
            ui.allocate_space(egui::vec2(avail, ROW_H));

            // rects はDFS順（木を書いた順）: [0]=eg_preview, [1..=17]=17ウィジェット。
            let mut r = rects.iter();
            layout::place(ui, origin, *r.next().unwrap(), |ui| {
                eg_preview(ui, EgAmplitudeMapping::DbLinear, op.tl.value() as u8, op_eg_params(op));
            });
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.tl, "TL"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.ar, "AR"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.d1r, "D1R"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.d2r, "D2R"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.d1l, "D1L"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.rr, "RR"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.mul, "MUL"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.dt1, "DT1"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.ksr, "KSR"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| {
                ui.add_enabled_ui(!is_carrier, |ui| knob(ui, &*op.vel_sens, "VEL"));
            });
            layout::place(ui, origin, *r.next().unwrap(), |ui| {
                ui.add_enabled_ui(is_carrier, |ui| knob(ui, &*op.velocity_gain, "V.GAIN"));
            });
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.op_fine_tune, "FINE"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| waveform_selector(ui, &*op.waveform, i));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.floor, "FLOOR"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| {
                ui.vertical(|ui| {
                    bool_checkbox(ui, &*op.ame, "AM");
                    bool_checkbox(ui, &*op.op_loop, "LOOP");
                    bool_checkbox(ui, &*op.curve, "CURVE");
                });
            });
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.eg_shift, "EGSFT"));
            layout::place(ui, origin, *r.next().unwrap(), |ui| knob(ui, &*op.level_scale, "LEVEL SCALE"));
        });
    });
}
