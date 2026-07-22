//! `panel-codegen`をwasm-bindgenでラップし、`tools/xml-panel-dsl/index.html`から
//! 呼べるようにする薄い橋渡し。ロジック本体は一切持たない（`panel-codegen`に一本化済み）。

use wasm_bindgen::prelude::*;

/// XML全文から`draw_param_panel`関数（本体アイテムのみ）を生成する。
#[wasm_bindgen]
pub fn generate_rust(xml: &str) -> Result<String, JsError> {
    panel_codegen::generate_rust(xml).map_err(|e| JsError::new(&e))
}

/// XML全文をパースし、ブラウザ側プレビュー描画に必要な構造をJSON文字列で返す。
#[wasm_bindgen]
pub fn parse_ir_preview(xml: &str) -> Result<String, JsError> {
    panel_codegen::parse_ir_preview(xml).map_err(|e| JsError::new(&e))
}

/// [`parse_ir_preview`]が返した`tree.node`のJSONを本物のtaffyで解決し、矩形配列をJSONで返す。
#[wasm_bindgen]
pub fn solve_tree_json(node_json: &str, container_w: f32, container_h: f32) -> Result<String, JsError> {
    panel_codegen::solve_tree_json(node_json, container_w, container_h).map_err(|e| JsError::new(&e))
}
