//! `ym38x6-ui/src/panel.xml`（実際の正本）を`generate_rust`に通し、構造が壊れていないことを
//! 検証する。ここで確認する木構造（OPパネルの`eg-preview`+17ウィジェット均等割り付け等）は、
//! `ym38x6-ui/src/spike.rs`の`draw_op_taffy`（垂直スライス検証で現行描画と同一と確認済みの
//! 手書きtaffy版リファレンス）と1:1で対応する。

fn panel_xml() -> String {
    std::fs::read_to_string("../src/panel.xml").expect("panel.xml が見つかりません")
}

#[test]
fn generates_without_error() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).expect("generate_rust が失敗しました");
    assert!(rust.starts_with("pub fn draw_param_panel(ui: &mut egui::Ui, params: &PanelParams) {"));
    assert!(rust.trim_end().ends_with('}'));
}

/// OPパネルのレイアウト木が、`spike.rs`の`draw_op_taffy`で検証済みの構造
/// （eg-preview(84x66) + 17ウィジェットの`row_grow`均等割り付け、うちcheckbox-stackは実測70幅）
/// と一致することを確認する。
#[test]
fn op_panel_tree_matches_taffy_reference() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).unwrap();
    let expected_tree = "let tree = row(Justify::Start, outer_gap, vec![leaf(84.0, 66.0), \
row_grow(Justify::Between, 0.0, vec![leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), \
leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), \
leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(130.0, 66.0), \
leaf(62.0, 66.0), leaf(70.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0)])]);";
    assert!(rust.contains(expected_tree), "OPパネルの木構造が想定と異なります:\n{rust}");

    // VEL/V.GAINのenabled-ifラップ、waveform_selectorのindex、checkbox-stackのui.vertical化。
    assert!(rust.contains("ui.add_enabled_ui(!is_carrier, |ui| { knob(ui, &*op.vel_sens, \"VEL\"); });"));
    assert!(rust.contains("ui.add_enabled_ui(is_carrier, |ui| { knob(ui, &*op.velocity_gain, \"V.GAIN\"); });"));
    assert!(rust.contains("waveform_selector(ui, &*op.waveform, (i) as usize);"));
    assert!(rust.contains(
        "ui.vertical(|ui| { bool_checkbox(ui, &*op.ame, \"AM\"); bool_checkbox(ui, &*op.op_loop, \"LOOP\"); bool_checkbox(ui, &*op.curve, \"CURVE\"); });"
    ));
}

/// XMLに書かれた8アイテム（OP repeat + CHANNEL/CHIP LFO columns + TEXTURE LFO + PITCH FG +
/// CUTOFF FG + GAIN FG + MASTER EFFECT）が全て生成されている（要素の脱落がない）ことを確認する。
#[test]
fn all_panels_present() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("for (i, op) in params.operators.iter().enumerate()"));
    assert!(rust.contains("\"TEXTURE LFO\""));
    assert!(rust.contains("\"PITCH FG\""));
    assert!(rust.contains("\"CUTOFF FG\""));
    assert!(rust.contains("\"GAIN FG\""));
    assert!(rust.contains("\"MASTER EFFECT\""));
    assert!(rust.contains("params.chip_lfo_freq"));
    assert!(rust.contains("finish_texture_lfo_patchbay"));
}

#[test]
fn is_deterministic() {
    let xml = panel_xml();
    let a = panel_codegen::generate_rust(&xml).unwrap();
    let b = panel_codegen::generate_rust(&xml).unwrap();
    assert_eq!(a, b);
}

#[test]
fn preview_roundtrip_solves_op_panel() {
    let xml = panel_xml();
    let preview_json = panel_codegen::parse_ir_preview(&xml).expect("parse_ir_preview が失敗しました");
    let value: serde_json::Value = serde_json::from_str(&preview_json).unwrap();
    let items = value["items"].as_array().unwrap();
    let op_panel = items.iter().find(|it| it["repeat"] == "operators").expect("OPパネルが見つかりません");
    let body = op_panel["body"].as_array().unwrap();
    let tree_stmt = body.iter().find(|st| st["kind"] == "tree").expect("tree文が見つかりません");
    assert_eq!(tree_stmt["max_height"].as_f64().unwrap(), 66.0);
    let leaves = tree_stmt["leaves"].as_array().unwrap();
    assert_eq!(leaves.len(), 18); // eg-preview + 17ウィジェット

    let node_json = serde_json::to_string(&tree_stmt["node"]).unwrap();
    let rects_json = panel_codegen::solve_tree_json(&node_json, 1200.0, 66.0).expect("solve_tree_json が失敗しました");
    let rects: Vec<serde_json::Value> = serde_json::from_str(&rects_json).unwrap();
    assert_eq!(rects.len(), 18);
    // eg-previewは左詰め・幅84のまま。
    assert_eq!(rects[0]["x"].as_f64().unwrap(), 0.0);
    assert_eq!(rects[0]["w"].as_f64().unwrap(), 84.0);
}
