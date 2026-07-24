//! `ym38x6-ui/src/panel.xml`（実際の正本）を`generate_rust`に通し、構造が壊れていないことを
//! 検証する。ここで確認する木構造（OPパネルの`eg-preview`+17ウィジェット均等割り付け等）は、
//! かつての垂直スライス検証（旧`spike.rs`の`draw_op_taffy`、現行描画と同一と確認済みの
//! 手書きtaffy版リファレンス。役目をpanel-codegen/interpret.rsが代替したため削除済み）と1:1で対応する。

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

/// OPパネルのレイアウト木が、旧`spike.rs`の`draw_op_taffy`(削除済み)で検証済みだった構造
/// （eg-preview(84x66) + LOOP/CURVEの`<stack>`(checkbox実測70x20を2個縦積み) +
/// 17ウィジェットの`row_grow`均等割り付け、うちwaveform+AMも`<stack>`(130x66+70x20)）と一致することを確認する。
#[test]
fn op_panel_tree_matches_taffy_reference() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).unwrap();
    let expected_tree = "let tree = row(Justify::Start, outer_gap, vec![leaf(84.0, 66.0), \
stack(outer_gap, vec![leaf(70.0, 20.0), leaf(70.0, 20.0)]), \
row_grow(Justify::Between, 0.0, vec![leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), \
leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), \
leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0), \
stack(0.0, vec![leaf(130.0, 66.0), leaf(70.0, 20.0)]), \
leaf(62.0, 66.0), leaf(62.0, 66.0), leaf(62.0, 66.0)])]);";
    assert!(rust.contains(expected_tree), "OPパネルの木構造が想定と異なります:\n{rust}");

    // VEL/V.GAINのenabled-ifラップ（is_carrier述語はインライン展開される。`<let>`廃止に伴い
    // 中間変数を経由しないが、計算内容は同一）、waveform_selectorのindex、LOOP/CURVEの<stack>展開。
    assert!(rust.contains(
        "ui.add_enabled_ui(!crate::algorithm_diagram::carriers(params.algorithm.value() as u8).contains(&i), |ui| { knob(ui, &*op.vel_sens, \"VEL\"); });"
    ));
    assert!(rust.contains(
        "ui.add_enabled_ui(crate::algorithm_diagram::carriers(params.algorithm.value() as u8).contains(&i), |ui| { knob(ui, &*op.velocity_gain, \"V.GAIN\"); });"
    ));
    assert!(rust.contains("waveform_selector(ui, &*op.waveform, (i) as usize);"));
    assert!(rust.contains("bool_checkbox(ui, &*op.op_loop, \"LOOP\");"));
    assert!(rust.contains("bool_checkbox(ui, &*op.curve, \"CURVE\");"));
    assert!(rust.contains("bool_checkbox(ui, &*op.ame, \"AM\");"));
}

/// OPパネルのヘッダ（動的タイトル`<title>OP {index+1}</title>` + `<readout>`）が、
/// `<raw>`廃止前と同一のRustを生成することを確認する。
#[test]
fn op_header_title_and_readout() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("ui.label(egui::RichText::new(format!(\"OP {}\", i + 1)).strong());"));
    assert!(rust.contains(
        "ui.label(egui::RichText::new(format!(\"×{:.2}\", mul_fine_ratio(op.mul.value() as u8, op.op_fine_tune.value() as u8))).size(10.0).weak()).on_hover_text(\"MUL×FINEの実効周波数比（DT1は含まない）\");"
    ));
}

/// コミット0da0584で`<column title=>`が実描画に配線されておらず脱落していた
/// CHANNEL/CHIP LFOの見出しが復活していることを確認する
/// （`<panels>`/`<panel>`統一後は他の見出しと同じ`ui.horizontal`ラップになる）。
#[test]
fn channel_and_chip_lfo_titles_present() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("ui.label(egui::RichText::new(\"CHANNEL\").strong());"));
    assert!(rust.contains("ui.label(egui::RichText::new(\"CHIP LFO\").strong());"));
}

/// `<header><title/>...`（空タグ）が親の`title=`属性から見出しを解決することを確認する
/// （PITCH FG等、タイトル+ジャックを同じ行に並べるケース）。インデント込みで一致を見るのは
/// 生成レイアウトのネスト段数（ScrollArea/group/vertical/horizontal）を暗黙に固定してしまう
/// ため、行ごとに`trim()`して緩く照合する。
#[test]
fn header_title_from_attr_resolves() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).unwrap();
    let lines: Vec<&str> = rust.lines().map(str::trim).collect();
    let idx = lines
        .iter()
        .position(|l| *l == "ui.label(egui::RichText::new(\"PITCH FG\").strong());")
        .expect("PITCH FGの見出し行が見つかりません");
    assert_eq!(
        lines[idx - 1],
        "crate::patchbay::texture_lfo_dest_jack(ui, &*params.texture_lfo_destination, 0, \"TX LFO\", &mut tx_jacks);"
    );
    assert_eq!(lines[idx - 2], "ui.horizontal(|ui| {");
    assert_eq!(lines[idx + 1], "});");
}

/// XMLに書かれた8つの`<panels>`グループ（OP repeat + CHANNEL/CHIP LFO(span4/8) + TEXTURE LFO +
/// PITCH FG + CUTOFF FG + GAIN FG + MASTER EFFECT(REVERB/CHORUS)）が全て生成されている
/// （要素の脱落がない）ことを確認する。
#[test]
fn all_panels_present() {
    let xml = panel_xml();
    let rust = panel_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("for (i, op) in params.operators.iter().enumerate()"));
    assert!(rust.contains("\"TEXTURE LFO\""));
    assert!(rust.contains("\"PITCH FG\""));
    assert!(rust.contains("\"CUTOFF FG\""));
    assert!(rust.contains("\"GAIN FG\""));
    assert!(rust.contains("\"MASTER EFFECT (REVERB)\""));
    assert!(rust.contains("\"MASTER EFFECT (CHORUS)\""));
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
