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
    /// 親`<panel>`の`title=`属性値（`<title/>`の空タグ解決用）。
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

/// 真偽属性を読む。省略時は`default`。`grow`/`match-height`は`== Some("true")`の素朴な比較だが、
/// こちらは`terminal-level`と同様に不正値をエラーにする（表示の正しさに関わる属性でタイポが
/// 黙って無視されると、ノブの数値だけが期待と違うという追いにくい症状になるため）。
fn attr_bool_or(el: Node, name: &str, default: bool) -> Result<bool, String> {
    match el.attribute(name) {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(format!(
            "<{}>の{name}はtrue/falseで指定してください: {other}",
            el.tag_name().name()
        )),
    }
}

/// `width`/`height`属性を数値として読む。省略時は`default`を使う。
fn attr_f32_or(el: Node, name: &str, default: f32) -> Result<f32, String> {
    match el.attribute(name) {
        Some(v) => v.parse().map_err(|_| format!("<{}>の{name}は数値で指定してください", el.tag_name().name())),
        None => Ok(default),
    }
}

/// CSSショートハンド記法のマージン値をパースする（`"4"`=4辺/`"4 8"`=上下・左右/`"4 8 6 2"`=上右下左）。
fn parse_margin(raw: &str) -> Result<Margin, String> {
    let nums: Vec<f32> = raw
        .split_whitespace()
        .map(|p| p.parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("marginは数値で指定してください: \"{raw}\""))?;
    match nums.len() {
        1 => Ok(Margin::same(nums[0])),
        2 => Ok(Margin { top: nums[0], bottom: nums[0], left: nums[1], right: nums[1] }),
        4 => Ok(Margin { top: nums[0], right: nums[1], bottom: nums[2], left: nums[3] }),
        n => Err(format!("marginは1個(4辺)/2個(上下 左右)/4個(上 右 下 左)の数値で指定してください（{n}個指定されました）: \"{raw}\"")),
    }
}

/// パネルのマージン（`egui::Margin`がi8ベースのため-128〜127の整数のみ許可）をパースする。
fn parse_panel_margin(raw: &str) -> Result<Margin, String> {
    let m = parse_margin(raw)?;
    for v in [m.top, m.right, m.bottom, m.left] {
        if v.fract() != 0.0 || !(-128.0..=127.0).contains(&v) {
            return Err(format!("パネルのマージンは-128〜127の整数で指定してください: \"{raw}\""));
        }
    }
    Ok(m)
}

/// `<style>`（`<layout>`直下、省略可・1個まで）をパースする。
fn parse_style(el: Node) -> Result<Style, String> {
    let mut style = Style::default();
    for c in element_children(el) {
        match c.tag_name().name() {
            "panels" => {
                if let Some(v) = c.attribute("gap") {
                    style.panels_gap =
                        v.parse().map_err(|_| "<style><panels>のgapは数値で指定してください".to_string())?;
                }
            }
            "panel" => {
                if let Some(v) = c.attribute("inner-margin") {
                    style.panel_inner_margin = parse_panel_margin(v)?;
                }
                if let Some(v) = c.attribute("outer-margin") {
                    style.panel_outer_margin = parse_panel_margin(v)?;
                }
            }
            "widget" => {
                if let Some(v) = c.attribute("margin") {
                    style.widget_margin = parse_margin(v)?;
                }
            }
            "eg-preview" => {
                style.eg_preview_size.w = attr_f32_or(c, "width", style.eg_preview_size.w)?;
                style.eg_preview_size.h = attr_f32_or(c, "height", style.eg_preview_size.h)?;
                if let Some(v) = c.attribute("margin") {
                    style.tag_margin.insert("eg-preview".to_string(), parse_margin(v)?);
                }
            }
            "algorithm-diagram" => {
                style.algorithm_diagram_size.w = attr_f32_or(c, "width", style.algorithm_diagram_size.w)?;
                style.algorithm_diagram_size.h = attr_f32_or(c, "height", style.algorithm_diagram_size.h)?;
                if let Some(v) = c.attribute("margin") {
                    style.tag_margin.insert("algorithm-diagram".to_string(), parse_margin(v)?);
                }
            }
            "time-eg-editor" => {
                style.time_eg_editor_size.w = attr_f32_or(c, "width", style.time_eg_editor_size.w)?;
                style.time_eg_editor_size.h = attr_f32_or(c, "height", style.time_eg_editor_size.h)?;
                if let Some(v) = c.attribute("margin") {
                    style.tag_margin.insert("time-eg-editor".to_string(), parse_margin(v)?);
                }
            }
            tag @ ("knob" | "checkbox" | "waveform" | "enum" | "raw") => {
                if let Some(v) = c.attribute("margin") {
                    style.tag_margin.insert(tag.to_string(), parse_margin(v)?);
                }
            }
            other => return Err(format!("<style>内で未知の要素です: <{other}>")),
        }
    }
    Ok(style)
}

/// leafウィジェット共通の`margin`属性を3段カスケード（インスタンス属性→`<style>`タグ別→
/// `<style><widget>`既定値）で解決する。
fn resolve_widget_margin(el: Node, tag: &str, style: &Style) -> Result<Margin, String> {
    match el.attribute("margin") {
        Some(v) => parse_margin(v),
        None => Ok(style.tag_margin.get(tag).copied().unwrap_or(style.widget_margin)),
    }
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

fn build_leaf_info(el: Node, ctx: &Ctx, style: &Style) -> Result<LeafInfo, String> {
    let tag = el.tag_name().name();
    let enabled_if = match el.attribute("enabled-if") {
        Some(raw) => Some(parse_enabled_if(raw, ctx)?),
        None => None,
    };
    let margin = resolve_widget_margin(el, tag, style)?;

    let (widget, preview_label, preview_type, size): (Widget, String, String, Size) = match tag {
        "knob" => {
            let label = req_attr(el, "label")?;
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let bipolar = attr_bool_or(el, "bipolar", false)?;
            (
                Widget::Knob { label: label.clone(), handle, bipolar },
                label,
                "knob".to_string(),
                // `ui_core::knob::KNOB_CELL_SIZE`と一致させること（ui-codegenはegui非依存で
                // あちらの定数を参照できないため手で同期する）。幅62はスピン行の実幅
                // （ボタン12×2＋間隔2×2＋数値欄32＝60）がちょうど収まる値。
                Size { w: 62.0, h: 66.0 },
            )
        }
        "checkbox" => {
            let label = req_attr(el, "label")?;
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            // 既定幅70は"CURVE"ラベル基準の実測値（bool_checkboxはallocate_exact_sizeせず内容依存幅の
            // ため）。"SELF-OSC"のような長いラベルは既定幅だと折り返されるため、`width`属性で
            // インスタンスごとに上書きできる。
            // 高さ20はegui既定`interact_size.y`(18.0)に対する実測ベースの近似値。<row>直下の単独配置では
            // 隣接ウィジェット(knob等66px)がRowのmax_heightを支配するため実害なく、<stack>直下では
            // この実高さのおかげでN個重ねても行高さが66pxのまま膨張しない。
            let width = attr_f32_or(el, "width", 70.0)?;
            (
                Widget::Checkbox { label: label.clone(), handle },
                label,
                "checkbox".to_string(),
                Size { w: width, h: 20.0 },
            )
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
        "sync-rate" => {
            let label = req_attr(el, "label")?;
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let salt = el.attribute("salt").unwrap_or("0").to_string();
            (
                Widget::SyncRate { label: label.clone(), handle, salt },
                label,
                "sync-rate".to_string(),
                // <enum>(100×66)の置き換え用。ノブ(28px)＋ComboBoxで縦は+4pxに収めてある
                // （FGパネルの<stack>は<time-eg-editor>の固定高さ245pxが上限）。
                Size { w: 100.0, h: 70.0 },
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
            let w = attr_f32_or(el, "width", style.eg_preview_size.w)?;
            let h = attr_f32_or(el, "height", style.eg_preview_size.h)?;
            (widget, "EG".to_string(), "eg-preview".to_string(), Size { w, h })
        }
        "algorithm-diagram" => {
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let w = attr_f32_or(el, "width", style.algorithm_diagram_size.w)?;
            let h = attr_f32_or(el, "height", style.algorithm_diagram_size.h)?;
            (Widget::AlgorithmDiagram { handle }, "ALG".to_string(), "algorithm-diagram".to_string(), Size { w, h })
        }
        "time-eg-editor" => {
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let mapping = el.attribute("mapping").unwrap_or("DbLinear").to_string();
            let tl = eg_field(el, ctx, "tl")?;
            let w = attr_f32_or(el, "width", style.time_eg_editor_size.w)?;
            let h = attr_f32_or(el, "height", style.time_eg_editor_size.h)?;
            let min_stages: u8 = match el.attribute("min-stages") {
                None => 2,
                Some(v) => v
                    .parse()
                    .ok()
                    .filter(|n| (1..=8).contains(n))
                    .ok_or_else(|| "<time-eg-editor>のmin-stagesは1〜8で指定してください".to_string())?,
            };
            let terminal_level_zero = match el.attribute("terminal-level").unwrap_or("zero") {
                "zero" => true,
                "free" => false,
                other => return Err(format!("<time-eg-editor>のterminal-levelはzero/freeのいずれかです: {other}")),
            };
            (
                Widget::TimeEgEditor { handle, mapping, tl, min_stages, terminal_level_zero },
                "EG".to_string(),
                "time-eg-editor".to_string(),
                Size { w, h },
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

    Ok(LeafInfo { size, margin, widget, enabled_if, preview_label, preview_type })
}

fn build_row_tree(el: Node, ctx: &Ctx, style: &Style) -> Result<TreeNode, String> {
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
        let kids = element_children(el);
        if kids.is_empty() {
            return Err(format!("<{tag}>には1個以上の子要素が必要です"));
        }
        let children = kids
            .into_iter()
            .map(|c| build_row_tree(c, ctx, style))
            .collect::<Result<Vec<_>, _>>()?;
        if tag == "row" {
            Ok(TreeNode::Row { justify, gap, grow, children })
        } else {
            Ok(TreeNode::Stack { gap, grow, children })
        }
    } else {
        Ok(TreeNode::Leaf(build_leaf_info(el, ctx, style)?))
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

fn parse_body(el: Node, ctx: &Ctx, style: &Style) -> Result<Vec<BodyStmt>, String> {
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
            "row" | "stack" => stmts.push(BodyStmt::Tree(build_row_tree(child, ctx, style)?)),
            "space" => {
                let size: f32 = req_attr(child, "size")?
                    .parse()
                    .map_err(|_| "<space>のsizeは数値で指定してください".to_string())?;
                stmts.push(BodyStmt::Space { size });
            }
            "jack" => stmts.push(BodyStmt::Jack(jack_attrs(child, ctx)?)),
            "raw" => stmts.push(BodyStmt::Raw(text_content(child).trim().to_string())),
            other => return Err(format!("<panel>直下で未知の要素です: <{other}>")),
        }
    }
    Ok(stmts)
}

/// bodyに既に見出し（`<header>`相当）が無ければ、`title=`属性から見出しを自動挿入する
/// （横並びラップ、`ui.horizontal`）。
fn auto_insert_title(body: &mut Vec<BodyStmt>, title: &str) {
    if title.is_empty() {
        return;
    }
    let has_heading = body.iter().any(|s| matches!(s, BodyStmt::Header { .. }));
    if has_heading {
        return;
    }
    body.insert(0, BodyStmt::Header { items: vec![HeaderItem::Title(Title::Static(title.to_string()))] });
}

/// `<panel>`をパースする。`span`属性は生の文字列のみ読み取り、まだ解決しない
/// （`<panels>`内の他の`<panel>`のspanが出揃ってから[`resolve_spans`]でグループ単位に解決する）。
fn parse_panel(el: Node, style: &Style) -> Result<(Panel, Option<u32>), String> {
    let repeat = el.attribute("repeat").map(|s| s.to_string());
    let as_ = el.attribute("as").map(|s| s.to_string());
    if repeat.is_some() && as_.is_none() {
        return Err("<panel repeat=\"...\">にはas属性が必要です".to_string());
    }
    let index = el.attribute("index").unwrap_or("i").to_string();
    let title = el.attribute("title").unwrap_or("").to_string();
    let span_raw = match el.attribute("span") {
        Some(v) => Some(
            v.parse::<u32>().map_err(|_| "<panel>のspanは0〜12の整数で指定してください".to_string())?,
        ),
        None => None,
    };
    let columns = match el.attribute("columns") {
        Some(v) => {
            let n = v.parse::<usize>().map_err(|_| "<panel>のcolumnsは1以上の整数で指定してください".to_string())?;
            if n == 0 {
                return Err("<panel>のcolumnsは1以上の整数で指定してください".to_string());
            }
            if repeat.is_none() {
                return Err("<panel columns=\"...\">はrepeat属性と併用してください".to_string());
            }
            Some(n)
        }
        None => None,
    };
    let base = if repeat.is_some() { as_.clone().unwrap() } else { "params".to_string() };
    let ctx = Ctx { base, index: index.clone(), title: title.clone() };
    let mut body = parse_body(el, &ctx, style)?;
    auto_insert_title(&mut body, &title);
    Ok((Panel { repeat, as_, index, title, span: 0, columns, body }, span_raw))
}

/// `<panels>`内の各`<panel>`の`span`を解決する。CSSの12カラムグリッド（Bootstrap等）を参考にした規約:
/// - 全`<panel>`が`span`省略 → 12を均等割り（12が要素数で割り切れない場合はエラー、明示指定を促す）
/// - 1個でも`span`が指定されていれば、**全`<panel>`に明示指定が必要**（一部省略は不可）で、
///   合計はちょうど12でなければならない（書き漏れ等のミスをパースエラーで検出するため）
fn resolve_spans(panels: &mut [Panel], spans_raw: &[Option<u32>]) -> Result<(), String> {
    let n = panels.len();
    if spans_raw.iter().all(|s| s.is_none()) {
        if 12 % n as u32 != 0 {
            return Err(format!(
                "<panels>内の<panel>が{n}個ではspanを12で均等割りできません。各<panel>にspan=\"...\"を明示してください"
            ));
        }
        let even = 12 / n as u32;
        for p in panels.iter_mut() {
            p.span = even;
        }
        return Ok(());
    }
    if spans_raw.iter().any(|s| s.is_none()) {
        return Err(
            "<panels>内で一部の<panel>だけspanを省略することはできません（全て明示するか、全て省略するかのどちらかにしてください）"
                .to_string(),
        );
    }
    let total: u32 = spans_raw.iter().map(|s| s.unwrap()).sum();
    if total != 12 {
        return Err(format!("<panels>内の<panel>のspan合計は12である必要があります（現在の合計: {total}）"));
    }
    for (p, s) in panels.iter_mut().zip(spans_raw.iter()) {
        p.span = s.unwrap();
    }
    Ok(())
}

fn parse_panels_group(el: Node, style: &Style) -> Result<PanelsGroup, String> {
    let match_height = el.attribute("match-height") == Some("true");
    let mut panels = Vec::new();
    let mut spans_raw = Vec::new();
    for c in element_children(el) {
        if c.tag_name().name() != "panel" {
            return Err("<panels>の子要素は<panel>のみです".to_string());
        }
        let (panel, span_raw) = parse_panel(c, style)?;
        panels.push(panel);
        spans_raw.push(span_raw);
    }
    if panels.is_empty() {
        return Err("<panels>には<panel>が1個以上必要です".to_string());
    }
    resolve_spans(&mut panels, &spans_raw)?;
    Ok(PanelsGroup { match_height, panels })
}

/// `panel.xml`の全文をパースし、[`Layout`]を返す。
pub fn parse_layout(xml_text: &str) -> Result<Layout, String> {
    let doc = roxmltree::Document::parse(xml_text).map_err(|e| format!("XML構文エラー: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "layout" {
        return Err("ルート要素は<layout>である必要があります".to_string());
    }
    let children = element_children(root);
    let style_elements: Vec<Node> = children.iter().filter(|c| c.tag_name().name() == "style").copied().collect();
    if style_elements.len() > 1 {
        return Err("<style>は<layout>直下に1個までです".to_string());
    }
    let style = match style_elements.first() {
        Some(el) => parse_style(*el)?,
        None => Style::default(),
    };
    let mut groups = Vec::new();
    for el in children {
        match el.tag_name().name() {
            "style" => {} // 上でパース済み
            "panels" => groups.push(parse_panels_group(el, &style)?),
            other => return Err(format!("<layout>直下で未知の要素です: <{other}>（<panels>で囲んでください）")),
        }
    }
    if groups.is_empty() {
        return Err("<layout>には1個以上の<panels>が必要です".to_string());
    }
    let fn_name = root.attribute("fn").unwrap_or("draw_param_panel").to_string();
    let params_type = root.attribute("params-type").unwrap_or("PanelParams").to_string();
    let scroll_id = root.attribute("scroll-id").map(|s| s.to_string());
    Ok(Layout { groups, style, fn_name, params_type, scroll_id })
}

#[cfg(test)]
mod span_tests {
    use super::parse_layout;

    fn wrap(inner: &str) -> String {
        format!("<layout><panels>{inner}</panels></layout>")
    }

    #[test]
    fn single_panel_defaults_span_to_12() {
        let xml = wrap(r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
        let layout = parse_layout(&xml).unwrap();
        assert_eq!(layout.groups[0].panels[0].span, 12);
    }

    #[test]
    fn two_panels_without_span_split_evenly() {
        let xml = wrap(
            r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        assert_eq!(layout.groups[0].panels[0].span, 6);
        assert_eq!(layout.groups[0].panels[1].span, 6);
    }

    #[test]
    fn explicit_spans_summing_to_12_are_kept() {
        let xml = wrap(
            r#"<panel span="4" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel span="8" title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        assert_eq!(layout.groups[0].panels[0].span, 4);
        assert_eq!(layout.groups[0].panels[1].span, 8);
    }

    #[test]
    fn explicit_spans_not_summing_to_12_is_error() {
        let xml = wrap(
            r#"<panel span="4" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel span="4" title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("合計は12"), "{err}");
    }

    #[test]
    fn mixing_explicit_and_omitted_span_is_error() {
        let xml = wrap(
            r#"<panel span="4" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("一部の<panel>だけ"), "{err}");
    }

    #[test]
    fn omitted_span_not_evenly_divisible_is_error() {
        let xml = wrap(
            r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel title="B"><row><knob label="Y" handle="y"/></row></panel>
               <panel title="C"><row><knob label="Z" handle="z"/></row></panel>
               <panel title="D"><row><knob label="W" handle="w"/></row></panel>
               <panel title="E"><row><knob label="V" handle="v"/></row></panel>"#,
        );
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("均等割りできません"), "{err}");
    }

    #[test]
    fn bare_panel_at_layout_root_is_rejected() {
        let xml = r#"<layout><panel title="A"><row><knob label="X" handle="x"/></row></panel></layout>"#;
        let err = parse_layout(xml).unwrap_err();
        assert!(err.contains("<panels>で囲んでください"), "{err}");
    }

    #[test]
    fn columns_without_repeat_is_rejected() {
        let xml = wrap(r#"<panel title="A" columns="2"><row><knob label="X" handle="x"/></row></panel>"#);
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("repeat属性と併用"), "{err}");
    }

    #[test]
    fn columns_zero_is_rejected() {
        let xml = wrap(
            r#"<panel repeat="operators" as="op" columns="0"><row><knob label="X" handle="op.x"/></row></panel>"#,
        );
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("1以上の整数"), "{err}");
    }

    #[test]
    fn columns_on_repeat_panel_is_kept() {
        let xml = wrap(
            r#"<panel repeat="operators" as="op" columns="2"><row><knob label="X" handle="op.x"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        assert_eq!(layout.groups[0].panels[0].columns, Some(2));
    }
}

#[cfg(test)]
mod style_tests {
    use super::parse_layout;
    use crate::ir::{BodyStmt, TreeNode};

    fn wrap(style: &str, inner: &str) -> String {
        format!("<layout>{style}<panels>{inner}</panels></layout>")
    }

    /// `<panel title="...">`は`<header>`省略時に自動でタイトル見出し(`BodyStmt::Header`)が
    /// 先頭へ挿入される（`auto_insert_title`）ため、`<row>`は常にbody[1]に入る。
    fn first_row_children(xml: &str) -> Vec<TreeNode> {
        let layout = parse_layout(xml).unwrap();
        let BodyStmt::Tree(TreeNode::Row { children, .. }) = &layout.groups[0].panels[0].body[1] else {
            panic!("<row>が見つかりません")
        };
        children.clone()
    }

    #[test]
    fn no_style_uses_defaults() {
        let xml = wrap("", r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
        let layout = parse_layout(&xml).unwrap();
        assert_eq!(layout.style.panels_gap, 8.0);
        assert_eq!(layout.style.panel_inner_margin.top, 6.0);
        assert_eq!(layout.style.panel_outer_margin.top, 0.0);
        assert_eq!(layout.style.widget_margin.top, 0.0);
        assert_eq!(layout.style.eg_preview_size.w, 84.0);
        assert_eq!(layout.style.algorithm_diagram_size.w, 150.0);
    }

    #[test]
    fn style_overrides_defaults() {
        let style = r#"<style>
            <panels gap="12"/>
            <panel inner-margin="4" outer-margin="0 10 0 0"/>
            <widget margin="2"/>
            <eg-preview width="120" height="90"/>
            <algorithm-diagram width="200" height="130"/>
        </style>"#;
        let xml = wrap(style, r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
        let layout = parse_layout(&xml).unwrap();
        assert_eq!(layout.style.panels_gap, 12.0);
        assert_eq!(layout.style.panel_inner_margin.top, 4.0);
        assert_eq!(layout.style.panel_outer_margin.right, 10.0);
        assert_eq!(layout.style.panel_outer_margin.top, 0.0);
        assert_eq!(layout.style.widget_margin.top, 2.0);
        assert_eq!(layout.style.eg_preview_size.w, 120.0);
        assert_eq!(layout.style.eg_preview_size.h, 90.0);
        assert_eq!(layout.style.algorithm_diagram_size.w, 200.0);
    }

    #[test]
    fn margin_shorthand_forms() {
        assert_eq!(super::parse_margin("4").unwrap().top, 4.0);
        assert_eq!(super::parse_margin("4").unwrap().left, 4.0);
        let m2 = super::parse_margin("4 8").unwrap();
        assert_eq!((m2.top, m2.bottom, m2.left, m2.right), (4.0, 4.0, 8.0, 8.0));
        let m4 = super::parse_margin("1 2 3 4").unwrap();
        assert_eq!((m4.top, m4.right, m4.bottom, m4.left), (1.0, 2.0, 3.0, 4.0));
        assert!(super::parse_margin("1 2 3").is_err());
    }

    #[test]
    fn panel_margin_rejects_non_integer() {
        let style = r#"<style><panel inner-margin="4.5"/></style>"#;
        let xml = wrap(style, r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("整数"), "{err}");
    }

    #[test]
    fn second_style_tag_is_rejected() {
        let xml = wrap(
            "<style/><style/>",
            r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#,
        );
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("1個までです"), "{err}");
    }

    #[test]
    fn widget_margin_cascade_instance_wins_over_tag_wins_over_default() {
        // <style>既定=1、<knob>タグ別上書き=2、インスタンス属性=3。優先順位はインスタンス→タグ→既定。
        let style = r#"<style><widget margin="1"/><knob margin="2"/></style>"#;
        let xml = wrap(
            style,
            r#"<panel title="A"><row>
                <knob label="A" handle="a"/>
                <knob label="B" handle="b" margin="3"/>
                <checkbox label="C" handle="c"/>
            </row></panel>"#,
        );
        let children = first_row_children(&xml);
        let TreeNode::Leaf(knob_tag_default) = &children[0] else { panic!("leaf not found") };
        let TreeNode::Leaf(knob_instance) = &children[1] else { panic!("leaf not found") };
        let TreeNode::Leaf(checkbox_widget_default) = &children[2] else { panic!("leaf not found") };
        assert_eq!(knob_tag_default.margin.top, 2.0); // <knob>タグ別上書き
        assert_eq!(knob_instance.margin.top, 3.0); // インスタンス属性が最優先
        assert_eq!(checkbox_widget_default.margin.top, 1.0); // <widget>既定へフォールバック
    }

    #[test]
    fn eg_preview_instance_width_overrides_style_default() {
        let xml = wrap(
            "",
            r#"<panel title="A"><row>
                <eg-preview width="200" ar="ar" d1r="d1r" d1l="d1l" d2r="d2r" rr="rr"
                  tl="tl" floor="floor" loop="op_loop" curve="curve" delay-value="0"/>
            </row></panel>"#,
        );
        let children = first_row_children(&xml);
        let TreeNode::Leaf(leaf) = &children[0] else { panic!("leaf not found") };
        assert_eq!(leaf.size.w, 200.0);
        assert_eq!(leaf.size.h, 66.0); // heightは省略したのでstyle既定値のまま
    }
}
