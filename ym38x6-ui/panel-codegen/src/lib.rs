//! `panel.xml` → `ym38x6-ui/src/panel.rs`の`draw_param_panel`本体を生成するコード生成器。
//!
//! `ym38x6-ui/build.rs`（ネイティブ、`generate_rust`のみ使用）と、
//! `tools/xml-panel-dsl/codegen-wasm`（ブラウザ、[`preview`]モジュールも使用）の
//! 双方から共有される。egui/wasm-bindgenに依存しない純粋なRustクレート。

mod codegen;
mod ir;
mod parse;
mod preview;

pub use ir::*;
pub use parse::parse_layout;
pub use preview::{parse_ir_preview, solve_tree_json};

/// XML全文から`draw_param_panel`関数（本体アイテムのみ、`use`文は含まない）を生成する。
pub fn generate_rust(xml: &str) -> Result<String, String> {
    let layout = parse_layout(xml)?;
    Ok(codegen::generate_rust(&layout))
}
