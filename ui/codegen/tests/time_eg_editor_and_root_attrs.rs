//! Step 3で追加した3機能の単体検証（実XMLに依存しない自己完結フィクスチャ）:
//! `<time-eg-editor>`要素・`<layout fn= params-type= scroll-id=>`ルート属性・
//! `<jack>`無しXMLでのpatchbay出力条件化。

use ui_codegen::{parse_layout, EgField, Widget};

fn wrap(layout_attrs: &str, style: &str, inner: &str) -> String {
    format!("<layout{layout_attrs}>{style}<panels>{inner}</panels></layout>")
}

#[test]
fn time_eg_editor_parses_with_style_default_size() {
    let xml = wrap(
        "",
        "",
        r#"<panel title="A"><row><time-eg-editor handle="eg" mapping="AmplitudeLinear" tl-value="255"/></row></panel>"#,
    );
    let layout = parse_layout(&xml).unwrap();
    let ui_codegen::BodyStmt::Tree(ui_codegen::TreeNode::Row { children, .. }) = &layout.groups[0].panels[0].body[1]
    else {
        panic!("<row>が見つかりません")
    };
    let ui_codegen::TreeNode::Leaf(leaf) = &children[0] else { panic!("leafが見つかりません") };
    assert_eq!(leaf.size.w, layout.style.time_eg_editor_size.w);
    assert_eq!(leaf.size.h, layout.style.time_eg_editor_size.h);
    match &leaf.widget {
        Widget::TimeEgEditor { handle, mapping, tl, min_stages, terminal_level_zero } => {
            assert_eq!(handle, "params.eg");
            assert_eq!(mapping, "AmplitudeLinear");
            assert!(matches!(tl, EgField::Literal(v) if v == "255"));
            assert_eq!(*min_stages, 2, "既定はSTAGE>=2（リリース段を必ず1本残す）");
            assert!(*terminal_level_zero, "既定は最終段level=0固定");
        }
        other => panic!("TimeEgEditorではありません: {other:?}"),
    }
}

#[test]
fn time_eg_editor_instance_size_overrides_style_default() {
    let xml = wrap(
        "",
        r#"<style><time-eg-editor width="300" height="200"/></style>"#,
        r#"<panel title="A"><row><time-eg-editor handle="eg" width="260" mapping="DbLinear" tl="tl"/></row></panel>"#,
    );
    let layout = parse_layout(&xml).unwrap();
    let ui_codegen::BodyStmt::Tree(ui_codegen::TreeNode::Row { children, .. }) = &layout.groups[0].panels[0].body[1]
    else {
        panic!("<row>が見つかりません")
    };
    let ui_codegen::TreeNode::Leaf(leaf) = &children[0] else { panic!("leafが見つかりません") };
    // widthはインスタンス属性(260)、heightはstyle既定値(200、<style>で300→200に上書き済み)のまま。
    assert_eq!(leaf.size.w, 260.0);
    assert_eq!(leaf.size.h, 200.0);
}

#[test]
fn time_eg_editor_generates_expected_call() {
    let xml = wrap(
        "",
        "",
        r#"<panel title="A"><row><time-eg-editor handle="eg" mapping="DbLinear" tl="tl"/></row></panel>"#,
    );
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(
        rust.contains(
            "time_eg_editor(ui, egui::vec2(260.0, 245.0), &*params.eg, EgAmplitudeMapping::DbLinear, params.tl.value() as u8, TimeEgProfile { min_stages: 2, terminal_level_zero: true });"
        ),
        "生成コードにtime_eg_editor呼び出しが見つかりません:\n{rust}"
    );
}

#[test]
fn root_attrs_default_to_draw_param_panel() {
    let xml = wrap("", "", r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("pub fn draw_param_panel(ui: &mut egui::Ui, params: &PanelParams) {"));
    assert!(!rust.contains("id_salt"), "省略時はid_saltを出力しないはず");
}

#[test]
fn root_attrs_override_fn_params_type_and_scroll_id() {
    let xml = wrap(
        r#" fn="draw_op505_panel" params-type="Op505PanelParams" scroll-id="op505_panel""#,
        "",
        r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#,
    );
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("pub fn draw_op505_panel(ui: &mut egui::Ui, params: &Op505PanelParams) {"));
    assert!(rust.contains("egui::ScrollArea::both().id_salt(\"op505_panel\").show(ui, |ui| {"));
}

/// ウィンドウ縮小時に重ならず横スクロールへ委ねるための下げ止まり幅（`ui/codegen/src/ir.rs`の
/// `Layout::min_full_width`）が、`PANEL_MIN_WIDTH`定数として出力され`full_width`に反映されること。
#[test]
fn full_width_is_floored_by_panel_min_width() {
    let xml = wrap("", "", r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("pub const PANEL_MIN_WIDTH: f32 ="), "{rust}");
    assert!(rust.contains("let full_width = ui.available_width().max(PANEL_MIN_WIDTH);"), "{rust}");
}

#[test]
fn xml_without_any_jack_omits_patchbay_output() {
    let xml = wrap("", "", r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(!rust.contains("JackLayout"), "jack無しXMLではtx_jacks宣言を出力しないはず");
    assert!(!rust.contains("finish_texture_lfo_patchbay"), "jack無しXMLではpatchbay呼び出しを出力しないはず");
}

#[test]
fn xml_with_jack_keeps_patchbay_output() {
    let xml = wrap(
        "",
        "",
        r#"<panel title="A"><header><jack kind="source"/><title/></header>
             <row><knob label="X" handle="x"/></row></panel>"#,
    );
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("let mut tx_jacks = crate::patchbay::JackLayout::new();"));
    assert!(rust.contains("crate::patchbay::finish_texture_lfo_patchbay"));
}
