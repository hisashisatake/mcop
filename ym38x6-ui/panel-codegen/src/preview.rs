//! ブラウザプレビュー用のJSON境界。
//!
//! `<row>`/`<stack>`の矩形計算だけを本物の`panel_layout::solve`（taffy）に委ね、
//! パネル/カラムの縦積み位置計算・SVG描画そのものはブラウザ側JS（`tools/xml-panel-dsl/index.html`）
//! に残す設計。ここでは
//! 1. [`parse_ir_preview`]: XML→JS側が必要とする構造（body文の種類・`<row>`/<stack>木・葉のラベル種別）
//! 2. [`solve_tree_json`]: (1)が返した木のJSONをそのまま渡すと、本物taffyで解いた矩形配列を返す
//! の2つだけを提供する。

use crate::ir::*;
use crate::parse::parse_layout;
use serde_json::{json, Value};

fn gap_to_json(gap: &Gap) -> Value {
    match gap {
        Gap::Spacing => json!("spacing"),
        Gap::Fixed(v) => json!(v),
    }
}

fn tree_node_to_json(node: &TreeNode) -> Value {
    match node {
        TreeNode::Leaf(l) => json!({ "kind": "leaf", "w": l.size.w, "h": l.size.h }),
        TreeNode::Row { justify, gap, grow, children } => json!({
            "kind": "row",
            "justify": justify,
            "gap": gap_to_json(gap),
            "grow": grow,
            "children": children.iter().map(tree_node_to_json).collect::<Vec<_>>(),
        }),
        TreeNode::Stack { gap, grow, children } => json!({
            "kind": "stack",
            "gap": gap_to_json(gap),
            "grow": grow,
            "children": children.iter().map(tree_node_to_json).collect::<Vec<_>>(),
        }),
    }
}

fn stmt_to_json(st: &BodyStmt) -> Value {
    match st {
        BodyStmt::Raw(_) => json!({ "kind": "spacer" }),
        BodyStmt::Title(_) => json!({ "kind": "header", "has_jack": false }),
        BodyStmt::Header { items } => json!({
            "kind": "header",
            "has_jack": items.iter().any(|i| matches!(i, HeaderItem::Jack(_))),
        }),
        BodyStmt::Space { size } => json!({ "kind": "space", "size": size }),
        BodyStmt::Jack(_) => json!({ "kind": "jack" }),
        BodyStmt::Tree(node) => json!({
            "kind": "tree",
            "max_height": node.max_height(),
            "node": tree_node_to_json(node),
            "leaves": node.leaves().iter().map(|l| json!({
                "preview_label": l.preview_label,
                "preview_type": l.preview_type,
            })).collect::<Vec<_>>(),
        }),
    }
}

fn body_to_json(body: &[BodyStmt]) -> Value {
    Value::Array(body.iter().map(stmt_to_json).collect())
}

fn item_to_json(item: &Item) -> Value {
    match item {
        Item::Panel(p) => json!({
            "kind": "panel",
            "repeat": p.repeat,
            "title": p.title,
            "body": body_to_json(&p.body),
        }),
        Item::Columns(c) => json!({
            "kind": "columns",
            "columns": c.columns.iter().map(|col| json!({
                "width": col.width,
                "title": col.title,
                "body": body_to_json(&col.body),
            })).collect::<Vec<_>>(),
        }),
    }
}

/// XML全文をパースし、ブラウザ側プレビュー描画に必要な構造をJSONで返す。
/// `tree`種別の`node`フィールドは[`solve_tree_json`]にそのまま渡せる形。
pub fn parse_ir_preview(xml: &str) -> Result<String, String> {
    let layout = parse_layout(xml)?;
    let value = json!({ "items": layout.items.iter().map(item_to_json).collect::<Vec<_>>() });
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

fn json_to_layout_node(v: &Value) -> Result<panel_layout::Node, String> {
    let kind = v["kind"].as_str().ok_or("木ノードJSONに'kind'がありません")?;
    match kind {
        "leaf" => {
            let w = v["w"].as_f64().ok_or("leafノードに'w'がありません")? as f32;
            let h = v["h"].as_f64().ok_or("leafノードに'h'がありません")? as f32;
            Ok(panel_layout::leaf(w, h))
        }
        "row" | "stack" => {
            let gap = match &v["gap"] {
                Value::String(s) if s == "spacing" => 8.0,
                Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
                _ => 0.0,
            };
            let children = v["children"]
                .as_array()
                .ok_or("row/stackノードに'children'がありません")?
                .iter()
                .map(json_to_layout_node)
                .collect::<Result<Vec<_>, _>>()?;
            if kind == "stack" {
                return Ok(panel_layout::stack(gap, children));
            }
            let grow = v["grow"].as_bool().unwrap_or(false);
            let justify = match v["justify"].as_str().unwrap_or("start") {
                "start" => panel_layout::Justify::Start,
                "between" => panel_layout::Justify::Between,
                "around" => panel_layout::Justify::Around,
                "evenly" => panel_layout::Justify::Evenly,
                "center" => panel_layout::Justify::Center,
                "end" => panel_layout::Justify::End,
                _ => panel_layout::Justify::Start,
            };
            Ok(if grow {
                panel_layout::row_grow(justify, gap, children)
            } else {
                panel_layout::row(justify, gap, children)
            })
        }
        other => Err(format!("未知の木ノード種別です: {other}")),
    }
}

/// [`parse_ir_preview`]が返した`tree.node`のJSONを本物のtaffyで解決し、
/// 葉の矩形（DFS順、`{x,y,w,h}`の配列）をJSONで返す。
pub fn solve_tree_json(node_json: &str, container_w: f32, container_h: f32) -> Result<String, String> {
    let value: Value = serde_json::from_str(node_json).map_err(|e| e.to_string())?;
    let node = json_to_layout_node(&value)?;
    let rects = panel_layout::solve(panel_layout::Vec2::new(container_w, container_h), &node);
    let out: Vec<Value> = rects.into_iter().map(|r| json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h })).collect();
    serde_json::to_string(&out).map_err(|e| e.to_string())
}
