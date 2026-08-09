//! `src/panel.xml`（正本）から`draw_op505_panel`本体を生成し、`panel.rs`の
//! `include!(concat!(env!("OUT_DIR"), "/panel_generated.rs"))`が読み込む
//! `$OUT_DIR/panel_generated.rs`へ書き出す。
//!
//! パース・コード生成のロジックは`ui-codegen`（egui非依存の純粋クレート）に一本化されており、
//! `tools/xml-panel-dsl`のブラウザプレビュー（wasm）とも共有する。ここでは
//! ファイルI/Oと失敗時のビルド停止のみを担う（`ym38x6-ui/build.rs`と同型）。

use std::path::Path;

fn main() {
    let xml_path = "src/panel.xml";
    println!("cargo:rerun-if-changed={xml_path}");

    let xml = std::fs::read_to_string(xml_path)
        .unwrap_or_else(|e| panic!("panel.xml を読み込めませんでした: {e}"));
    let rust = ui_codegen::generate_rust(&xml)
        .unwrap_or_else(|e| panic!("panel.xml のコード生成に失敗しました: {e}"));

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR が設定されていません");
    let out_path = Path::new(&out_dir).join("panel_generated.rs");
    std::fs::write(&out_path, rust)
        .unwrap_or_else(|e| panic!("{}への書き込みに失敗しました: {e}", out_path.display()));
}
