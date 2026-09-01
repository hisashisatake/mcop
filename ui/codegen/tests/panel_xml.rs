//! `op505-ui/src/panel.xml`（実際の正本）を`generate_rust`に通し、構造が壊れていないことを
//! 検証する。ここで確認する木構造（OPパネルの`<stack center="true">`複数本+`time-eg-editor`等）は、
//! ビルド成果物（`$OUT_DIR/panel_generated.rs`）の実測と一致させてある。panel.xmlのOPパネル内訳を
//! 編集するたびに`op_panel_tree_matches_taffy_reference`の期待値も追従が必要（2026-08-21時点の構成）。

fn panel_xml() -> String {
    std::fs::read_to_string("../../op505/ui/src/panel.xml").expect("panel.xml が見つかりません")
}

#[test]
fn generates_without_error() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).expect("generate_rust が失敗しました");
    assert!(rust.contains("pub fn draw_op505_panel(ui: &mut egui::Ui, params: &Op505PanelParams) {"));
    assert!(rust.trim_end().ends_with('}'));
}

/// OPパネルのレイアウト木が、ビルド成果物の実測と一致することを確認する（2026-08-23更新、
/// 5列stackからrow×3段（TL/MUL/EGSFT/V.GAIN・VEL統合・DT1/FINE/KSR/LEVEL SCALE・WAVE/AM）
/// への組み替え、およびV.GAIN/VELの1ノブ統合(dual_knob)に追従）。
/// **この木構造アサーションは非常に壊れやすい**——panel.xmlのOPパネル内訳
/// （stack/rowの分割単位・並び順）を変えるたびに追従が必要。
/// 外形サイズはウィジェット自然サイズ+既定マージン4px×2辺。
#[test]
fn op_panel_tree_matches_taffy_reference() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    let expected_tree = "let tree = row(Justify::Start, outer_gap, vec![\
leaf(268.0, 253.0), \
stack_centered(outer_gap, vec![\
row(Justify::Start, outer_gap, vec![leaf(70.0, 74.0), leaf(70.0, 74.0), leaf(70.0, 74.0), leaf(70.0, 74.0)]), \
row(Justify::Start, outer_gap, vec![leaf(70.0, 74.0), leaf(70.0, 74.0), leaf(70.0, 74.0), leaf(70.0, 74.0)]), \
row(Justify::Start, outer_gap, vec![leaf(138.0, 74.0), leaf(78.0, 28.0)])\
])]);";
    assert!(rust.contains(expected_tree), "OPパネルの木構造が想定と異なります:\n{rust}");

    // V.GAIN/VELは1ノブへ統合済み（dual_knob）。is_carrier述語で実ハンドル自体を出し分ける
    // if/elseへ展開される（通常のenabled-ifグレーアウトとは異なる）。
    assert!(rust.contains(
        "if crate::algorithm_diagram::carriers(params.algorithm.value() as u8).contains(&i) { dual_knob(ui, &*op.velocity_gain, \"V.GAIN\", \"VEL\", true); } else { dual_knob(ui, &*op.vel_sens, \"V.GAIN\", \"VEL\", false); }"
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
        "ui.label(egui::RichText::new(format!(\"×{:.2}\", mul_fine_ratio(op.mul.value() as u8, op.op_fine_tune.value() as u8))).size(10.0).weak()).on_hover_text(\"Effective frequency ratio of MUL×FINE (excludes DT1)\");"
    ));
}

/// コミット0da0584で`<column title=>`が実描画に配線されておらず脱落していた
/// CHANNELの見出しが復活していることを確認する
/// （`<panels>`/`<panel>`統一後は他の見出しと同じ`ui.horizontal`ラップになる）。
/// CHIP LFOパネルは2026-08-20のCHIP LFO完全退役でXMLから削除済み。
#[test]
fn channel_title_present() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("ui.label(egui::RichText::new(\"CHANNEL\").strong());"));
}

/// `<header><title/>...`（空タグ）が親の`title=`属性から見出しを解決することを確認する。
/// インデント込みで一致を見るのは生成レイアウトのネスト段数（ScrollArea/group/vertical/
/// horizontal）を暗黙に固定してしまうため、行ごとに`trim()`して緩く照合する。
///
/// 質感LFO退役（2026-08-21）でPITCH FG等の`<header>`から`<jack>`が消え、`<title/>`のみに
/// なった。パッチベイ機構自体（`ui-core::patchbay`）は将来のOP/フィードバック配線の布石として
/// 温存する方針のため、`xml_with_jack_keeps_patchbay_output`（インラインXMLフィクスチャ、
/// `time_eg_editor_and_root_attrs.rs`）がjack生成コードパスの回帰を引き続き担保する。
#[test]
fn header_title_from_attr_resolves() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    let lines: Vec<&str> = rust.lines().map(str::trim).collect();
    let idx = lines
        .iter()
        .position(|l| *l == "ui.label(egui::RichText::new(\"PITCH FG\").strong());")
        .expect("PITCH FGの見出し行が見つかりません");
    assert_eq!(lines[idx - 1], "ui.horizontal(|ui| {");
    assert_eq!(lines[idx + 1], "});");
}

/// XMLに書かれたパネル（OP repeat + CHANNEL(span12) + FILTER +
/// PITCH FG + CUTOFF FG + GAIN FG + MASTER EFFECT）が全て生成されている
/// （要素の脱落がない）ことを確認する。CHIP LFOパネルは2026-08-20のCHIP LFO完全退役、
/// TEXTURE LFOパネル（jack機構含む）は2026-08-21の質感LFO完全退役でXMLから削除済み
/// （GAIN FGパネルのMASTER/OPチェックボックス・各FGパネルのTEXTUREドロップダウンが厳密代替）。
/// MASTER EFFECTはREVERB/CHORUSの2パネル構成だったが、レイアウト調整で1パネルに統合された。
#[test]
fn all_panels_present() {
    let xml = panel_xml();
    let rust = ui_codegen::generate_rust(&xml).unwrap();
    assert!(rust.contains("for (i_row, i_chunk) in params.operators.chunks(2).enumerate()"));
    assert!(rust.contains("\"PITCH FG\""));
    assert!(rust.contains("\"CUTOFF FG\""));
    assert!(rust.contains("\"GAIN FG\""));
    assert!(rust.contains("\"MASTER EFFECT\""));
    assert!(rust.contains("params.gain_fg_to_master"));
    assert!(rust.contains("params.gain_fg_to_operators"));
    assert!(!rust.contains("\"TEXTURE LFO\""), "TEXTURE LFOパネルは質感LFO退役で削除済みのはず");
    assert!(!rust.contains("finish_texture_lfo_patchbay"), "panel.xmlにjackが無いためpatchbay呼び出しは生成されないはず");
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
