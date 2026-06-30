//! gesture-app埋め込み用の音色エディタ（egui-wasm）。`ym38x6-ui`の共有レイアウトを
//! eframeのWebRunnerでcanvasに描画し、パラメーター変更をTauri IPC経由で
//! src-tauriの`Ym38x6Engine`へ反映する。wasm32-unknown-unknown専用クレート
//! （ワークスペースからは除外。ビルドはrustup配下のcargoで個別に行う）。

mod app;
mod handle;
mod ipc;
mod keyboard;
mod shift_keys;
mod state;

pub use shift_keys::notify_shift;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen]
pub struct EditorHandle {
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl EditorHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        console_error_panic_hook::set_once();
        Self { runner: eframe::WebRunner::new() }
    }

    /// `canvas_id`要素にエディタを描画開始する。一度startしたら`destroy()`まで動き続ける。
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
                    // eframe(web)はOSのprefers-color-scheme設定に追従して自動でLight/Darkを
                    // 切り替える。VST(nice-plug-egui、eframe非使用のため自動追従しない)と
                    // 見た目を揃えるため、システム設定に関わらずDark固定にする。
                    cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
                    Ok(Box::new(app::EditorApp::new()))
                }),
            )
            .await
    }

    /// エディタを停止し、canvasの描画ループを解放する（トグルキーで非表示にする際に呼ぶ）。
    #[wasm_bindgen]
    pub fn destroy(&self) {
        self.runner.destroy();
    }
}

impl Default for EditorHandle {
    fn default() -> Self {
        Self::new()
    }
}
