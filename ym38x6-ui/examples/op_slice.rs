//! 【一時・検証用】OPパネルの現行レイアウト vs taffy生成レイアウトの目視比較。
//!
//!   cargo run -p ym38x6-ui --example op_slice
//!
//! 上段=現行(panel.rs忠実複製) / 下段=taffy(XMLが生成すべき姿)。同一に見えれば
//! 垂直スライス成功。設計方針が固まったらspike.rs・本example・eframe dev-depごと削除する。

use std::cell::Cell;

use ym38x6_ui::spike::{draw_op_current, draw_op_taffy};
use ym38x6_ui::{BoolParamHandle, IntParamHandle, OperatorPanelParams};

struct DummyInt {
    v: Cell<i32>,
    name: String,
}
impl IntParamHandle for DummyInt {
    fn value(&self) -> i32 {
        self.v.get()
    }
    fn min(&self) -> i32 {
        0
    }
    fn max(&self) -> i32 {
        255
    }
    fn default(&self) -> i32 {
        0
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        self.v.set(value);
    }
    fn end_edit(&self) {}
}

struct DummyBool {
    v: Cell<bool>,
}
impl BoolParamHandle for DummyBool {
    fn value(&self) -> bool {
        self.v.get()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: bool) {
        self.v.set(value);
    }
    fn end_edit(&self) {}
}

fn di(value: i32, name: &str) -> Box<dyn IntParamHandle> {
    Box::new(DummyInt { v: Cell::new(value), name: name.to_string() })
}
fn db(value: bool) -> Box<dyn BoolParamHandle> {
    Box::new(DummyBool { v: Cell::new(value) })
}

fn make_op() -> OperatorPanelParams<'static> {
    OperatorPanelParams {
        tl: di(200, "TL"),
        ar: di(120, "AR"),
        d1r: di(80, "D1R"),
        d2r: di(40, "D2R"),
        d1l: di(200, "D1L"),
        rr: di(90, "RR"),
        mul: di(1, "MUL"),
        dt1: di(128, "DT1"),
        ksr: di(0, "KSR"),
        vel_sens: di(128, "VEL"),
        op_fine_tune: di(128, "FINE"),
        ame: db(false),
        waveform: di(1, "WAVE"),
        floor: di(0, "FLOOR"),
        op_loop: db(false),
        curve: db(false),
        eg_shift: di(0, "EGSFT"),
        level_scale: di(0, "LEVEL SCALE"),
        velocity_gain: di(255, "V.GAIN"),
    }
}

struct App {
    op: OperatorPanelParams<'static>,
    algorithm: u8,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let panel_width = ui.available_width() - 4.0;
        ui.label(egui::RichText::new("CURRENT (panel.rs 忠実複製)").strong());
        draw_op_current(ui, panel_width, 0, &self.op, self.algorithm);
        ui.add_space(16.0);
        ui.label(egui::RichText::new("TAFFY (XMLが生成すべき姿)").strong());
        draw_op_taffy(ui, panel_width, 0, &self.op, self.algorithm);
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1500.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "op_slice — OPパネル レイアウト比較",
        options,
        Box::new(|_cc| Ok(Box::new(App { op: make_op(), algorithm: 0 }))),
    )
}
