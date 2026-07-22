//! XML(`panel.xml`) → [`crate::ir`]。
//!
//! `tools/xml-panel-dsl/index.html`のJSパーサー（`parseLayout`〜`buildLeafInfo`）の機械的移植。
//! エラーメッセージも極力そのまま踏襲する（ブラウザツール利用時と同じ文言で気付けるように）。

use crate::ir::*;
use roxmltree::Node;

struct Ctx {
    base: String,
}

fn resolve_path(raw: &str, ctx: &Ctx) -> String {
    if raw.contains('.') {
        raw.to_string()
    } else {
        format!("{}.{}", ctx.base, raw)
    }
}

fn element_children<'a, 'input>(el: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    el.children().filter(|n| n.is_element()).collect()
}

fn text_content(el: Node) -> String {
    el.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect::<Vec<_>>().join("")
}

fn req_attr(el: Node, name: &str) -> Result<String, String> {
    el.attribute(name)
        .map(|s| s.to_string())
        .ok_or_else(|| format!("<{}> に {} 属性が必要です", el.tag_name().name(), name))
}

fn eg_field(el: Node, ctx: &Ctx, name: &str) -> Result<String, String> {
    if let Some(v) = el.attribute(name) {
        Ok(format!("{}.value() as u8", resolve_path(v, ctx)))
    } else if let Some(v) = el.attribute(format!("{name}-value").as_str()) {
        Ok(v.to_string())
    } else {
        Err(format!("<eg-preview>に{name}属性(または{name}-value)が必要です"))
    }
}

fn build_leaf_info(el: Node, ctx: &Ctx) -> Result<LeafInfo, String> {
    let tag = el.tag_name().name();
    let enabled_if = el.attribute("enabled-if").map(|s| s.to_string());
    let wrap = |stmt: String| -> String {
        match &enabled_if {
            Some(cond) => format!("ui.add_enabled_ui({cond}, |ui| {{ {stmt} }});"),
            None => stmt,
        }
    };

    match tag {
        "knob" => {
            let label = req_attr(el, "label")?;
            let path = resolve_path(&req_attr(el, "handle")?, ctx);
            Ok(LeafInfo {
                size: Size { w: 62.0, h: 66.0 },
                rust_stmt: wrap(format!("knob(ui, &*{path}, \"{label}\");")),
                preview_label: label,
                preview_type: "knob".to_string(),
            })
        }
        "checkbox" => {
            let label = req_attr(el, "label")?;
            let path = resolve_path(&req_attr(el, "handle")?, ctx);
            Ok(LeafInfo {
                size: Size { w: 70.0, h: 66.0 },
                rust_stmt: wrap(format!("bool_checkbox(ui, &*{path}, \"{label}\");")),
                preview_label: label,
                preview_type: "checkbox".to_string(),
            })
        }
        "checkbox-stack" => {
            let mut items: Vec<(String, String)> = Vec::new();
            for c in element_children(el) {
                if c.tag_name().name() != "checkbox" {
                    return Err("<checkbox-stack>の子要素は<checkbox>のみです".to_string());
                }
                let label = req_attr(c, "label")?;
                let path = resolve_path(&req_attr(c, "handle")?, ctx);
                items.push((label.clone(), format!("bool_checkbox(ui, &*{path}, \"{label}\");")));
            }
            if items.is_empty() {
                return Err("<checkbox-stack>には1個以上<checkbox>が必要です".to_string());
            }
            let stmt = format!(
                "ui.vertical(|ui| {{ {} }});",
                items.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>().join(" ")
            );
            Ok(LeafInfo {
                size: Size { w: 70.0, h: 66.0 },
                rust_stmt: wrap(stmt),
                preview_label: items.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>().join("/"),
                preview_type: "checkbox-stack".to_string(),
            })
        }
        "waveform" => {
            let path = resolve_path(&req_attr(el, "handle")?, ctx);
            let index = el.attribute("index").unwrap_or("0");
            Ok(LeafInfo {
                size: Size { w: 130.0, h: 66.0 },
                rust_stmt: wrap(format!("waveform_selector(ui, &*{path}, ({index}) as usize);")),
                preview_label: "WAVE".to_string(),
                preview_type: "waveform".to_string(),
            })
        }
        "enum" => {
            let label = req_attr(el, "label")?;
            let path = resolve_path(&req_attr(el, "handle")?, ctx);
            let names = req_attr(el, "names")?;
            let salt = el.attribute("salt").unwrap_or("0");
            Ok(LeafInfo {
                size: Size { w: 100.0, h: 66.0 },
                rust_stmt: wrap(format!("enum_selector(ui, &*{path}, \"{label}\", &{names}, {salt});")),
                preview_label: label,
                preview_type: "enum".to_string(),
            })
        }
        "eg-preview" => {
            let mapping = el.attribute("mapping").unwrap_or("DbLinear");
            let tl_expr = eg_field(el, ctx, "tl")?;
            let eg_expr = format!(
                "EgParams {{ ar: {}, d1r: {}, d1l: {}, d2r: {}, rr: {}, floor: {}, loop_enabled: {}, curve: {}, delay: {} }}",
                eg_field(el, ctx, "ar")?,
                eg_field(el, ctx, "d1r")?,
                eg_field(el, ctx, "d1l")?,
                eg_field(el, ctx, "d2r")?,
                eg_field(el, ctx, "rr")?,
                eg_field(el, ctx, "floor")?,
                eg_field(el, ctx, "loop")?,
                eg_field(el, ctx, "curve")?,
                eg_field(el, ctx, "delay")?,
            );
            Ok(LeafInfo {
                size: Size { w: 84.0, h: 66.0 },
                rust_stmt: wrap(format!("eg_preview(ui, EgAmplitudeMapping::{mapping}, {tl_expr}, {eg_expr});")),
                preview_label: "EG".to_string(),
                preview_type: "eg-preview".to_string(),
            })
        }
        "algorithm-diagram" => {
            let path = resolve_path(&req_attr(el, "handle")?, ctx);
            Ok(LeafInfo {
                size: Size { w: 150.0, h: 100.0 },
                rust_stmt: wrap(format!("algorithm_diagram(ui, {path}.value() as u8);")),
                preview_label: "ALG".to_string(),
                preview_type: "algorithm-diagram".to_string(),
            })
        }
        "raw" => {
            let w: f32 = req_attr(el, "width")?
                .parse()
                .map_err(|_| "<raw>のwidth/heightは数値で指定してください".to_string())?;
            let h: f32 = req_attr(el, "height")?
                .parse()
                .map_err(|_| "<raw>のwidth/heightは数値で指定してください".to_string())?;
            let code = text_content(el).trim().to_string();
            if code.is_empty() {
                return Err("<raw>は空にできません".to_string());
            }
            Ok(LeafInfo {
                size: Size { w, h },
                rust_stmt: wrap(code),
                preview_label: "raw".to_string(),
                preview_type: "raw".to_string(),
            })
        }
        other => Err(format!("<row>/<stack>内で未知の要素です: <{other}>")),
    }
}

fn build_row_tree(el: Node, ctx: &Ctx) -> Result<TreeNode, String> {
    let tag = el.tag_name().name();
    if tag == "row" || tag == "stack" {
        let gap = match el.attribute("gap") {
            None => Gap::Fixed(0.0),
            Some("spacing") => Gap::Spacing,
            Some(v) => Gap::Fixed(
                v.parse()
                    .map_err(|_| format!("<{tag}>のgapは数値または\"spacing\"で指定してください"))?,
            ),
        };
        let grow = el.attribute("grow") == Some("true");
        let justify = el.attribute("justify").unwrap_or("start").to_string();
        if tag == "stack" && grow {
            return Err("<stack grow=\"true\">は現時点で未対応です（layout.rsにstack_growが無い）".to_string());
        }
        let kids = element_children(el);
        if kids.is_empty() {
            return Err(format!("<{tag}>には1個以上の子要素が必要です"));
        }
        let children = kids
            .into_iter()
            .map(|c| build_row_tree(c, ctx))
            .collect::<Result<Vec<_>, _>>()?;
        if tag == "row" {
            Ok(TreeNode::Row { justify, gap, grow, children })
        } else {
            Ok(TreeNode::Stack { gap, grow, children })
        }
    } else {
        Ok(TreeNode::Leaf(build_leaf_info(el, ctx)?))
    }
}

fn jack_attrs(el: Node, ctx: &Ctx) -> Result<Jack, String> {
    let kind = el.attribute("kind").unwrap_or("dest");
    if kind == "source" {
        return Ok(Jack::Source);
    }
    let dest_index = req_attr(el, "dest-index")?;
    let label = req_attr(el, "label")?;
    let handle = resolve_path(&req_attr(el, "handle")?, ctx);
    Ok(Jack::Dest { dest_index, label, handle })
}

fn parse_body(el: Node, ctx: &Ctx) -> Result<Vec<BodyStmt>, String> {
    let mut stmts = Vec::new();
    for child in element_children(el) {
        match child.tag_name().name() {
            "let" => stmts.push(BodyStmt::Let {
                name: req_attr(child, "name")?,
                expr: req_attr(child, "expr")?,
            }),
            "header" => {
                let mut items = Vec::new();
                for c in element_children(child) {
                    match c.tag_name().name() {
                        "raw" => items.push(HeaderItem::Raw(text_content(c).trim().to_string())),
                        "jack" => items.push(HeaderItem::Jack(jack_attrs(c, ctx)?)),
                        other => return Err(format!("<header>内で未知の要素です: <{other}>")),
                    }
                }
                stmts.push(BodyStmt::Header { items });
            }
            "row" | "stack" => stmts.push(BodyStmt::Tree(build_row_tree(child, ctx)?)),
            "space" => {
                let size: f32 = req_attr(child, "size")?
                    .parse()
                    .map_err(|_| "<space>のsizeは数値で指定してください".to_string())?;
                stmts.push(BodyStmt::Space { size });
            }
            "jack" => stmts.push(BodyStmt::Jack(jack_attrs(child, ctx)?)),
            "raw" => stmts.push(BodyStmt::Raw(text_content(child).trim().to_string())),
            other => return Err(format!("<panel>/<column>直下で未知の要素です: <{other}>")),
        }
    }
    Ok(stmts)
}

fn parse_panel(el: Node) -> Result<Panel, String> {
    let repeat = el.attribute("repeat").map(|s| s.to_string());
    let as_ = el.attribute("as").map(|s| s.to_string());
    if repeat.is_some() && as_.is_none() {
        return Err("<panel repeat=\"...\">にはas属性が必要です".to_string());
    }
    let index = el.attribute("index").unwrap_or("i").to_string();
    let title = el.attribute("title").unwrap_or("").to_string();
    let base = if repeat.is_some() { as_.clone().unwrap() } else { "params".to_string() };
    let ctx = Ctx { base };
    let body = parse_body(el, &ctx)?;
    Ok(Panel { repeat, as_, index, title, body })
}

fn parse_columns(el: Node) -> Result<Columns, String> {
    let match_height = el.attribute("match-height") == Some("true");
    let ctx = Ctx { base: "params".to_string() };
    let mut cols = Vec::new();
    for c in element_children(el) {
        if c.tag_name().name() != "column" {
            return Err("<columns>の子要素は<column>のみです".to_string());
        }
        let width: f32 = c.attribute("width").unwrap_or("1").parse().unwrap_or(1.0);
        let title = c.attribute("title").unwrap_or("").to_string();
        let body = parse_body(c, &ctx)?;
        cols.push(Column { width, title, body });
    }
    if cols.len() < 2 {
        return Err("<columns>には<column>が2個以上必要です".to_string());
    }
    Ok(Columns { match_height, columns: cols })
}

/// `panel.xml`の全文をパースし、[`Layout`]を返す。
pub fn parse_layout(xml_text: &str) -> Result<Layout, String> {
    let doc = roxmltree::Document::parse(xml_text).map_err(|e| format!("XML構文エラー: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "layout" {
        return Err("ルート要素は<layout>である必要があります".to_string());
    }
    let mut items = Vec::new();
    for el in element_children(root) {
        match el.tag_name().name() {
            "panel" => items.push(Item::Panel(parse_panel(el)?)),
            "columns" => items.push(Item::Columns(parse_columns(el)?)),
            other => return Err(format!("<layout>直下で未知の要素です: <{other}>")),
        }
    }
    if items.is_empty() {
        return Err("<layout>には1個以上の<panel>/<columns>が必要です".to_string());
    }
    Ok(Layout { items })
}
