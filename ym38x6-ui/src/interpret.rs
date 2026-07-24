//! `panel.xml`のIR（`panel-codegen`）をランタイムで解釈し、実eguiウィジェットを描画する
//! （フェーズB、`preview`フィーチャ配下）。`codegen.rs`（IR→Rust文字列）と対をなす
//! 「IR→実egui呼び出し」変換で、`tools/xml-panel-dsl`のブラウザプレビュー（`preview-wasm`）から
//! 使われる。VST/gesture-app向けの`panel.rs`（build.rsが生成する`draw_param_panel`）とは別経路
//! であり、生成コードには一切影響しない。
//!
//! `PanelParams`/`OperatorPanelParams`等の実パラメーター構造体は使わず、XMLの`handle`属性の
//! 文字列（例:`"op.tl"`・`"params.pitch_fg.eg.ar"`）をそのままキーにした[`HandleStore`]の
//! モックハンドルへ描画する。プレビューはレイアウト・見た目の検証が目的でパラメーター意味論
//! （レンジ・デフォルト値）は保証しない（既定0〜255・中央値、enum/algorithm等はウィジェット側の
//! 内部クランプに委ねる）。

use std::cell::Cell;
use std::collections::HashMap;

use panel_codegen::{
    BodyStmt, Compute, EgField, HeaderItem, Jack, Layout, LeafInfo, Margin, Panel, PanelsGroup, Predicate, Readout,
    Style, Title, TreeNode, Widget,
};
use sound_core::eg::EgParams;

use crate::algorithm_diagram::{algorithm_diagram, carriers};
use crate::eg_preview::{eg_preview, EgAmplitudeMapping};
use crate::knob::{bool_checkbox, knob};
use crate::layout;
use crate::panel::mul_fine_ratio;
use crate::param_handle::{BoolParamHandle, IntParamHandle};
use crate::patchbay::{finish_texture_lfo_patchbay, texture_lfo_dest_jack, texture_lfo_source_jack, JackLayout};
use crate::selector::{enum_selector, CHORUS_TYPE_NAMES, LFO_FADE_MODE_NAMES, LFO_WAVEFORM_NAMES, REVERB_TYPE_NAMES};
use crate::waveform::waveform_selector;

/// `<panel repeat="...">`の要素数解決（現状"operators"=4のみ。今後増える場合はここへ追加する）。
fn repeat_count(name: &str) -> usize {
    match name {
        "operators" => 4,
        _ => 1,
    }
}

/// プレビュー専用のモック整数パラメーター（Cellでフレーム跨ぎの値を保持し、ドラッグを効かせる）。
struct MockInt {
    value: Cell<i32>,
    min: i32,
    max: i32,
    default: i32,
    name: String,
}

impl IntParamHandle for MockInt {
    fn value(&self) -> i32 {
        self.value.get()
    }
    fn min(&self) -> i32 {
        self.min
    }
    fn max(&self) -> i32 {
        self.max
    }
    fn default(&self) -> i32 {
        self.default
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: i32) {
        self.value.set(value.clamp(self.min, self.max));
    }
    fn end_edit(&self) {}
}

struct MockBool {
    value: Cell<bool>,
}

impl BoolParamHandle for MockBool {
    fn value(&self) -> bool {
        self.value.get()
    }
    fn begin_edit(&self) {}
    fn set(&self, value: bool) {
        self.value.set(value);
    }
    fn end_edit(&self) {}
}

/// 解決済みハンドルパス文字列（例:`"op.tl"`、リピートパネル内は[`scoped`]で
/// `"{index}#op.tl"`へ変換したもの）→モックハンドルの対応。呼び出し側（`preview-wasm`のApp）が
/// フレームをまたいで使い回すことで、ドラッグ操作の値が次フレームにも保持される。
#[derive(Default)]
pub struct HandleStore {
    ints: HashMap<String, MockInt>,
    bools: HashMap<String, MockBool>,
}

impl HandleStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn int(&mut self, key: &str) -> &MockInt {
        &*self.ints.entry(key.to_string()).or_insert_with(|| MockInt {
            value: Cell::new(128),
            min: 0,
            max: 255,
            default: 128,
            name: key.to_string(),
        })
    }

    fn bool_(&mut self, key: &str) -> &MockBool {
        &*self.bools.entry(key.to_string()).or_insert_with(|| MockBool { value: Cell::new(false) })
    }
}

/// `<panel repeat>`内では同じ`handle`文字列（例:`"op.tl"`）が4オペレーター分共有されてしまうため、
/// ループindexを前置して別々のモックへ振り分ける。`repeat`が無い（`idx=None`）場合はそのまま。
fn scoped(path: &str, idx: Option<usize>) -> String {
    match idx {
        Some(i) => format!("{i}#{path}"),
        None => path.to_string(),
    }
}

/// `index="i"`（ループ変数参照）/`index="0"`（リテラル）のどちらかを実行時のusizeへ解決する。
/// 数値としてパースできなければループ変数参照とみなす（IR側の閉じた文法を前提にした割り切り）。
fn resolve_dyn_index(raw: &str, idx: Option<usize>) -> usize {
    raw.parse().unwrap_or_else(|_| idx.unwrap_or(0))
}

/// `names`属性（Rust定数名の文字列）→実スライス。閉じた語彙（`enum-selector`が使う4定数のみ）。
fn resolve_names(name: &str) -> &'static [&'static str] {
    match name {
        "REVERB_TYPE_NAMES" => &REVERB_TYPE_NAMES,
        "CHORUS_TYPE_NAMES" => &CHORUS_TYPE_NAMES,
        "LFO_WAVEFORM_NAMES" => &LFO_WAVEFORM_NAMES,
        "LFO_FADE_MODE_NAMES" => &LFO_FADE_MODE_NAMES,
        _ => &["?"],
    }
}

/// `enabled-if`述語の評価。`is_carrier`は常にグローバルな`"params.algorithm"`ハンドルを参照する
/// （`codegen.rs`の`compute_expr`が常に`params.algorithm.value()`を埋め込むのと同じ割り切り）。
fn eval_predicate(store: &mut HandleStore, pred: &Predicate, idx: Option<usize>) -> bool {
    let value = match &pred.compute {
        Compute::IsCarrier { .. } => {
            let algorithm = store.int("params.algorithm").value() as u8;
            carriers(algorithm).contains(&idx.unwrap_or(0))
        }
        // parse.rsのparse_readoutが"mul-fine-ratio"のみ許可するため、enabled-if側には現れない。
        Compute::MulFineRatio { .. } => false,
    };
    if pred.negate {
        !value
    } else {
        value
    }
}

fn eg_field_value(store: &mut HandleStore, f: &EgField, idx: Option<usize>) -> u8 {
    match f {
        EgField::Handle(h) => store.int(&scoped(h, idx)).value() as u8,
        EgField::Literal(v) => v.parse().unwrap_or(0),
    }
}

/// `{value}`/`{value:.N}`（閉じた文法、`format`属性で観測される唯一の形）専用の簡易フォーマッタ。
/// `codegen.rs`はRustの`format!`マクロ（コンパイル時文字列）に頼れるが、インタープリタは
/// 実行時に任意のテンプレート文字列を解釈する必要があるため、この極小サブセットだけ自前で処理する。
fn format_readout(template: &str, value: f32) -> String {
    let Some(start) = template.find("{value") else {
        return template.to_string();
    };
    let rest = &template[start..];
    let Some(end) = rest.find('}') else {
        return template.to_string();
    };
    let spec = &rest["{value".len()..end];
    let formatted = match spec.strip_prefix(":.") {
        Some(prec) => {
            let p: usize = prec.parse().unwrap_or(2);
            format!("{value:.p$}")
        }
        None => format!("{value}"),
    };
    format!("{}{}{}", &template[..start], formatted, &rest[end + 1..])
}

fn draw_readout(ui: &mut egui::Ui, store: &mut HandleStore, r: &Readout, idx: Option<usize>) {
    let Compute::MulFineRatio { mul, fine } = &r.compute else {
        return;
    };
    let mul_v = store.int(&scoped(mul, idx)).value() as u8;
    let fine_v = store.int(&scoped(fine, idx)).value() as u8;
    let value = mul_fine_ratio(mul_v, fine_v);
    let text = format_readout(&r.format, value);
    let resp = ui.label(egui::RichText::new(text).size(10.0).weak());
    if let Some(tooltip) = &r.tooltip {
        resp.on_hover_text(tooltip.as_str());
    }
}

fn draw_title(ui: &mut egui::Ui, t: &Title, idx: Option<usize>) {
    let text = match t {
        Title::Static(s) => s.clone(),
        Title::Dynamic { before, offset, after, .. } => {
            let n = idx.unwrap_or(0) as i32 + offset;
            format!("{before}{n}{after}")
        }
    };
    ui.label(egui::RichText::new(text).strong());
}

fn draw_jack(ui: &mut egui::Ui, store: &mut HandleStore, j: &Jack, idx: Option<usize>, jacks: &mut JackLayout) {
    match j {
        Jack::Source => texture_lfo_source_jack(ui, jacks),
        Jack::Dest { dest_index, label, handle } => {
            let dest_i: usize = dest_index.parse().unwrap_or(0);
            let key = scoped(handle, idx);
            texture_lfo_dest_jack(ui, store.int(&key), dest_i, label, jacks);
        }
    }
}

/// プレースホルダ描画（`<raw>`専用の最終手段）。生Rustは解釈できないため、確保済みの矩形へ
/// クラッシュしない程度の枠+ラベルだけ描く。
fn draw_raw_placeholder(ui: &mut egui::Ui, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(60));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "raw",
        egui::FontId::proportional(10.0),
        egui::Color32::WHITE,
    );
}

fn draw_widget(ui: &mut egui::Ui, store: &mut HandleStore, leaf: &LeafInfo, idx: Option<usize>) {
    match &leaf.widget {
        Widget::Knob { label, handle } => {
            let key = scoped(handle, idx);
            knob(ui, store.int(&key), label);
        }
        Widget::Checkbox { label, handle } => {
            let key = scoped(handle, idx);
            bool_checkbox(ui, store.bool_(&key), label);
        }
        Widget::Waveform { handle, index } => {
            let key = scoped(handle, idx);
            let salt = resolve_dyn_index(index, idx);
            waveform_selector(ui, store.int(&key), salt);
        }
        Widget::Enum { label, handle, names, salt } => {
            let key = scoped(handle, idx);
            let names_slice = resolve_names(names);
            let salt_n: usize = salt.parse().unwrap_or(0);
            enum_selector(ui, store.int(&key), label, names_slice, salt_n);
        }
        Widget::EgPreview { mapping, tl, ar, d1r, d1l, d2r, rr, floor, loop_enabled, curve, delay } => {
            let mapping_v =
                if mapping == "AmplitudeLinear" { EgAmplitudeMapping::AmplitudeLinear } else { EgAmplitudeMapping::DbLinear };
            let tl_v = eg_field_value(store, tl, idx);
            let eg = EgParams {
                ar: eg_field_value(store, ar, idx),
                d1r: eg_field_value(store, d1r, idx),
                d1l: eg_field_value(store, d1l, idx),
                d2r: eg_field_value(store, d2r, idx),
                rr: eg_field_value(store, rr, idx),
                floor: eg_field_value(store, floor, idx),
                loop_enabled: eg_field_value(store, loop_enabled, idx),
                curve: eg_field_value(store, curve, idx),
                delay: eg_field_value(store, delay, idx),
            };
            eg_preview(ui, egui::vec2(leaf.size.w, leaf.size.h), mapping_v, tl_v, eg);
        }
        Widget::AlgorithmDiagram { handle } => {
            let key = scoped(handle, idx);
            let v = store.int(&key).value() as u8;
            algorithm_diagram(ui, egui::vec2(leaf.size.w, leaf.size.h), v);
        }
        Widget::Raw(_) => draw_raw_placeholder(ui, egui::vec2(leaf.size.w, leaf.size.h)),
    }
}

/// `<row>`/`<stack>`の木を`panel_layout::solve`で解決し、各葉を実ウィジェットで描画する
/// （`codegen.rs`の`gen_tree_block`と同じ構造）。
fn draw_tree(ui: &mut egui::Ui, store: &mut HandleStore, node: &TreeNode, idx: Option<usize>) {
    let h = node.max_height();
    let avail = ui.available_width();
    let layout_node = node.to_layout_node();
    let rects = layout::solve(egui::vec2(avail, h), &layout_node);
    let origin = ui.cursor().min;
    ui.allocate_space(egui::vec2(avail, h));
    for (leaf, rect) in node.leaves().iter().zip(rects.iter()) {
        let enabled = match &leaf.enabled_if {
            Some(pred) => eval_predicate(store, pred, idx),
            None => true,
        };
        layout::place(ui, origin, *rect, leaf.margin, |ui| {
            ui.add_enabled_ui(enabled, |ui| draw_widget(ui, store, leaf, idx));
        });
    }
}

fn draw_header(ui: &mut egui::Ui, store: &mut HandleStore, items: &[HeaderItem], idx: Option<usize>, jacks: &mut JackLayout) {
    ui.horizontal(|ui| {
        for item in items {
            match item {
                HeaderItem::Title(t) => draw_title(ui, t, idx),
                HeaderItem::Readout(r) => draw_readout(ui, store, r, idx),
                HeaderItem::Jack(j) => draw_jack(ui, store, j, idx, jacks),
                HeaderItem::Raw(_) => draw_raw_placeholder(ui, egui::vec2(60.0, 16.0)),
            }
        }
    });
}

fn draw_body(ui: &mut egui::Ui, store: &mut HandleStore, body: &[BodyStmt], idx: Option<usize>, jacks: &mut JackLayout) {
    for stmt in body {
        match stmt {
            BodyStmt::Header { items } => draw_header(ui, store, items, idx, jacks),
            BodyStmt::Tree(tree) => draw_tree(ui, store, tree, idx),
            BodyStmt::Space { size } => {
                ui.add_space(*size);
            }
            BodyStmt::Jack(j) => draw_jack(ui, store, j, idx, jacks),
            BodyStmt::Raw(_) => draw_raw_placeholder(ui, egui::vec2(120.0, 16.0)),
        }
    }
}

/// 1個の`<panel>`を描画する。`min_height`は`<panels match-height="true">`でグループ先頭から
/// 捕捉した中身の高さ（先頭自身には`None`を渡す）。戻り値はこのパネルの`Frame::show`応答
/// （先頭の高さ捕捉用、`repeat`ありパネルは複数回描画するため最後の応答を返す）。
/// `codegen.rs`の`gen_panel`と同じくFrame外形高さからマージン・枠線ぶんを差し引いて
/// `set_min_height`に渡す（`Frame::show`のresponse.rectは`inner_margin`+枠線+`outer_margin`を
/// 含む外形のため、そのまま次のパネルの中身へ渡すと二重に積み増しされる）。
#[allow(clippy::too_many_arguments)]
fn draw_panel(
    ui: &mut egui::Ui,
    store: &mut HandleStore,
    p: &Panel,
    width: f32,
    min_height: Option<f32>,
    style: &Style,
    base_spacing: egui::Vec2,
    jacks: &mut JackLayout,
) -> egui::Response {
    let inner_margin = margin_to_egui(&style.panel_inner_margin);
    let outer_margin = margin_to_egui(&style.panel_outer_margin);
    let mut draw_one = |ui: &mut egui::Ui, idx: Option<usize>| {
        egui::Frame::group(ui.style())
            .inner_margin(inner_margin)
            .outer_margin(outer_margin)
            .show(ui, |ui| {
                ui.set_width(width);
                if let Some(h) = min_height {
                    ui.set_min_height(h);
                }
                ui.spacing_mut().item_spacing = base_spacing;
                ui.vertical(|ui| draw_body(ui, store, &p.body, idx, jacks));
            })
            .response
    };
    match &p.repeat {
        None => draw_one(ui, None),
        Some(name) => {
            let mut resp = None;
            for i in 0..repeat_count(name) {
                resp = Some(draw_one(ui, Some(i)));
            }
            resp.expect("repeat_count()は常に1以上を返す")
        }
    }
}

/// `<panels>`（`<panel>`を1個以上持つ、12分割グリッドの1行）を描画する。
/// `codegen.rs`の`gen_panels_group`と同じ幅計算（パネル外形が利用可能幅にちょうど収まる式）を踏襲する。
/// **n=1のとき`ui.horizontal`ではラップしない**（`repeat`ありパネルだと、内部のfor文全体が
/// 1個の`ui.horizontal`クロージャに閉じ込められ、本来縦積みされるべき複数回の`Frame::show`が
/// 横並びになってしまう実バグを踏んだため。n>1の複数カラム行だけ`ui.horizontal`で横並びにする）。
#[allow(clippy::too_many_arguments)]
fn draw_panels_group(
    ui: &mut egui::Ui,
    store: &mut HandleStore,
    g: &PanelsGroup,
    full_width: f32,
    frame_stroke: f32,
    style: &Style,
    base_spacing: egui::Vec2,
    jacks: &mut JackLayout,
) {
    let n = g.panels.len();
    let overhead = style.panel_inner_margin.horizontal() + style.panel_outer_margin.horizontal() + frame_stroke * 2.0;
    let usable = full_width - style.panels_gap * n.saturating_sub(1) as f32 - overhead * n as f32;
    if n == 1 {
        let w = usable * g.panels[0].span as f32 / 12.0;
        draw_panel(ui, store, &g.panels[0], w, None, style, base_spacing, jacks);
        return;
    }
    let match_height = g.match_height;
    let margin_v = style.panel_inner_margin.vertical() + style.panel_outer_margin.vertical();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = style.panels_gap;
        let mut captured_height: Option<f32> = None;
        for (i, p) in g.panels.iter().enumerate() {
            let w = usable * p.span as f32 / 12.0;
            let min_height = if match_height && i > 0 { captured_height } else { None };
            let resp = draw_panel(ui, store, p, w, min_height, style, base_spacing, jacks);
            if i == 0 && match_height {
                captured_height = Some(resp.rect.height() - margin_v - frame_stroke * 2.0);
            }
        }
    });
}

/// [`panel_codegen::Margin`]（f32・全パネル共通）をパース時に検証済みの`egui::Margin`（i8）へ変換する。
fn margin_to_egui(m: &Margin) -> egui::Margin {
    egui::Margin { left: m.left as i8, right: m.right as i8, top: m.top as i8, bottom: m.bottom as i8 }
}

/// `panel.xml`（をパースした[`Layout`]）をランタイム解釈し、`params: &PanelParams`を必要とせず
/// 直接eguiウィジェットを描画する。`store`は呼び出し側（`preview-wasm`のApp）がフレームをまたいで
/// 保持し、ドラッグ等の操作結果を次フレームへ引き継ぐ。
pub fn draw_panel_from_ir(ui: &mut egui::Ui, layout_ir: &Layout, store: &mut HandleStore) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let full_width = ui.available_width();
        let base_spacing = ui.spacing().item_spacing;
        let frame_stroke = ui.style().visuals.widgets.noninteractive.bg_stroke.width;
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        let mut jacks = JackLayout::new();

        for group in &layout_ir.groups {
            draw_panels_group(ui, store, group, full_width, frame_stroke, &layout_ir.style, base_spacing, &mut jacks);
        }

        finish_texture_lfo_patchbay(ui, store.int("params.texture_lfo_destination"), jacks);
    });
}
