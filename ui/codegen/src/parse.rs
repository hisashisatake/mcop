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
            "level-meter" => {
                style.level_meter_size.w = attr_f32_or(c, "width", style.level_meter_size.w)?;
                style.level_meter_size.h = attr_f32_or(c, "height", style.level_meter_size.h)?;
                if let Some(v) = c.attribute("margin") {
                    style.tag_margin.insert("level-meter".to_string(), parse_margin(v)?);
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
    let mut enabled_if = match el.attribute("enabled-if") {
        Some(raw) => Some(parse_enabled_if(raw, ctx)?),
        None => None,
    };
    let margin = resolve_widget_margin(el, tag, style)?;

    let (widget, preview_label, preview_type, size): (Widget, String, String, Size) = match tag {
        "knob" => {
            let label = req_attr(el, "label")?;
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let bipolar = attr_bool_or(el, "bipolar", false)?;
            // alt-label/alt-handle（V.GAIN/VELのような役割入れ替え2パラメーターの合体指定）は
            // enabled-ifと必ずセットで使う。通常のenabled-if（グレーアウトのみ）とは違い、
            // この述語の結果でleaf自体のenabled_ifではなくWidget::Knob.altの述語として消費し、
            // 下のwrap_enabled_ifによる二重ラップを避ける（dual_knobが自前で出し分けるため）。
            let alt_label = el.attribute("alt-label");
            let alt_handle = el.attribute("alt-handle");
            let alt = match (alt_label, alt_handle) {
                (Some(alt_label), Some(alt_handle)) => {
                    let predicate = enabled_if
                        .take()
                        .ok_or_else(|| "<knob alt-label alt-handle>にはenabled-if属性も必要です".to_string())?;
                    Some(KnobAlt {
                        label: alt_label.to_string(),
                        handle: resolve_path(alt_handle, ctx),
                        predicate,
                    })
                }
                (None, None) => None,
                _ => {
                    return Err("<knob>のalt-label/alt-handleは両方指定してください".to_string());
                }
            };
            (
                Widget::Knob { label: label.clone(), handle, bipolar, alt },
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
            // 既定66.0は<knob>とのRow内揃え用。<stack>縦積み時は`height`属性で個別に縮められる
            // （中身はラベル+ComboBoxのみで66pxより実際にはずっと低いため、既定のままだと
            // 下に大きな余白が残る。<checkbox width="...">と同じ前例踏襲）。
            let height = attr_f32_or(el, "height", 66.0)?;
            (
                Widget::Enum { label: label.clone(), handle, names, salt, height },
                label,
                "enum".to_string(),
                Size { w: 100.0, h: height },
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
        "level-meter" => {
            let label = el.attribute("label").unwrap_or("LEVEL").to_string();
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let w = attr_f32_or(el, "width", style.level_meter_size.w)?;
            let h = attr_f32_or(el, "height", style.level_meter_size.h)?;
            (
                Widget::LevelMeter { label: label.clone(), handle },
                label,
                "level-meter".to_string(),
                Size { w, h },
            )
        }
        "time-eg-editor" => {
            let handle = resolve_path(&req_attr(el, "handle")?, ctx);
            let mapping = el.attribute("mapping").unwrap_or("DbLinear").to_string();
            let tl = eg_field(el, ctx, "tl")?;
            let w = attr_f32_or(el, "width", style.time_eg_editor_size.w)?;
            let h = attr_f32_or(el, "height", style.time_eg_editor_size.h)?;
            // 上限10は`sound_core::time_eg::MAX_STAGES`とのリテラル二重管理（ui-codegenは
            // egui/sound-core非依存の純粋クレートという設計を守るため、依存を足さずリテラルで持つ）。
            // 下限0はFG専用の特殊値（無効化。`ui_core::TimeEgProfile::min_stages`のdoc参照）。
            let min_stages: u8 = match el.attribute("min-stages") {
                None => 2,
                Some(v) => v
                    .parse()
                    .ok()
                    .filter(|n| (0..=10).contains(n))
                    .ok_or_else(|| "<time-eg-editor>のmin-stagesは0〜10で指定してください".to_string())?,
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
        // <stack center="true">は子の幅が揃わないとき（<knob>62pxと<enum>100px混在等）、
        // 幅の狭い子をスタック幅（最大幅の子）の中央へ揃える。<row>には無関係（rowのjustifyは
        // 主軸=横方向の分配で、centerは既に別の意味を持つため属性を分けてある）。
        let center = el.attribute("center") == Some("true");
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
            Ok(TreeNode::Stack { gap, grow, center, children })
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

/// `<panel span="...">`の生の指定形態。整数（12分割グリッド）とパーセントは
/// 同じ`<panels>`内では混在できない（[`resolve_spans`]で検証）。
#[derive(Clone, Copy, Debug)]
enum SpanRaw {
    /// `span="4"`（12分割グリッドでのマス数）。
    Grid(u32),
    /// `span="33%"`（直接の割合、0〜100）。
    Percent(f32),
}

fn parse_span_raw(v: &str) -> Result<SpanRaw, String> {
    if let Some(pct) = v.strip_suffix('%') {
        let p: f32 = pct
            .parse()
            .map_err(|_| format!("<panel>のspanは0〜12の整数か\"N%\"のパーセントで指定してください: \"{v}\""))?;
        if !(0.0..=100.0).contains(&p) {
            return Err(format!("<panel>のspanのパーセント指定は0〜100の範囲で指定してください: \"{v}\""));
        }
        Ok(SpanRaw::Percent(p))
    } else {
        let n: u32 = v
            .parse()
            .map_err(|_| format!("<panel>のspanは0〜12の整数か\"N%\"のパーセントで指定してください: \"{v}\""))?;
        Ok(SpanRaw::Grid(n))
    }
}

/// `<panel>`をパースする。`span`属性は生の文字列のみ読み取り、まだ解決しない
/// （`<panels>`内の他の`<panel>`のspanが出揃ってから[`resolve_spans`]でグループ単位に解決する）。
fn parse_panel(el: Node, style: &Style) -> Result<(Panel, Option<SpanRaw>), String> {
    let repeat = el.attribute("repeat").map(|s| s.to_string());
    let as_ = el.attribute("as").map(|s| s.to_string());
    if repeat.is_some() && as_.is_none() {
        return Err("<panel repeat=\"...\">にはas属性が必要です".to_string());
    }
    let index = el.attribute("index").unwrap_or("i").to_string();
    let title = el.attribute("title").unwrap_or("").to_string();
    let span_raw = match el.attribute("span") {
        Some(v) => Some(parse_span_raw(v)?),
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
    Ok((Panel { repeat, as_, index, title, span_fraction: 0.0, columns, body }, span_raw))
}

/// `<panels>`内の各`<panel>`の`span`を解決し、`span_fraction`（0〜1の割合）へ変換する。規約:
/// - 全`<panel>`が`span`省略 → 均等割り（`1.0/n`。12で割り切れるかは問わない）
/// - 1個でも`span`が指定されていれば、**全`<panel>`に明示指定が必要**（一部省略は不可）
/// - 明示指定は「全て整数（12分割グリッド、合計12）」か「全てパーセント（合計100%、
///   誤差0.05%まで許容）」のどちらかに揃える必要がある（同じ`<panels>`内での混在は禁止）
fn resolve_spans(panels: &mut [Panel], spans_raw: &[Option<SpanRaw>]) -> Result<(), String> {
    let n = panels.len();
    if spans_raw.iter().all(|s| s.is_none()) {
        let even = 1.0 / n as f32;
        for p in panels.iter_mut() {
            p.span_fraction = even;
        }
        return Ok(());
    }
    if spans_raw.iter().any(|s| s.is_none()) {
        return Err(
            "<panels>内で一部の<panel>だけspanを省略することはできません（全て明示するか、全て省略するかのどちらかにしてください）"
                .to_string(),
        );
    }
    let all_grid = spans_raw.iter().all(|s| matches!(s, Some(SpanRaw::Grid(_))));
    let all_percent = spans_raw.iter().all(|s| matches!(s, Some(SpanRaw::Percent(_))));
    if !all_grid && !all_percent {
        return Err(
            "<panels>内でspanの整数指定（12分割グリッド）とパーセント指定は混在できません（どちらかに揃えてください）"
                .to_string(),
        );
    }
    if all_grid {
        let values: Vec<u32> = spans_raw
            .iter()
            .map(|s| match s {
                Some(SpanRaw::Grid(v)) => *v,
                _ => unreachable!("all_gridで検証済み"),
            })
            .collect();
        let total: u32 = values.iter().sum();
        if total != 12 {
            return Err(format!("<panels>内の<panel>のspan合計は12である必要があります（現在の合計: {total}）"));
        }
        for (p, v) in panels.iter_mut().zip(values.iter()) {
            p.span_fraction = *v as f32 / 12.0;
        }
    } else {
        let values: Vec<f32> = spans_raw
            .iter()
            .map(|s| match s {
                Some(SpanRaw::Percent(v)) => *v,
                _ => unreachable!("all_percentで検証済み"),
            })
            .collect();
        let total: f32 = values.iter().sum();
        if (total - 100.0).abs() > 0.05 {
            return Err(format!(
                "<panels>内の<panel>のspan(%)合計は100%である必要があります（現在の合計: {total}%）"
            ));
        }
        for (p, v) in panels.iter_mut().zip(values.iter()) {
            p.span_fraction = *v / 100.0;
        }
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

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn single_panel_defaults_span_to_full_width() {
        let xml = wrap(r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>"#);
        let layout = parse_layout(&xml).unwrap();
        approx(layout.groups[0].panels[0].span_fraction, 1.0);
    }

    #[test]
    fn two_panels_without_span_split_evenly() {
        let xml = wrap(
            r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        approx(layout.groups[0].panels[0].span_fraction, 0.5);
        approx(layout.groups[0].panels[1].span_fraction, 0.5);
    }

    #[test]
    fn explicit_spans_summing_to_12_are_kept() {
        let xml = wrap(
            r#"<panel span="4" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel span="8" title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        approx(layout.groups[0].panels[0].span_fraction, 4.0 / 12.0);
        approx(layout.groups[0].panels[1].span_fraction, 8.0 / 12.0);
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

    /// 12で割り切れない要素数でも、全省略なら単純な均等割りが通る（`span_fraction`は`1/n`）。
    /// 旧実装（12分割グリッド固定）はここでエラーにしていたが、`span_fraction`化に伴い緩和した。
    #[test]
    fn omitted_span_splits_evenly_even_when_not_divisible_by_12() {
        let xml = wrap(
            r#"<panel title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel title="B"><row><knob label="Y" handle="y"/></row></panel>
               <panel title="C"><row><knob label="Z" handle="z"/></row></panel>
               <panel title="D"><row><knob label="W" handle="w"/></row></panel>
               <panel title="E"><row><knob label="V" handle="v"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        for p in &layout.groups[0].panels {
            approx(p.span_fraction, 0.2);
        }
    }

    #[test]
    fn percent_spans_summing_to_100_are_kept() {
        let xml = wrap(
            r#"<panel span="25%" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel span="75%" title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        approx(layout.groups[0].panels[0].span_fraction, 0.25);
        approx(layout.groups[0].panels[1].span_fraction, 0.75);
    }

    /// 100.01%（誤差0.05%以内）は端数丸めの範囲として許容する。
    #[test]
    fn percent_spans_with_rounding_slack_are_kept() {
        let xml = wrap(
            r#"<panel span="33.34%" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel span="33.33%" title="B"><row><knob label="Y" handle="y"/></row></panel>
               <panel span="33.34%" title="C"><row><knob label="Z" handle="z"/></row></panel>"#,
        );
        let layout = parse_layout(&xml).unwrap();
        approx(layout.groups[0].panels[0].span_fraction, 0.3334);
    }

    #[test]
    fn percent_spans_not_summing_to_100_is_error() {
        let xml = wrap(
            r#"<panel span="25%" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel span="25%" title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("合計は100%"), "{err}");
    }

    #[test]
    fn mixing_grid_and_percent_span_is_error() {
        let xml = wrap(
            r#"<panel span="6" title="A"><row><knob label="X" handle="x"/></row></panel>
               <panel span="50%" title="B"><row><knob label="Y" handle="y"/></row></panel>"#,
        );
        let err = parse_layout(&xml).unwrap_err();
        assert!(err.contains("混在できません"), "{err}");
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
