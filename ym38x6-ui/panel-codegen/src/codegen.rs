//! [`crate::ir`] → Rustソース文字列。
//!
//! `tools/xml-panel-dsl/index.html`のJSコード生成（`genPanel`〜`generateRust`）の機械的移植。
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
        lines.push(format!(
            "layout::place(ui, origin, *r.next().unwrap(), |ui| {{ {} }});",
            leaf.rust_stmt
        ));
    }
    lines
}

fn gen_jack_stmt(j: &Jack) -> String {
    match j {
        Jack::Source => "crate::patchbay::texture_lfo_source_jack(ui, &mut tx_jacks);".to_string(),
        Jack::Dest { dest_index, label, handle } => format!(
            "crate::patchbay::texture_lfo_dest_jack(ui, &*{handle}, {dest_index}, \"{label}\", &mut tx_jacks);"
        ),
    }
}

fn gen_body_lines(body: &[BodyStmt]) -> Vec<String> {
    let mut lines = Vec::new();
    for st in body {
        match st {
            BodyStmt::Let { name, expr } => lines.push(format!("let {name} = {expr};")),
            BodyStmt::Header { items } => {
                let inner = items
                    .iter()
                    .map(|i| match i {
                        HeaderItem::Raw(code) => code.clone(),
                        HeaderItem::Jack(j) => gen_jack_stmt(j),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
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

fn gen_panel(p: &Panel) -> String {
    let body_lines = gen_body_lines(&p.body);
    let mut inner = vec![
        "ui.group(|ui| {".to_string(),
        "    ui.set_width(panel_width);".to_string(),
        "    ui.vertical(|ui| {".to_string(),
    ];
    for l in &body_lines {
        inner.push(indent(l, 8));
    }
    inner.push("    });".to_string());
    inner.push("});".to_string());
    match &p.repeat {
        None => inner.join("\n"),
        Some(repeat) => {
            let as_ = p.as_.as_deref().unwrap_or("");
            let mut out = vec![format!("for ({}, {as_}) in params.{repeat}.iter().enumerate() {{", p.index)];
            for l in &inner {
                out.push(indent(l, 4));
            }
            out.push("}".to_string());
            out.join("\n")
        }
    }
}

fn gen_columns(c: &Columns) -> String {
    let n = c.columns.len();
    let total_ratio: f32 = c.columns.iter().map(|col| col.width).sum();
    let mut out = Vec::new();
    out.push(format!("let columns_usable = full_width - inter_gap * {};", fmt_num(n as f32 - 1.0)));
    for (i, col) in c.columns.iter().enumerate() {
        out.push(format!(
            "let col_w_{i} = columns_usable * {} / {};",
            fmt_num(col.width),
            fmt_num(total_ratio)
        ));
    }
    out.push("ui.horizontal(|ui| {".to_string());
    for (i, col) in c.columns.iter().enumerate() {
        let body_lines = gen_body_lines(&col.body);
        if i == 0 {
            out.push("    let col0_resp = ui.group(|ui| {".to_string());
            out.push("        ui.set_width(col_w_0);".to_string());
            out.push("        ui.vertical(|ui| {".to_string());
            for l in &body_lines {
                out.push(indent(l, 12));
            }
            out.push("        });".to_string());
            out.push("    });".to_string());
            if c.match_height {
                out.push("    let match_height = col0_resp.response.rect.height();".to_string());
            }
        } else {
            out.push("    ui.group(|ui| {".to_string());
            out.push(format!("        ui.set_width(col_w_{i});"));
            if c.match_height {
                out.push("        ui.set_min_height(match_height);".to_string());
            }
            out.push("        ui.vertical(|ui| {".to_string());
            for l in &body_lines {
                out.push(indent(l, 12));
            }
            out.push("        });".to_string());
            out.push("    });".to_string());
        }
    }
    out.push("});".to_string());
    out.join("\n")
}

/// [`Layout`]全体から`draw_param_panel`関数（本体アイテムのみ）を生成する。
pub fn generate_rust(layout: &Layout) -> String {
    let parts: Vec<String> = layout
        .items
        .iter()
        .map(|it| match it {
            Item::Panel(p) => gen_panel(p),
            Item::Columns(c) => gen_columns(c),
        })
        .collect();
    let body = indent(&parts.join("\n\n"), 8);
    format!(
        "pub fn draw_param_panel(ui: &mut egui::Ui, params: &PanelParams) {{\n    \
         let mut tx_jacks = crate::patchbay::JackLayout::new();\n    \
         egui::ScrollArea::vertical().show(ui, |ui| {{\n        \
         let full_width = ui.available_width();\n        \
         let inter_gap = ui.spacing().item_spacing.x;\n        \
         let panel_width = full_width - inter_gap;\n\n\
         {body}\n\n        \
         crate::patchbay::finish_texture_lfo_patchbay(ui, &*params.texture_lfo_destination, tx_jacks);\n    \
         }});\n\
         }}\n"
    )
}
