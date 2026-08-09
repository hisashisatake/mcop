//! `tools/xml-panel-dsl/index.html`から呼ばれる、panel.xmlの実egui描画プレビュー（フェーズB）。
//! `ym38x6-ui`の`preview`フィーチャ（`interpret.rs`）をeframeのWebRunnerでcanvasに描画し、
//! ユーザーがXMLを編集するたびに`set_xml`で再パース・再描画する（wasm32-unknown-unknown専用
//! クレート、ワークスペースから除外。ビルドはrustup配下のcargoで個別に行う。
//! gesture-app/editor-wasmと同型）。

use std::cell::RefCell;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use ym38x6_ui::interpret::{draw_panel_from_ir, HandleStore};

thread_local! {
    static XML_STATE: RefCell<XmlState> = RefCell::new(XmlState::default());
    /// `app.rs`（gesture-app/editor-wasm）と同じパターン: `set_xml`はeguiのcanvas入力経路を
    /// 経由しないJS直呼び出しのため、直近フレームの`egui::Context`を保持して即座に再描画を促す。
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct XmlState {
    layout: Option<ui_codegen::Layout>,
    error: Option<String>,
}

fn request_repaint() {
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

/// エディタ側でXMLを変更するたびに呼ぶ。パースに失敗しても直前まで表示していたレイアウトは
/// 保持し（キャンバスが突然空にならないように）、エラーメッセージのみ上書きする。
#[wasm_bindgen]
pub fn set_xml(xml: &str) {
    XML_STATE.with(|s| {
        let mut s = s.borrow_mut();
        match ui_codegen::parse_layout(xml) {
            Ok(layout) => {
                s.layout = Some(layout);
                s.error = None;
            }
            Err(e) => s.error = Some(e),
        }
    });
    request_repaint();
}

/// XMLが妥当かどうかだけを検査する（index.htmlの上部OK/NGバッジ用）。
/// 実際の描画反映は`set_xml`、生成Rustコードの実体は`ym38x6-ui/build.rs`が使う
/// `ui_codegen::generate_rust`（index.html側では表示しない）が担う。
#[wasm_bindgen]
pub fn validate_xml(xml: &str) -> Result<(), JsError> {
    ui_codegen::parse_layout(xml).map(|_| ()).map_err(|e| JsError::new(&e))
}

struct PreviewApp {
    store: HandleStore,
}

impl PreviewApp {
    fn new() -> Self {
        Self { store: HandleStore::new() }
    }
}

impl eframe::App for PreviewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        CTX.with(|c| *c.borrow_mut() = Some(ui.ctx().clone()));
        egui::CentralPanel::default().show_inside(ui, |ui| {
            XML_STATE.with(|s| {
                let s = s.borrow();
                // キャンバス内に描く文字列は必ずASCIIにする。wasm版のeguiは既定フォントしか持たず
                // CJKグリフが無いため日本語は豆腐(□)になる（preview-nativeはシステムの游ゴシックを
                // フォールバック読み込みできるが、wasmで同じことをするとフォントをbase64で
                // 抱え込むことになり自己完結HTMLのサイズが破綻する）。日本語のエラー本文は
                // index.html側のDOM（#errBox）が表示するので、ここでは短い英語で足りる。
                if s.error.is_some() {
                    ui.colored_label(egui::Color32::from_rgb(230, 80, 70), "XML parse error (see message above the canvas)");
                    ui.separator();
                }
                match &s.layout {
                    Some(layout) => draw_panel_from_ir(ui, layout, &mut self.store),
                    None => {
                        ui.label("Open or edit panel.xml to see the live preview here.");
                    }
                }
            });
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}

/// JS側から生成・保持するプレビューのハンドル（`gesture-app/editor-wasm`の`EditorHandle`と同型）。
#[wasm_bindgen]
pub struct PreviewHandle {
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl PreviewHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        console_error_panic_hook::set_once();
        Self { runner: eframe::WebRunner::new() }
    }

    /// `canvas_id`要素にプレビューの描画を開始する。一度startしたら`destroy()`まで動き続ける。
    #[wasm_bindgen]
    pub async fn start(&self, canvas_id: &str) -> Result<(), JsValue> {
        let document = web_sys::window()
            .ok_or_else(|| JsValue::from_str("no window"))?
            .document()
            .ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas = document
            .get_element_by_id(canvas_id)
            .ok_or_else(|| JsValue::from_str(&format!("canvas '{canvas_id}' not found")))?
            .dyn_into::<web_sys::HtmlCanvasElement>()?;

        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
                    Ok(Box::new(PreviewApp::new()))
                }),
            )
            .await
    }

    /// プレビューを停止し、canvasの描画ループを解放する。
    #[wasm_bindgen]
    pub fn destroy(&self) {
        self.runner.destroy();
    }
}

impl Default for PreviewHandle {
    fn default() -> Self {
        Self::new()
    }
}
