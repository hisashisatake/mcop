//! `panel.xml` → `ym38x6-ui/src/panel.rs`の`draw_param_panel`本体を生成するコード生成器。
//!
//! `ym38x6-ui/build.rs`（ネイティブ、`generate_rust`のみ使用）と、`ym38x6-ui/preview-wasm`
//! （ブラウザ、`parse_layout`のIRを`ym38x6-ui`の`interpret`モジュールへそのまま渡し実egui描画する）の
//! 双方から共有される。egui/wasm-bindgenに依存しない純粋なRustクレート。

mod codegen;
mod ir;
mod parse;

pub use ir::*;
pub use parse::parse_layout;

/// XML全文から`draw_param_panel`関数（本体アイテムのみ、`use`文は含まない）を生成する。
pub fn generate_rust(xml: &str) -> Result<String, String> {
    let layout = parse_layout(xml)?;
    Ok(codegen::generate_rust(&layout))
}
