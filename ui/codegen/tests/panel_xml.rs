//! `op505-ui/src/panel.xml`（実際の正本）を`generate_rust`に通し、構造が壊れていないことを
//! 検証する。ここで確認する木構造（OPパネルの`time-eg-editor`+11ウィジェットの`stack_grow`3行
//! 折り返し等）は、ビルド成果物（`$OUT_DIR/panel_generated.rs`）の実測と一致させてある
//! （2026-08-20、ym38x6-ui削除に伴い旧ym38x6-ui版のリファレンスから移行。旧版はOPパネルが
//! 1列・17ウィジェット均等割りだったが、op505-uiはOPパネルのcolumns=2グリッド化・TimeEg
//! エディタ化により木構造が異なる）。

fn panel_xml() -> String {
    std::fs::read_to_string("../../op505/ui/src/panel.xml").expect("panel.xml が見つかりません")
}

#[test]
fn generates_without_error() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).expect("generate_rust が失敗しました");
    assert!(rust.starts_with("pub fn draw_op505_panel(ui: &mut egui::Ui, params: &Op505PanelParams) {"));
    assert!(rust.trim_end().ends_with('}'));
}

/// OPパネルのレイアウト木が、ビルド成果物の実測（`time-eg-editor`(260x250) + 11ウィジェットの
/// `stack_grow`3行折り返し(62x71×4 + 62x71×4 + 62x71+waveform/AMの`<stack>`(130x71+70x25))）と
/// 一致することを確認する。
///
/// ノブの宣言幅62は`ui_core::knob::KNOB_CELL_SIZE`と同期する値。
#[test]
fn op_panel_tree_matches_taffy_reference() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    let expected_tree = "let tree = row(Justify::Start, outer_gap, vec![leaf(260.0, 250.0), \
stack_grow(outer_gap, vec![\
row(Justify::Between, 0.0, vec![leaf(62.0, 71.0), leaf(62.0, 71.0), leaf(62.0, 71.0), leaf(62.0, 71.0)]), \
row(Justify::Between, 0.0, vec![leaf(62.0, 71.0), leaf(62.0, 71.0), leaf(62.0, 71.0), leaf(62.0, 71.0)]), \
row(Justify::Between, 0.0, vec![leaf(62.0, 71.0), stack(0.0, vec![leaf(130.0, 71.0), leaf(70.0, 25.0)])])\
])]);";
    assert!(rust.contains(expected_tree), "OPパネルの木構造が想定と異なります:\n{rust}");

    // VEL/V.GAINのenabled-ifラップ（is_carrier述語はインライン展開される）、waveform_selectorのindex。
    assert!(rust.contains(
        "ui.add_enabled_ui(!crate::algorithm_diagram::carriers(params.algorithm.value() as u8).contains(&i), |ui| { knob(ui, &*op.vel_sens, \"VEL\"); });"
    ));
    assert!(rust.contains(
        "ui.add_enabled_ui(crate::algorithm_diagram::carriers(params.algorithm.value() as u8).contains(&i), |ui| { knob(ui, &*op.velocity_gain, \"V.GAIN\"); });"
    ));
    assert!(rust.contains("waveform_selector(ui, &*op.waveform, (i) as usize);"));
    assert!(rust.contains("bool_checkbox(ui, &*op.ame, \"AM\");"));
}

/// OPパネルのヘッダ（動的タイトル`<title>OP {index+1}</title>` + `<readout>`）が、
/// `<raw>`廃止前と同一のRustを生成することを確認する。
#[test]
fn op_header_title_and_readout() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).unwrap();
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
    let rust = ui_codegen::generate_rust(&xml).unwrap();
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
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    let lines: Vec<&str> = rust.lines().map(str::trim).collect();
    let idx = lines
        .iter()
        .position(|l| *l == "ui.label(egui::RichText::new(\"PITCH FG\").strong());")
        .expect("PITCH FGの見出し行が見つかりません");
    // dest-index=1（Pitch、2026-08-18のFmLfoDestination並べ替え後。旧0=Pitch→新1=Pitch）。
    assert_eq!(
        lines[idx - 1],
        "crate::patchbay::texture_lfo_dest_jack(ui, &*params.texture_lfo_destination, 1, \"TX LFO\", &mut tx_jacks);"
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
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("for (i_row, i_chunk) in params.operators.chunks(2).enumerate()"));
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
    let a = ui_codegen::generate_rust(&xml).unwrap();
    let b = ui_codegen::generate_rust(&xml).unwrap();
    assert_eq!(a, b);
}

/// `<panel repeat="..." columns="N">`（op505-ui/src/panel.xmlのOPパネルで採用したN列グリッド、
/// 2026-08-14追加）が、行ごとの`chunks(N)`+`ui.horizontal`折り返しへ展開されることを確認する。
/// フラットな`index`変数（本文中の`{index+1}`等）が行×列から復元される点も合わせて検証する。
#[test]
fn repeat_panel_with_columns_wraps_into_grid() {
    let xml = r#"<layout><panels><panel repeat="operators" as="op" index="i" columns="2">
        <header><title>OP {index+1}</title></header>
        <row><knob label="X" handle="op.x"/></row>
    </panel></panels></layout>"#;
    let rust = ui_codegen::generate_rust(xml).unwrap();
    assert!(rust.contains("for (i_row, i_chunk) in params.operators.chunks(2).enumerate()"), "{rust}");
    assert!(rust.contains("for (i_col, op) in i_chunk.iter().enumerate()"), "{rust}");
    assert!(rust.contains("let i = i_row * 2 + i_col;"), "{rust}");
    assert!(rust.contains("ui.set_width(w_cell);"), "{rust}");
    assert!(rust.contains("ui.label(egui::RichText::new(format!(\"OP {}\", i + 1)).strong());"), "{rust}");
}

/// `columns`なしの既存経路（縦一列）は今回の変更で壊れていないことを確認する回帰テスト。
#[test]
fn repeat_panel_without_columns_stays_linear() {
    let xml = r#"<layout><panels><panel repeat="operators" as="op" index="i">
        <row><knob label="X" handle="op.x"/></row>
    </panel></panels></layout>"#;
    let rust = ui_codegen::generate_rust(xml).unwrap();
    assert!(rust.contains("for (i, op) in params.operators.iter().enumerate()"), "{rust}");
    assert!(!rust.contains("chunks("), "{rust}");
}

/// `<stack grow="true">`（2026-08-14追加、`<panel repeat columns="N">`でセル幅が狭まったOPパネルの
/// ノブ群をN行へ折り返すのに使う）が`stack_grow(...)`へ生成されることを確認する。
/// 中の`<row>`は自身に`grow`を付けなくても、既定の`align-items: stretch`でこの幅いっぱいに
/// 引き伸ばされる想定（`gen_repeat_grid`のセル幅計算とは独立した、木構造の生成だけを確認する）。
#[test]
fn stack_grow_generates_stack_grow_ctor() {
    let xml = r#"<layout><panels><panel title="A">
        <row justify="start" gap="spacing">
            <knob label="X" handle="x"/>
            <stack grow="true" gap="spacing">
                <row justify="between"><knob label="Y" handle="y"/></row>
                <row justify="between"><knob label="Z" handle="z"/></row>
            </stack>
        </row>
    </panel></panels></layout>"#;
    let rust = ui_codegen::generate_rust(xml).unwrap();
    assert!(rust.contains("stack_grow(outer_gap, vec![row(Justify::Between"), "{rust}");
}

/// 通常の`<stack>`（`grow`省略）は従来通り`stack(...)`のままであることを確認する回帰テスト。
#[test]
fn stack_without_grow_stays_plain_stack() {
    let xml = r#"<layout><panels><panel title="A">
        <row justify="start" gap="spacing">
            <stack>
                <knob label="Y" handle="y"/>
                <checkbox label="Z" handle="z"/>
            </stack>
        </row>
    </panel></panels></layout>"#;
    let rust = ui_codegen::generate_rust(xml).unwrap();
    assert!(rust.contains("let tree = row(Justify::Start, outer_gap, vec![stack(0.0"), "{rust}");
    assert!(!rust.contains("stack_grow"), "{rust}");
}
