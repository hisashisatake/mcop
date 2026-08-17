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

/// パネルのマージン（i8ベース、`parse_panel_margin`で整数検証済み）を`egui::Margin`リテラルへ。
fn egui_margin_literal(m: &Margin) -> String {
    format!(
        "egui::Margin {{ left: {}, right: {}, top: {}, bottom: {} }}",
        m.left as i32, m.right as i32, m.top as i32, m.bottom as i32
    )
}

/// ウィジェットのマージン（f32）を`layout::Margin`リテラルへ。
fn layout_margin_literal(m: &Margin) -> String {
    format!(
        "layout::Margin {{ top: {}, right: {}, bottom: {}, left: {} }}",
        fmt_num(m.top),
        fmt_num(m.right),
        fmt_num(m.bottom),
        fmt_num(m.left)
    )
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
/// `size`はそのleafの自然サイズ（マージンを含まない）。`eg-preview`/`algorithm-diagram`のみ、
/// 可変サイズを描画関数へ渡すために使う。
fn gen_widget_stmt(w: &Widget, size: Size) -> String {
    match w {
        Widget::Knob { label, handle, bipolar } => {
            if *bipolar {
                format!("knob(ui, &BipolarHandle::new(&*{handle}), \"{label}\");")
            } else {
                format!("knob(ui, &*{handle}, \"{label}\");")
            }
        }
        Widget::Checkbox { label, handle } => format!("bool_checkbox(ui, &*{handle}, \"{label}\");"),
        Widget::Waveform { handle, index } => format!("waveform_selector(ui, &*{handle}, ({index}) as usize);"),
        Widget::Enum { label, handle, names, salt } => {
            format!("enum_selector(ui, &*{handle}, \"{label}\", &{names}, {salt});")
        }
        Widget::SyncRate { label, handle, salt } => {
            format!("sync_rate_selector(ui, &*{handle}, \"{label}\", {salt});")
        }
        Widget::EgPreview { mapping, tl, ar, d1r, d1l, d2r, rr, floor, loop_enabled, curve, delay } => format!(
            "eg_preview(ui, egui::vec2({}, {}), EgAmplitudeMapping::{mapping}, {}, EgParams {{ ar: {}, d1r: {}, d1l: {}, d2r: {}, rr: {}, floor: {}, loop_enabled: {}, curve: {}, delay: {} }});",
            fmt_num(size.w),
            fmt_num(size.h),
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
        Widget::AlgorithmDiagram { handle } => {
            format!("algorithm_diagram(ui, egui::vec2({}, {}), {handle}.value() as u8);", fmt_num(size.w), fmt_num(size.h))
        }
        Widget::TimeEgEditor { handle, mapping, tl, min_stages, terminal_level_zero } => format!(
            "time_eg_editor(ui, egui::vec2({}, {}), &*{handle}, EgAmplitudeMapping::{mapping}, {}, TimeEgProfile {{ min_stages: {min_stages}, terminal_level_zero: {terminal_level_zero} }});",
            fmt_num(size.w),
            fmt_num(size.h),
            eg_field_expr(tl),
        ),
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
        let stmt = wrap_enabled_if(gen_widget_stmt(&leaf.widget, leaf.size), &leaf.enabled_if);
        let margin = layout_margin_literal(&leaf.margin);
        lines.push(format!("layout::place(ui, origin, *r.next().unwrap(), {margin}, |ui| {{ {stmt} }});"));
    }
    lines
}

fn tree_expr(node: &TreeNode) -> String {
    match node {
        TreeNode::Leaf(l) => {
            let outer = l.outer_size();
            format!("leaf({}, {})", fmt_num(outer.w), fmt_num(outer.h))
        }
        TreeNode::Row { justify, gap, grow, children } => {
            let ctor = if *grow { "row_grow" } else { "row" };
            let children_str = children.iter().map(tree_expr).collect::<Vec<_>>().join(", ");
            format!("{ctor}(Justify::{}, {}, vec![{children_str}])", capitalize(justify), gap_str(gap))
        }
        TreeNode::Stack { gap, grow, children } => {
            let ctor = if *grow { "stack_grow" } else { "stack" };
            let children_str = children.iter().map(tree_expr).collect::<Vec<_>>().join(", ");
            format!("{ctor}({}, vec![{children_str}])", gap_str(gap))
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

/// パネルの`body`に`Jack::Source`（質感LFOの出力ジャック）が含まれるか判定する。
/// 該当するのは`panel.xml`中のTEXTURE LFOパネル1個のみ。このパネルの外形矩形（`Frame::show`の
/// `response.rect`）を`tx_jacks.set_source_panel_rect`へ渡し、質感LFOパッチベイの「ケーブルを
/// パネル自身へドロップして未接続化する」判定（`patchbay::finish_texture_lfo_patchbay`）に使う。
fn panel_has_source_jack(body: &[BodyStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        BodyStmt::Header { items } => items.iter().any(|item| matches!(item, HeaderItem::Jack(Jack::Source))),
        BodyStmt::Jack(j) => matches!(j, Jack::Source),
        _ => false,
    })
}

/// `body`に`<jack>`（source/dest問わず）が1個以上含まれるか判定する。
/// `layout_has_any_jack`が全体を走査し、`generate_rust`が`tx_jacks`/patchbay呼び出しの
/// 出力要否（Step 3(c)）を決めるために使う。
fn body_has_any_jack(body: &[BodyStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        BodyStmt::Header { items } => items.iter().any(|item| matches!(item, HeaderItem::Jack(_))),
        BodyStmt::Jack(_) => true,
        _ => false,
    })
}

/// XML全体（全`<panels>`グループの全`<panel>`）に`<jack>`が1個以上あるか判定する。
/// 無ければ`tx_jacks`（`JackLayout`）自体を生成しない（op505-uiのようにTEXTURE LFOパッチベイを
/// 持たないパネルではpatchbayモジュールへの依存もコード生成物に出さない、Step 3(c)）。
fn layout_has_any_jack(layout: &Layout) -> bool {
    layout.groups.iter().any(|g| g.panels.iter().any(|p| body_has_any_jack(&p.body)))
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

/// 1個の`<panel>`（`egui::Frame::group`本体、`repeat`があればfor文でラップ）を生成する。
/// `width_expr`はこのパネルが占める幅を表すRust式（変数名）。`match_height`が`true`かつ
/// このパネルがグループ先頭（`capture_height`）なら`let group_resp = ...`で外形高さを捕捉し、
/// 先頭以外は捕捉済みの`match_height`（マージン・枠線ぶんを差し引いた中身の高さ）を
/// `set_min_height`する（`<panels match-height="true">`、グループが1個の`<panel>`のみの場合は実質無効）。
///
/// `Frame::show`が返す`response.rect`は`inner_margin`+枠線+`outer_margin`を含む外形なので、
/// 捕捉した高さをそのまま次のパネルの中身（`ui.set_min_height`は`inner_margin`の内側に効く）へ
/// 渡すとマージンぶん二重に積み増しされる。`margin_v`（style由来のコンパイル時定数）と
/// 実行時の`frame_stroke`を差し引いて中身の高さへ変換してから使う。
fn gen_panel(p: &Panel, width_expr: &str, match_height: bool, capture_height: bool, style: &Style) -> Vec<String> {
    let is_grid = p.repeat.is_some() && p.columns.is_some();
    let cell_width_expr = if is_grid { "w_cell" } else { width_expr };
    let body_lines = gen_body_lines(&p.body);
    let inner_margin = egui_margin_literal(&style.panel_inner_margin);
    let outer_margin = egui_margin_literal(&style.panel_outer_margin);
    let frame_expr =
        format!("egui::Frame::group(ui.style()).inner_margin({inner_margin}).outer_margin({outer_margin})");
    let has_source_jack = panel_has_source_jack(&p.body);
    let group_stmt = if capture_height || has_source_jack {
        format!("let group_resp = {frame_expr}.show(ui, |ui| {{")
    } else {
        format!("{frame_expr}.show(ui, |ui| {{")
    };
    let mut inner = vec![group_stmt, format!("    ui.set_width({cell_width_expr});")];
    if match_height && !capture_height {
        inner.push("    ui.set_min_height(match_height);".to_string());
    }
    inner.push("    ui.spacing_mut().item_spacing = base_spacing;".to_string());
    inner.push("    ui.vertical(|ui| {".to_string());
    for l in &body_lines {
        inner.push(indent(l, 8));
    }
    inner.push("    });".to_string());
    inner.push("});".to_string());
    if has_source_jack {
        inner.push("tx_jacks.set_source_panel_rect(group_resp.response.rect);".to_string());
    }
    if capture_height && match_height {
        let margin_v = fmt_num(style.panel_inner_margin.vertical() + style.panel_outer_margin.vertical());
        inner.push(format!("let match_height = group_resp.response.rect.height() - {margin_v} - frame_stroke * 2.0;"));
    }
    match &p.repeat {
        None => inner,
        Some(repeat) => {
            let as_ = p.as_.as_deref().unwrap_or("");
            match p.columns {
                None => {
                    let mut out =
                        vec![format!("for ({}, {as_}) in params.{repeat}.iter().enumerate() {{", p.index)];
                    for l in &inner {
                        out.push(indent(l, 4));
                    }
                    out.push("}".to_string());
                    out
                }
                Some(cols) => gen_repeat_grid(p, repeat, as_, cols, width_expr, &inner, style),
            }
        }
    }
}

/// `<panel repeat="..." columns="N">`をN列グリッドに折り返すRustコードを生成する。
/// `width_expr`（グループから渡された、このパネル1個ぶんの**中身**の幅）をセル1個ぶんの
/// 幅`w_cell`へ分割し、`chunks(N)`で行ごとに`ui.horizontal`をネストする。
///
/// `width_expr`はすでに`gen_panels_group`側で「このパネル自身の枠（マージン＋枠線）ぶん」を
/// 1回差し引き済みの値（`gen_panels_group`のn=1分岐: `w_0 = usable`、`usable`算出時に
/// `overhead*1`を控除済み）。グリッドの各セルは`gen_panel`が生成する**個別の**
/// `egui::Frame::group`（それぞれが独自にマージン＋枠線ぶんの`grid_overhead`を消費する）なので、
/// セル数`cols`ぶんの`grid_overhead`を単純に`width_expr`から追加で差し引くと、実際には存在しない
/// 「外側の1個ぶんの枠」の余白まで二重に控除してしまい、グリッド全体の右端が兄弟パネル
/// （`gen_panels_group`のn>1分岐で計算される通常パネル列）より内側に寄ってしまう
/// （margin/frame_strokeの合計だけ右マージンが余分に空く実バグを踏んだ）。
/// 正しくは「`width_expr`に一度差し引かれた1個ぶんの`grid_overhead`をまず足し戻し、
/// 列間ギャップ`gap`と`grid_overhead`をセル間の`(cols-1)`個ぶんだけ差し引く」計算になる
/// （`width_expr + grid_overhead - gap*(cols-1) - grid_overhead*cols`を整理すると
/// `width_expr - (grid_overhead + gap) * (cols - 1)`。cols=1では`width_expr`のまま＝
/// 非グリッド経路と一致し、footprint合計が`width_expr + grid_overhead`＝
/// 兄弟パネルの外形幅と厳密に一致することを式変形で確認済み）。
fn gen_repeat_grid(
    p: &Panel,
    repeat: &str,
    as_: &str,
    cols: usize,
    width_expr: &str,
    inner: &[String],
    style: &Style,
) -> Vec<String> {
    let gap = fmt_num(style.panels_gap);
    let margin_h = fmt_num(style.panel_inner_margin.horizontal() + style.panel_outer_margin.horizontal());
    let index = &p.index;
    let mut out = vec![
        format!("let grid_overhead = {margin_h} + frame_stroke * 2.0;"),
        format!(
            "let grid_usable = {width_expr} - (grid_overhead + {gap}) * {};",
            fmt_num((cols.saturating_sub(1)) as f32)
        ),
        format!("let w_cell = grid_usable / {};", fmt_num(cols as f32)),
        format!("for ({index}_row, {index}_chunk) in params.{repeat}.chunks({cols}).enumerate() {{"),
        "    ui.horizontal(|ui| {".to_string(),
        format!("        ui.spacing_mut().item_spacing.x = {gap};"),
        format!("        for ({index}_col, {as_}) in {index}_chunk.iter().enumerate() {{"),
        format!("            let {index} = {index}_row * {cols} + {index}_col;"),
    ];
    for l in inner {
        out.push(indent(l, 12));
    }
    out.push("        }".to_string());
    out.push("    });".to_string());
    out.push("}".to_string());
    out
}

/// `<panels>`（`<panel>`を1個以上持つ、12分割グリッドの1行）を生成する。
/// 幅計算はspan/12ベース（`<panel>`のspan合計は必ず12、parse.rsで検証済み）。
/// `<style><panels gap>`は列間(n-1)個ぶんのみ引く。パネル自身の枠（inner/outer margin＋枠線）は
/// 各パネルの`overhead`として`n`個ぶん引き、パネルの**外形**が利用可能幅にちょうど収まるようにする
/// （旧実装は中身の幅だけを`set_width`しており、`ui.group`が外側に足す既定マージンぶんはみ出していた）。
///
/// **n=1のとき`ui.horizontal`ではラップしない**（`repeat`ありパネルだと、for文全体が
/// 1個の`ui.horizontal`クロージャに閉じ込められ、本来ScrollArea直下で縦積みされるべき
/// 複数回の`Frame::show`が横並びになってしまう実バグを踏んだため。n>1の複数カラム行だけ
/// `ui.horizontal`で横並びにする）。
fn gen_panels_group(g: &PanelsGroup, style: &Style) -> String {
    let n = g.panels.len();
    let gap = fmt_num(style.panels_gap);
    let margin_h = fmt_num(style.panel_inner_margin.horizontal() + style.panel_outer_margin.horizontal());
    let mut out = vec![
        format!("let overhead = {margin_h} + frame_stroke * 2.0;"),
        format!(
            "let usable = full_width - {gap} * {} - overhead * {};",
            fmt_num(n.saturating_sub(1) as f32),
            fmt_num(n as f32)
        ),
    ];
    for (i, p) in g.panels.iter().enumerate() {
        out.push(format!("let w_{i} = usable * {} / 12.0;", fmt_num(p.span as f32)));
    }
    if n == 1 {
        out.extend(gen_panel(&g.panels[0], "w_0", false, false, style));
        return out.join("\n");
    }
    out.push("ui.horizontal(|ui| {".to_string());
    out.push(format!("    ui.spacing_mut().item_spacing.x = {gap};"));
    for (i, p) in g.panels.iter().enumerate() {
        let width_expr = format!("w_{i}");
        let capture_height = i == 0 && g.match_height;
        let lines = gen_panel(p, &width_expr, g.match_height, capture_height, style);
        for l in &lines {
            out.push(indent(l, 4));
        }
    }
    out.push("});".to_string());
    out.join("\n")
}

/// [`Layout`]全体から描画関数（本体アイテムのみ）を生成する。関数名・`params`の型・
/// `ScrollArea`の`id_salt`は`<layout fn= params-type= scroll-id=>`ルート属性で決まる
/// （省略時は`draw_param_panel`/`PanelParams`/id_saltなし、Step 3(b)）。
/// `<jack>`が1個も無いXML（op505-uiのFG/OPパネル等）では`tx_jacks`宣言とpatchbay呼び出し
/// 自体を出力しない（Step 3(c)）。
pub fn generate_rust(layout: &Layout) -> String {
    let style = &layout.style;
    let parts: Vec<String> = layout.groups.iter().map(|g| gen_panels_group(g, style)).collect();
    let body = indent(&parts.join("\n\n"), 8);
    let scroll_area = match &layout.scroll_id {
        Some(id) => format!("egui::ScrollArea::vertical().id_salt(\"{id}\").show(ui, |ui| {{"),
        None => "egui::ScrollArea::vertical().show(ui, |ui| {".to_string(),
    };
    let has_jack = layout_has_any_jack(layout);
    let tx_jacks_decl = if has_jack { "let mut tx_jacks = crate::patchbay::JackLayout::new();\n    " } else { "" };
    let finish_patchbay = if has_jack {
        "\n\n        crate::patchbay::finish_texture_lfo_patchbay(ui, &*params.texture_lfo_destination, tx_jacks);"
    } else {
        ""
    };
    format!(
        "pub fn {}(ui: &mut egui::Ui, params: &{}) {{\n    \
         {tx_jacks_decl}{scroll_area}\n        \
         let full_width = ui.available_width();\n        \
         let base_spacing = ui.spacing().item_spacing;\n        \
         let frame_stroke = ui.style().visuals.widgets.noninteractive.bg_stroke.width;\n        \
         ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);\n\n\
         {body}{finish_patchbay}\n    \
         }});\n\
         }}\n",
        layout.fn_name, layout.params_type,
    )
}
