//! XML(`panel.xml`) → [`crate::ir`]。
//!
//! `tools/xml-panel-dsl/index.html`のJSパーサー（`parseLayout`〜`buildLeafInfo`）の機械的移植から
//! さらに一歩進め、`<raw>`頼みだった見出し・派生値表示・条件グレーアウトを閉じた語彙
//! （`<title>`/`<readout>`/`enabled-if`）で表現する。エラーメッセージも極力そのまま踏襲する。

use crate::ir::*;
use roxmltree::Node;

struct Ctx {
    base: String,
    /// リピートループの変数名（既定"i"）。`enabled-if`の述語や`<title>`の`{index+N}`が参照する。
    index: String,
    /// 親`<panel>`/`<column>`の`title=`属性値（`<title/>`の空タグ解決用）。
    title: String,
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

fn eg_field(el: Node, ctx: &Ctx, name: &str) -> Result<EgField, String> {
    if let Some(v) = el.attribute(name) {
        Ok(EgField::Handle(resolve_path(v, ctx)))
    } else if let Some(v) = el.attribute(format!("{name}-value").as_str()) {
        Ok(EgField::Literal(v.to_string()))
    } else {
        Err(format!("<eg-preview>に{name}属性(または{name}-value)が必要です"))
    }
}

/// `enabled-if="[!]<述語名>"`を解決する。述語名は閉じた語彙（現状`is_carrier`のみ）。
fn parse_enabled_if(raw: &str, ctx: &Ctx) -> Result<Predicate, String> {
    let (negate, name) = match raw.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let compute = match name {
        "is_carrier" => Compute::IsCarrier { index_var: ctx.index.clone() },
        other => return Err(format!("未知のenabled-if述語です: {other}（現状is_carrierのみ対応）")),
    };
    Ok(Predicate { compute, negate })
}

/// `<readout compute="..." args="a,b" format="..." tooltip="...">`を解決する。
/// `compute`は閉じた語彙（現状`mul-fine-ratio`のみ）。
fn parse_readout(el: Node, ctx: &Ctx) -> Result<Readout, String> {
    let compute_name = req_attr(el, "compute")?;
    let args_raw = req_attr(el, "args")?;
    let args: Vec<&str> = args_raw.split(',').map(|s| s.trim()).collect();
    let compute = match compute_name.as_str() {
        "mul-fine-ratio" => {
            if args.len() != 2 {
                return Err("<readout compute=\"mul-fine-ratio\">のargsは2個(mul,fine)必要です".to_string());
            }
            Compute::MulFineRatio { mul: resolve_path(args[0], ctx), fine: resolve_path(args[1], ctx) }
        }
        other => return Err(format!("未知のcompute値です: {other}（現状mul-fine-ratioのみ対応）")),
    };
    let format = req_attr(el, "format")?;
    let tooltip = el.attribute("tooltip").map(|s| s.to_string());
    Ok(Readout { compute, format, tooltip })
}

/// `<title>`（テキスト、または`{index+N}`を1箇所だけ含む動的テンプレート）を解決する。
/// 空（`<title/>`）なら親の`title=`属性を使う。
fn parse_title(el: Node, ctx: &Ctx) -> Result<Title, String> {
    let text = text_content(el).trim().to_string();
    if text.is_empty() {
        if ctx.title.is_empty() {
            return Err("<title/>は空にできません(親のtitle=属性も未設定です)".to_string());
        }
        return Ok(Title::Static(ctx.title.clone()));
    }
    if let Some(start) = text.find("{index+") {
        let rest = &text[start + "{index+".len()..];
        let end = rest
            .find('}')
            .ok_or_else(|| "<title>の{index+N}が}で閉じられていません".to_string())?;
        let n: i32 = rest[..end]
            .parse()
            .map_err(|_| "<title>の{index+N}のNは整数で指定してください".to_string())?;
        let before = text[..start].to_string();
        let after = rest[end + 1..].to_string();
        return Ok(Title::Dynamic { before, offset: n, after, index_var: ctx.index.clone() });
    }
    Ok(Title::Static(text))
}

fn build_leaf_info(el: Node, ctx: &Ctx) -> Result<LeafInfo, String> {
    let tag = el.tag_name().name();
    let enabled_if = match el.attribute("enabled-if") {
        Some(raw) => Some(parse_enabled_if(raw, ctx)?),
        None => None,
    };

    let (widget, preview_label, preview_type, size): (Widget, String, String, Size) = match tag {
        "knob" => {
            let label = req_attr(el, "label")?;
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            (Widget::Knob { label: label.clone(), handle }, label, "knob".to_string(), Size { w: 62.0, h: 66.0 })
        }
        "checkbox" => {
            let label = req_attr(el, "label")?;
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            (
                Widget::Checkbox { label: label.clone(), handle },
                label,
                "checkbox".to_string(),
                Size { w: 70.0, h: 66.0 },
            )
        }
        "checkbox-stack" => {
            let mut items: Vec<(String, String)> = Vec::new();
            for c in element_children(el) {
                if c.tag_name().name() != "checkbox" {
                    return Err("<checkbox-stack>の子要素は<checkbox>のみです".to_string());
                }
                let label = req_attr(c, "label")?;
                let handle = resolve_path(&req_attr(c, "handle")?, ctx);
                items.push((label, handle));
            }
            if items.is_empty() {
                return Err("<checkbox-stack>には1個以上<checkbox>が必要です".to_string());
            }
            let preview_label = items.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>().join("/");
            (Widget::CheckboxStack { items }, preview_label, "checkbox-stack".to_string(), Size { w: 70.0, h: 66.0 })
        }
        "waveform" => {
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let index = el.attribute("index").unwrap_or("0").to_string();
            (
                Widget::Waveform { handle, index },
                "WAVE".to_string(),
                "waveform".to_string(),
                Size { w: 130.0, h: 66.0 },
            )
        }
        "enum" => {
            let label = req_attr(el, "label")?;
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let names = req_attr(el, "names")?;
            let salt = el.attribute("salt").unwrap_or("0").to_string();
            (
                Widget::Enum { label: label.clone(), handle, names, salt },
                label,
                "enum".to_string(),
                Size { w: 100.0, h: 66.0 },
            )
        }
        "eg-preview" => {
            let mapping = el.attribute("mapping").unwrap_or("DbLinear").to_string();
            let widget = Widget::EgPreview {
                mapping,
                tl: eg_field(el, ctx, "tl")?,
                ar: eg_field(el, ctx, "ar")?,
                d1r: eg_field(el, ctx, "d1r")?,
                d1l: eg_field(el, ctx, "d1l")?,
                d2r: eg_field(el, ctx, "d2r")?,
                rr: eg_field(el, ctx, "rr")?,
                floor: eg_field(el, ctx, "floor")?,
                loop_enabled: eg_field(el, ctx, "loop")?,
                curve: eg_field(el, ctx, "curve")?,
                delay: eg_field(el, ctx, "delay")?,
            };
            (widget, "EG".to_string(), "eg-preview".to_string(), Size { w: 84.0, h: 66.0 })
        }
        "algorithm-diagram" => {
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            (
                Widget::AlgorithmDiagram { handle },
                "ALG".to_string(),
                "algorithm-diagram".to_string(),
                Size { w: 150.0, h: 100.0 },
            )
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
            (Widget::Raw(code), "raw".to_string(), "raw".to_string(), Size { w, h })
        }
        other => return Err(format!("<row>/<stack>内で未知の要素です: <{other}>")),
    };

    Ok(LeafInfo { size, widget, enabled_if, preview_label, preview_type })
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
            "header" => {
                let mut items = Vec::new();
                for c in element_children(child) {
                    match c.tag_name().name() {
                        "title" => items.push(HeaderItem::Title(parse_title(c, ctx)?)),
                        "readout" => items.push(HeaderItem::Readout(parse_readout(c, ctx)?)),
                        "jack" => items.push(HeaderItem::Jack(jack_attrs(c, ctx)?)),
                        "raw" => items.push(HeaderItem::Raw(text_content(c).trim().to_string())),
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

/// bodyに既に見出し（`<header>`または自動挿入前の`<title>`相当）が無ければ、
/// `title=`属性から見出しを自動挿入する。`is_bare`が`true`なら`BodyStmt::Title`（裸のラベル、
/// `<column>`向け）、`false`なら`BodyStmt::Header`（横並びラップ、`<panel>`向け）で挿入する。
fn auto_insert_title(body: &mut Vec<BodyStmt>, title: &str, is_bare: bool) {
    if title.is_empty() {
        return;
    }
    let has_heading = body.iter().any(|s| matches!(s, BodyStmt::Header { .. } | BodyStmt::Title(_)));
    if has_heading {
        return;
    }
    let stmt = if is_bare {
        BodyStmt::Title(Title::Static(title.to_string()))
    } else {
        BodyStmt::Header { items: vec![HeaderItem::Title(Title::Static(title.to_string()))] }
    };
    body.insert(0, stmt);
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
    let ctx = Ctx { base, index: index.clone(), title: title.clone() };
    let mut body = parse_body(el, &ctx)?;
    auto_insert_title(&mut body, &title, false);
    Ok(Panel { repeat, as_, index, title, body })
}

fn parse_columns(el: Node) -> Result<Columns, String> {
    let match_height = el.attribute("match-height") == Some("true");
    let mut cols = Vec::new();
    for c in element_children(el) {
        if c.tag_name().name() != "column" {
            return Err("<columns>の子要素は<column>のみです".to_string());
        }
        let width: f32 = c.attribute("width").unwrap_or("1").parse().unwrap_or(1.0);
        let title = c.attribute("title").unwrap_or("").to_string();
        let ctx = Ctx { base: "params".to_string(), index: "i".to_string(), title: title.clone() };
        let mut body = parse_body(c, &ctx)?;
        auto_insert_title(&mut body, &title, true);
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
