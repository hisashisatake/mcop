//! [`crate::ir`] → Rustソース文字列。
//!
//! 出力は`draw_param_panel`関数の**本体アイテムのみ**（`use`文は含まない）。
//! `ym38x6-ui/build.rs`が`include!`する生成物、およびブラウザツールの
//! Rust出力タブの表示内容として、この1関数だけが唯一の生成対象になる
//! （`panel.rs`側の構造体定義・ヘルパー関数・`use`は手書きのまま）。

use crate::ir::*;

fn fmt_num(n: f32) -> String {
    if n == n.trunc() {
        format!("{n:.1}")
    } else {
        format!("{n}")
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn indent(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    text.split('\n')
        .map(|l| if l.is_empty() { l.to_string() } else { format!("{pad}{l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

fn gap_str(gap: &Gap) -> String {
    match gap {
        Gap::Spacing => "outer_gap".to_string(),
        Gap::Fixed(v) => fmt_num(*v),
    }
}

/// 名前付き派生計算をRust式へ展開する（`Compute`のvariantごとに1本）。
fn compute_expr(c: &Compute) -> String {
    match c {
        Compute::IsCarrier { index_var } => {
            format!("crate::algorithm_diagram::carriers(params.algorithm.value() as u8).contains(&{index_var})")
        }
        Compute::MulFineRatio { mul, fine } => {
            format!("mul_fine_ratio({mul}.value() as u8, {fine}.value() as u8)")
        }
    }
}

fn eg_field_expr(f: &EgField) -> String {
    match f {
        EgField::Handle(h) => format!("{h}.value() as u8"),
        EgField::Literal(v) => v.clone(),
    }
}

/// [`Widget`]をRust文（`layout::place`のクロージャ内に置かれる1文）へ変換する。
fn gen_widget_stmt(w: &Widget) -> String {
    match w {
        Widget::Knob { label, handle } => format!("knob(ui, &*{handle}, \"{label}\");"),
        Widget::Checkbox { label, handle } => format!("bool_checkbox(ui, &*{handle}, \"{label}\");"),
        Widget::CheckboxStack { items } => format!(
            "ui.vertical(|ui| {{ {} }});",
            items
                .iter()
                .map(|(label, handle)| format!("bool_checkbox(ui, &*{handle}, \"{label}\");"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Widget::Waveform { handle, index } => format!("waveform_selector(ui, &*{handle}, ({index}) as usize);"),
        Widget::Enum { label, handle, names, salt } => {
            format!("enum_selector(ui, &*{handle}, \"{label}\", &{names}, {salt});")
        }
        Widget::EgPreview { mapping, tl, ar, d1r, d1l, d2r, rr, floor, loop_enabled, curve, delay } => format!(
            "eg_preview(ui, EgAmplitudeMapping::{mapping}, {}, EgParams {{ ar: {}, d1r: {}, d1l: {}, d2r: {}, rr: {}, floor: {}, loop_enabled: {}, curve: {}, delay: {} }});",
            eg_field_expr(tl),
            eg_field_expr(ar),
            eg_field_expr(d1r),
            eg_field_expr(d1l),
            eg_field_expr(d2r),
            eg_field_expr(rr),
            eg_field_expr(floor),
            eg_field_expr(loop_enabled),
            eg_field_expr(curve),
            eg_field_expr(delay),
        ),
        Widget::AlgorithmDiagram { handle } => format!("algorithm_diagram(ui, {handle}.value() as u8);"),
        Widget::Raw(code) => code.clone(),
    }
}

fn wrap_enabled_if(stmt: String, pred: &Option<Predicate>) -> String {
    match pred {
        None => stmt,
        Some(p) => {
            let cond = compute_expr(&p.compute);
            let cond = if p.negate { format!("!{cond}") } else { cond };
            format!("ui.add_enabled_ui({cond}, |ui| {{ {stmt} }});")
        }
    }
}

fn gen_title_stmt(t: &Title) -> String {
    match t {
        Title::Static(s) => format!("ui.label(egui::RichText::new(\"{s}\").strong());"),
        Title::Dynamic { before, offset, after, index_var } => format!(
            "ui.label(egui::RichText::new(format!(\"{before}{{}}{after}\", {index_var} + {offset})).strong());"
        ),
    }
}

fn gen_readout_stmt(r: &Readout) -> String {
    let value_expr = compute_expr(&r.compute);
    // DSLの`{value...}`トークンをRustの位置引数プレースホルダ`{...}`へ変換する
    // （`format`属性の値はそれ自体がRustのフォーマット構文なので、変換はトークン名の除去だけでよい）。
    let fmt = r.format.replace("{value", "{");
    let tooltip = r.tooltip.as_ref().map(|t| format!(".on_hover_text(\"{t}\")")).unwrap_or_default();
    format!("ui.label(egui::RichText::new(format!(\"{fmt}\", {value_expr})).size(10.0).weak()){tooltip};")
}

fn gen_tree_block(node: &TreeNode) -> Vec<String> {
    let mut lines = Vec::new();
    if tree_uses_spacing(node) {
        lines.push("let outer_gap = ui.spacing().item_spacing.x;".to_string());
    }
    let h = fmt_num(node.max_height());
    lines.push("let avail = ui.available_width();".to_string());
    lines.push(format!("let tree = {};", tree_expr(node)));
    lines.push(format!("let rects = layout::solve(egui::vec2(avail, {h}), &tree);"));
    lines.push("let origin = ui.cursor().min;".to_string());
    lines.push(format!("ui.allocate_space(egui::vec2(avail, {h}));"));
    lines.push("let mut r = rects.iter();".to_string());
    for leaf in node.leaves() {
        let stmt = wrap_enabled_if(gen_widget_stmt(&leaf.widget), &leaf.enabled_if);
        lines.push(format!("layout::place(ui, origin, *r.next().unwrap(), |ui| {{ {stmt} }});"));
    }
    lines
}

fn tree_expr(node: &TreeNode) -> String {
    match node {
        TreeNode::Leaf(l) => format!("leaf({}, {})", fmt_num(l.size.w), fmt_num(l.size.h)),
        TreeNode::Row { justify, gap, grow, children } => {
            let ctor = if *grow { "row_grow" } else { "row" };
            let children_str = children.iter().map(tree_expr).collect::<Vec<_>>().join(", ");
            format!("{ctor}(Justify::{}, {}, vec![{children_str}])", capitalize(justify), gap_str(gap))
        }
        TreeNode::Stack { gap, children, .. } => {
            let children_str = children.iter().map(tree_expr).collect::<Vec<_>>().join(", ");
            format!("stack({}, vec![{children_str}])", gap_str(gap))
        }
    }
}

fn tree_uses_spacing(node: &TreeNode) -> bool {
    match node {
        TreeNode::Leaf(_) => false,
        TreeNode::Row { gap, children, .. } | TreeNode::Stack { gap, children, .. } => {
            matches!(gap, Gap::Spacing) || children.iter().any(tree_uses_spacing)
        }
    }
}

fn gen_jack_stmt(j: &Jack) -> String {
    match j {
        Jack::Source => "crate::patchbay::texture_lfo_source_jack(ui, &mut tx_jacks);".to_string(),
        Jack::Dest { dest_index, label, handle } => format!(
            "crate::patchbay::texture_lfo_dest_jack(ui, &*{handle}, {dest_index}, \"{label}\", &mut tx_jacks);"
        ),
    }
}

fn gen_header_item_stmt(item: &HeaderItem) -> String {
    match item {
        HeaderItem::Title(t) => gen_title_stmt(t),
        HeaderItem::Readout(r) => gen_readout_stmt(r),
        HeaderItem::Jack(j) => gen_jack_stmt(j),
        HeaderItem::Raw(code) => code.clone(),
    }
}

fn gen_body_lines(body: &[BodyStmt]) -> Vec<String> {
    let mut lines = Vec::new();
    for st in body {
        match st {
            BodyStmt::Header { items } => {
                let inner = items.iter().map(gen_header_item_stmt).collect::<Vec<_>>().join("\n");
                lines.push(format!("ui.horizontal(|ui| {{\n{}\n}});", indent(&inner, 4)));
            }
            BodyStmt::Tree(tree) => lines.extend(gen_tree_block(tree)),
            BodyStmt::Space { size } => lines.push(format!("ui.add_space({});", fmt_num(*size))),
            BodyStmt::Jack(j) => lines.push(gen_jack_stmt(j)),
            BodyStmt::Raw(code) => lines.push(code.clone()),
        }
    }
    lines
}

/// 1個の`<panel>`（`ui.group`本体、`repeat`があればfor文でラップ）を生成する。
/// `width_expr`はこのパネルが占める幅を表すRust式（変数名）。`match_height`が`true`かつ
/// このパネルがグループ先頭（`capture_height`）なら`let resp = ...`で高さを捕捉し、
/// 先頭以外は捕捉済みの`match_height`変数へ`set_min_height`する
/// （`<panels match-height="true">`、グループが1個の`<panel>`のみの場合は実質無効）。
fn gen_panel(p: &Panel, width_expr: &str, match_height: bool, capture_height: bool) -> Vec<String> {
    let body_lines = gen_body_lines(&p.body);
    let group_stmt = if capture_height { "let group_resp = ui.group(|ui| {" } else { "ui.group(|ui| {" };
    let mut inner = vec![group_stmt.to_string(), format!("    ui.set_width({width_expr});")];
    if match_height && !capture_height {
        inner.push("    ui.set_min_height(match_height);".to_string());
    }
    inner.push("    ui.vertical(|ui| {".to_string());
    for l in &body_lines {
        inner.push(indent(l, 8));
    }
    inner.push("    });".to_string());
    inner.push("});".to_string());
    if capture_height && match_height {
        inner.push("let match_height = group_resp.response.rect.height();".to_string());
    }
    match &p.repeat {
        None => inner,
        Some(repeat) => {
            let as_ = p.as_.as_deref().unwrap_or("");
            let mut out = vec![format!("for ({}, {as_}) in params.{repeat}.iter().enumerate() {{", p.index)];
            for l in &inner {
                out.push(indent(l, 4));
            }
            out.push("}".to_string());
            out
        }
    }
}

/// `<panels>`（`<panel>`を1個以上持つ、12分割グリッドの1行）を生成する。
/// 幅計算はspan/12ベース（`<panel>`のspan合計は必ず12、parse.rsで検証済み）。
/// gapは列間の(n-1)個ぶんのみ引く。ただしn=1（単独パネル行）でも従来通り右マージン
/// 1個ぶん(inter_gap)を引くため、gap本数は`max(n-1, 1)`とする
/// （n=1: usable=full_width-inter_gap、n=2: usable=full_width-inter_gapで、
/// どちらも旧`gen_panel`/旧`gen_columns`と数値上完全に一致する）。
///
/// **n=1のとき`ui.horizontal`ではラップしない**（`repeat`ありパネルだと、for文全体が
/// 1個の`ui.horizontal`クロージャに閉じ込められ、本来ScrollArea直下で縦積みされるべき
/// 複数回の`ui.group`が横並びになってしまう実バグを踏んだため。n>1の複数カラム行だけ
/// `ui.horizontal`で横並びにする）。
fn gen_panels_group(g: &PanelsGroup) -> String {
    let n = g.panels.len();
    let gap_count = if n <= 1 { 1 } else { n - 1 };
    let mut out = vec![format!("let usable = full_width - inter_gap * {};", fmt_num(gap_count as f32))];
    for (i, p) in g.panels.iter().enumerate() {
        out.push(format!("let w_{i} = usable * {} / 12.0;", fmt_num(p.span as f32)));
    }
    if n == 1 {
        out.extend(gen_panel(&g.panels[0], "w_0", false, false));
        return out.join("\n");
    }
    out.push("ui.horizontal(|ui| {".to_string());
    for (i, p) in g.panels.iter().enumerate() {
        let width_expr = format!("w_{i}");
        let capture_height = i == 0 && g.match_height;
        let lines = gen_panel(p, &width_expr, g.match_height, capture_height);
        for l in &lines {
            out.push(indent(l, 4));
        }
    }
    out.push("});".to_string());
    out.join("\n")
}

/// [`Layout`]全体から`draw_param_panel`関数（本体アイテムのみ）を生成する。
pub fn generate_rust(layout: &Layout) -> String {
    let parts: Vec<String> = layout.groups.iter().map(gen_panels_group).collect();
    let body = indent(&parts.join("\n\n"), 8);
    format!(
        "pub fn draw_param_panel(ui: &mut egui::Ui, params: &PanelParams) {{\n    \
         let mut tx_jacks = crate::patchbay::JackLayout::new();\n    \
         egui::ScrollArea::vertical().show(ui, |ui| {{\n        \
         let full_width = ui.available_width();\n        \
         let inter_gap = ui.spacing().item_spacing.x;\n\n\
         {body}\n\n        \
         crate::patchbay::finish_texture_lfo_patchbay(ui, &*params.texture_lfo_destination, tx_jacks);\n    \
         }});\n\
         }}\n"
    )
}
