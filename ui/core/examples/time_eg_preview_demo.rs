//! `time_eg_preview`の見た目を素早く確認するためのeframeネイティブデモ。
//!
//! かつてop505-coreの試聴デモ（`op505_core::demo`、現在は廃止済み）が使っていた形と同じ
//! 折れ線をここでも独立に定義する（ui-coreはチップ非依存を保つ方針のため、op505-coreへは
//! 依存せずTimeEgParamsを直接書く）。平坦区間・ループ区間・多段リリースが折れ線として
//! どう見えるかを目視確認する。
//!
//! 実行: cargo run -p ui-core --example time_eg_preview_demo

use eframe::egui;
use sound_core::{TimeEgParams, TimeStage, MAX_STAGES};
use ui_core::eg_preview::EgAmplitudeMapping;
use ui_core::time_eg_preview::time_eg_preview;

/// eguiの既定フォントにはCJKグリフが含まれず日本語ラベルが豆腐(□)化するため、
/// Windows標準の日本語フォント(游ゴシック)をフォールバックとして追加する
/// （`tools/xml-panel-dsl/preview-native`と同じ対処）。
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\YuGothR.ttc") {
        let mut font_data = egui::FontData::from_owned(bytes);
        font_data.index = 0;
        fonts.font_data.insert("jp".to_owned(), std::sync::Arc::new(font_data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("jp".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

fn stages(entries: &[(u8, u8, u8)]) -> [TimeStage; MAX_STAGES] {
    let mut stages = [TimeStage::default(); MAX_STAGES];
    for (i, &(time, level, curve)) in entries.iter().enumerate() {
        stages[i] = TimeStage { time, level, curve };
    }
    stages
}

struct Sample {
    label: &'static str,
    mapping: EgAmplitudeMapping,
    tl: u8,
    params: TimeEgParams,
}

fn samples() -> Vec<Sample> {
    vec![
        Sample {
            label: "Gain Switch (e3) — 静止を挟んだ音量2値スイッチ・4段ループ+リリース",
            mapping: EgAmplitudeMapping::AmplitudeLinear,
            tl: 255,
            params: TimeEgParams {
                stages: stages(&[(15, 230, 0), (40, 230, 0), (15, 40, 0), (40, 40, 0), (100, 0, 0)]),
                stage_count: 5,
                loop_enabled: 1,
                loop_start: 0,
                release_point: 3,
            },
        },
        Sample {
            label: "Modulator Multi-stage (m2) — モジュレーターEGの多段ループ（DbLinear）",
            mapping: EgAmplitudeMapping::DbLinear,
            tl: 220,
            params: TimeEgParams {
                // 4段ループ(0..=3)を保ったままリリース段を1本足した（旧定義はrelease_start=3で
                // ループ終端と同じ段を指しており、リリース区間が実質空だった）。
                stages: stages(&[(20, 220, 0), (50, 220, 0), (20, 80, 0), (50, 80, 0), (60, 0, 0)]),
                stage_count: 5,
                loop_enabled: 1,
                loop_start: 0,
                release_point: 3,
            },
        },
        Sample {
            label: "Pitch Steps (m3) — 階段状プラトー・8段ループ+速いリセット+リリース",
            mapping: EgAmplitudeMapping::AmplitudeLinear,
            tl: 255,
            params: TimeEgParams {
                stages: stages(&[
                    (15, 90, 0),
                    (45, 90, 0),
                    (15, 170, 0),
                    (45, 170, 0),
                    (15, 255, 0),
                    (45, 255, 0),
                    (25, 20, 0),
                    (110, 0, 0),
                ]),
                stage_count: 8,
                loop_enabled: 1,
                loop_start: 0,
                release_point: 6,
            },
        },
        Sample {
            label: "One-shot（ループなし）— 3段ADSR風、curve=1で丸めた立ち上がり",
            mapping: EgAmplitudeMapping::DbLinear,
            tl: 255,
            params: TimeEgParams {
                // Attack(→255) + Decay(→180)が保持区間、サステインは180。リリースは段2で無音へ。
                stages: stages(&[(20, 255, 1), (60, 180, 0), (150, 0, 0)]),
                stage_count: 3,
                loop_enabled: 0,
                loop_start: 0,
                release_point: 1,
            },
        },
        Sample {
            label: "多段リリース — リリース点の次から複数段を順に辿って無音へ",
            mapping: EgAmplitudeMapping::AmplitudeLinear,
            tl: 255,
            params: TimeEgParams {
                stages: stages(&[(10, 255, 0), (200, 255, 0), (30, 150, 0), (60, 60, 0), (80, 0, 0)]),
                stage_count: 5,
                loop_enabled: 0,
                loop_start: 0,
                release_point: 1,
            },
        },
    ]
}

struct App {
    samples: Vec<Sample>,
}

impl App {
    fn new() -> Self {
        Self { samples: samples() }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for sample in &self.samples {
                ui.group(|ui| {
                    ui.label(sample.label);
                    let p = &sample.params;
                    ui.label(format!(
                        "stage_count={} loop_enabled={} loop=[{},{}] release_point={}",
                        p.stage_count, p.loop_enabled, p.loop_start, p.release_point, p.release_point
                    ));
                    time_eg_preview(ui, egui::vec2(360.0, 140.0), sample.mapping, sample.tl, *p);
                });
                ui.add_space(8.0);
            }
        });
    }
}

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("time_eg_preview demo").with_inner_size([420.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "time_eg_preview demo",
        native_options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}
