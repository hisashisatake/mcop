//! `panel.xml`のパース結果（中間表現）。
//!
//! ウィジェットは生成用文字列ではなく**構造化記述子**（[`Widget`]）として保持する。
//! `codegen.rs`はこれをRust文字列へ、将来のインタープリタ（フェーズB）は実egui呼び出しへ変換する。
//! 両者が同じ記述子を消費するため、ウィジェット選択のズレは原理的に発生しない。
//!
//! 生Rust注入（`<raw>`）は最終手段として文法上は残すが、`panel.xml`本体では使用しない
//! （`<title>`/`<readout>`/`enabled-if`の閉じた語彙で代替する）。

pub use ui_layout::Margin;

/// 固定サイズ（ウィジェットの自然サイズ）。
#[derive(Clone, Copy, Debug)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

/// `<style>`の解決結果（`<layout>`全体で1個、省略時は既定値）。
/// パネルのマージンは全パネル共通（`<panel>`単位の個別上書きは持たない）、
/// ウィジェットのマージンは既定値(`<widget margin>`)→タグ別上書き(`tag_margin`)→
/// インスタンス属性(`margin="..."`)の3段カスケードで解決する（後勝ち）。
#[derive(Clone, Debug)]
pub struct Style {
    /// 横並び`<panels>`グループ内、パネル間の隙間。
    pub panels_gap: f32,
    /// パネル枠（`ui.group`相当）の内側余白。`egui::Margin`がi8ベースのため整数のみ。
    pub panel_inner_margin: Margin,
    /// パネル枠の外側余白。整数のみ。
    pub panel_outer_margin: Margin,
    /// 全ウィジェット共通の既定マージン。
    pub widget_margin: Margin,
    /// タグ名（`"knob"`等）→マージンの個別上書き（`<style>`内の`<knob margin="...">`等）。
    pub tag_margin: std::collections::HashMap<String, Margin>,
    /// `<eg-preview>`の既定サイズ（インスタンス側`width`/`height`属性で個別上書き可能）。
    pub eg_preview_size: Size,
    /// `<algorithm-diagram>`の既定サイズ（インスタンス側`width`/`height`属性で個別上書き可能）。
    pub algorithm_diagram_size: Size,
    /// `<time-eg-editor>`の既定サイズ（インスタンス側`width`/`height`属性で個別上書き可能）。
    pub time_eg_editor_size: Size,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            panels_gap: 8.0,
            panel_inner_margin: Margin::same(6.0),
            panel_outer_margin: Margin::ZERO,
            widget_margin: Margin::ZERO,
            tag_margin: std::collections::HashMap::new(),
            // ym38x6-ui/src/eg_preview.rsのDEFAULT_WIDTH/DEFAULT_HEIGHTと一致させること
            // （panel-codegenはym38x6-uiに依存できないため値を複製している）。
            eg_preview_size: Size { w: 84.0, h: 66.0 },
            // ym38x6-ui/src/algorithm_diagram.rsのDEFAULT_WIDTH/DEFAULT_HEIGHTと一致させること。
            algorithm_diagram_size: Size { w: 150.0, h: 100.0 },
            // op505/ui/src/panel.xmlの<style><time-eg-editor>実測値と一致させること
            // （KNOBSモードの1段カラムがはみ出さない高さ、Step6実機確認で175→245へ追い込み済み）。
            time_eg_editor_size: Size { w: 260.0, h: 245.0 },
        }
    }
}

/// `<row>`/`<stack>`の`gap`属性。`"spacing"`は実行時の`ui.spacing().item_spacing.x`を指す
/// 記号値（コード生成では`outer_gap`変数）で、プレビュー側では8.0の近似値として扱う。
#[derive(Clone, Copy, Debug)]
pub enum Gap {
    Spacing,
    Fixed(f32),
}

impl Gap {
    /// プレビュー用の数値化（`"spacing"`は8.0固定の近似値。JS版`gapNumeric`と同じ割り切り）。
    pub fn numeric(&self) -> f32 {
        match self {
            Gap::Spacing => 8.0,
            Gap::Fixed(v) => *v,
        }
    }
}

/// `<eg-preview>`の各フィールド（`{field}="handle"`か`{field}-value="リテラル"`のどちらか）。
#[derive(Clone, Debug)]
pub enum EgField {
    /// パラメーターハンドルへの解決済みパス（`.value() as u8`を付けて参照する）。
    Handle(String),
    /// リテラル値（そのまま埋め込む）。
    Literal(String),
}

/// `<knob alt-label alt-handle>`（V.GAIN/VELのように役割で入れ替わる2パラメーターを
/// 1セルへまとめる指定）。`predicate`は`enabled-if`属性から借用する——通常の`enabled-if`
/// （グレーアウトのみ）と違い、`true`側と`false`側で表示するhandle自体を切り替える。
#[derive(Clone, Debug)]
pub struct KnobAlt {
    pub label: String,
    pub handle: String,
    pub predicate: Predicate,
}

/// `<row>`/`<stack>`直下の1葉（配置対象ウィジェット1個ぶん）の種別・構造化記述子。
#[derive(Clone, Debug)]
pub enum Widget {
    /// `bipolar`は中央128のパラメーターを-128〜+127の符号付き表示にする
    /// （`ui_core::BipolarHandle`を挟む。P.DEP±/F.DEP±/DT1/FINE/TX.OFS向け）。
    /// `alt`はV.GAIN/VELのような役割入れ替え2パラメーターの合体指定（[`KnobAlt`]参照）。
    /// Someのときはenabled-ifによる通常のグレーアウトではなく`dual_knob`で描画する。
    Knob { label: String, handle: String, bipolar: bool, alt: Option<KnobAlt> },
    Checkbox { label: String, handle: String },
    Waveform { handle: String, index: String },
    /// `height`は`<stack>`縦積み時の余白調整用（既定66.0は`<knob>`とのRow内揃えを保つ値、
    /// `<enum height="...">`で個別に縮小できる。`ui_core::selector::enum_selector`の
    /// `egui::vec2(100.0, height)`と一致させること）。
    Enum { label: String, handle: String, names: String, salt: String, height: f32 },
    /// TimeEgのテンポ同期レート（連続ノブ＋音価ドロップダウンの複合ウィジェット）。
    /// 候補名は`SYNC_NOTE_NAMES`固定なので`<enum>`と違い`names`属性を取らない。
    SyncRate { label: String, handle: String, salt: String },
    #[allow(clippy::too_many_arguments)]
    EgPreview {
        mapping: String,
        tl: EgField,
        ar: EgField,
        d1r: EgField,
        d1l: EgField,
        d2r: EgField,
        rr: EgField,
        floor: EgField,
        loop_enabled: EgField,
        curve: EgField,
        delay: EgField,
    },
    AlgorithmDiagram { handle: String },
    /// TimeEg（可変1〜10段・ループ・多段リリース）のハイブリッドエディタ（`ui_core::time_eg_editor`）。
    /// `handle`は`TimeEgHandle`実装（`Box<dyn TimeEgHandle>`フィールド）を指す。
    ///
    /// `min_stages`/`terminal_level_zero`はEG種別ごとの編集制約（`ui_core::TimeEgProfile`）。
    /// 既定はOP1〜4 EG/Pitch FG/Cutoff FG向けの`min-stages="2" terminal-level="zero"`で、
    /// キーオフで必ずレベル0へ着地させる。Gain FGだけ`min-stages="1" terminal-level="free"`を
    /// 指定し、透過既定（ゲートを一切閉じない1段EG）を表現できるようにする。
    TimeEgEditor { handle: String, mapping: String, tl: EgField, min_stages: u8, terminal_level_zero: bool },
    /// 生Rustの最終手段（`panel.xml`本体では未使用、文法としてのみ温存）。
    Raw(String),
}

/// 名前付きの派生計算（閉じた語彙。新しい計算を増やす場合はここへvariantを追加する）。
#[derive(Clone, Debug)]
pub enum Compute {
    /// キャリア判定: `carriers(params.algorithm.value() as u8).contains(&<loop変数>)`。
    IsCarrier { index_var: String },
    /// MUL×FINEの実効周波数比: `mul_fine_ratio(<mul>.value() as u8, <fine>.value() as u8)`。
    MulFineRatio { mul: String, fine: String },
}

/// `enabled-if="[!]<述語名>"`の解決結果。
#[derive(Clone, Debug)]
pub struct Predicate {
    pub compute: Compute,
    pub negate: bool,
}

/// `<readout>`（派生値の読み取り表示、ヘッダ内専用）。
#[derive(Clone, Debug)]
pub struct Readout {
    pub compute: Compute,
    /// Rustの`format!`テンプレート。`{value...}`トークン（例: `{value:.2}`）が計算結果の埋め込み位置。
    pub format: String,
    pub tooltip: Option<String>,
}

/// パネルの見出し。`<title/>`（空）は親の`title=`属性を使う。
#[derive(Clone, Debug)]
pub enum Title {
    Static(String),
    /// `{index+N}`を1箇所だけ含む動的テンプレート（例: OP見出しの`"OP {index+1}"`）。
    Dynamic { before: String, offset: i32, after: String, index_var: String },
}

/// `<row>`/`<stack>`の木構造。
#[derive(Clone, Debug)]
pub enum TreeNode {
    Leaf(LeafInfo),
    Row { justify: String, gap: Gap, grow: bool, children: Vec<TreeNode> },
    /// `center`は横方向（クロス軸）の揃え。既定false＝左詰め。`<stack center="true">`で
    /// 幅の狭い子（`<knob>`62px等）を幅の広い子（`<enum>`100px等）の中央へ揃えられる。
    Stack { gap: Gap, grow: bool, center: bool, children: Vec<TreeNode> },
}

/// `<row>`/`<stack>`直下の1葉（配置対象ウィジェット1個ぶん）。
#[derive(Clone, Debug)]
pub struct LeafInfo {
    /// ウィジェット自身の自然サイズ（マージンを含まない）。
    pub size: Size,
    /// このウィジェットの周囲余白（3段カスケードで解決済み）。
    pub margin: Margin,
    pub widget: Widget,
    pub enabled_if: Option<Predicate>,
    pub preview_label: String,
    pub preview_type: String,
}

impl LeafInfo {
    /// taffyに占有領域として渡す外形サイズ（`size` + `margin`）。
    pub fn outer_size(&self) -> Size {
        Size { w: self.size.w + self.margin.horizontal(), h: self.size.h + self.margin.vertical() }
    }
}

impl TreeNode {
    /// 木全体の高さ（JS版`maxHeight`と同一の再帰計算。leafは外形h、rowは子の最大、stackは子の合計+gap）。
    pub fn max_height(&self) -> f32 {
        match self {
            TreeNode::Leaf(l) => l.outer_size().h,
            TreeNode::Row { children, .. } => {
                children.iter().map(|c| c.max_height()).fold(f32::MIN, f32::max)
            }
            TreeNode::Stack { gap, children, .. } => {
                let n = children.len();
                let sum: f32 = children.iter().map(|c| c.max_height()).sum();
                sum + gap.numeric() * (n.saturating_sub(1)) as f32
            }
        }
    }

    /// 木が必要とする最小幅（`max_height`と対の再帰計算）。leafは外形w、rowは子の合計+gap
    /// （縮められないため、コンテナがこれより狭いとコンテナからあふれる＝呼び出し側で
    /// `ui.available_width().max(min_width())`として使う、下げ止まりの下限値）、
    /// stackは子の最大（縦積みなので幅は最大幅の子で決まる）。
    pub fn min_width(&self) -> f32 {
        match self {
            TreeNode::Leaf(l) => l.outer_size().w,
            TreeNode::Row { gap, children, .. } => {
                let n = children.len();
                let sum: f32 = children.iter().map(|c| c.min_width()).sum();
                sum + gap.numeric() * (n.saturating_sub(1)) as f32
            }
            TreeNode::Stack { children, .. } => {
                children.iter().map(|c| c.min_width()).fold(0.0_f32, f32::max)
            }
        }
    }

    /// DFS順（＝`layout::place`が呼ばれる順）で葉を列挙する。
    pub fn leaves(&self) -> Vec<&LeafInfo> {
        match self {
            TreeNode::Leaf(l) => vec![l],
            TreeNode::Row { children, .. } | TreeNode::Stack { children, .. } => {
                children.iter().flat_map(|c| c.leaves()).collect()
            }
        }
    }

    /// 実行時と同じ`ui_layout::Node`へ変換する（プレビューの本物taffy解決に使う）。
    /// `justify`が未知の値の場合はStartへフォールバックする（コード生成側は検証せず
    /// そのまま`Justify::{capitalize}`を出力しRustコンパイルエラーに委ねるが、
    /// プレビュー側はブラウザ内でパニックさせないためのフォールバック）。
    pub fn to_layout_node(&self) -> ui_layout::Node {
        match self {
            TreeNode::Leaf(l) => {
                let outer = l.outer_size();
                ui_layout::leaf(outer.w, outer.h)
            }
            TreeNode::Row { justify, gap, grow, children } => {
                let kids: Vec<ui_layout::Node> = children.iter().map(|c| c.to_layout_node()).collect();
                let j = justify_from_str(justify);
                if *grow {
                    ui_layout::row_grow(j, gap.numeric(), kids)
                } else {
                    ui_layout::row(j, gap.numeric(), kids)
                }
            }
            TreeNode::Stack { gap, grow, center, children } => {
                let kids: Vec<ui_layout::Node> = children.iter().map(|c| c.to_layout_node()).collect();
                match (*grow, *center) {
                    (false, false) => ui_layout::stack(gap.numeric(), kids),
                    (true, false) => ui_layout::stack_grow(gap.numeric(), kids),
                    (false, true) => ui_layout::stack_centered(gap.numeric(), kids),
                    (true, true) => ui_layout::stack_grow_centered(gap.numeric(), kids),
                }
            }
        }
    }
}

fn justify_from_str(s: &str) -> ui_layout::Justify {
    use ui_layout::Justify;
    match s {
        "start" => Justify::Start,
        "between" => Justify::Between,
        "around" => Justify::Around,
        "evenly" => Justify::Evenly,
        "center" => Justify::Center,
        "end" => Justify::End,
        _ => Justify::Start,
    }
}

#[derive(Clone, Debug)]
pub enum Jack {
    Source,
    Dest { dest_index: String, label: String, handle: String },
}

#[derive(Clone, Debug)]
pub enum HeaderItem {
    Title(Title),
    Readout(Readout),
    Jack(Jack),
    /// 生Rustの最終手段（`panel.xml`本体では未使用、文法としてのみ温存）。
    Raw(String),
}

#[derive(Clone, Debug)]
pub enum BodyStmt {
    /// `ui.horizontal(|ui| {...})`でラップされる先頭行（見出し＋ジャック等を横並びにする）。
    Header { items: Vec<HeaderItem> },
    Tree(TreeNode),
    Space { size: f32 },
    Jack(Jack),
    /// 生Rustの最終手段（`panel.xml`本体では未使用、文法としてのみ温存）。
    Raw(String),
}

/// `<panel>`（常に`<panels>`の子として1個以上並ぶ）。
#[derive(Clone, Debug)]
pub struct Panel {
    pub repeat: Option<String>,
    pub as_: Option<String>,
    pub index: String,
    pub title: String,
    /// このパネルが占める幅の割合（0〜1）。`span="4"`（12分割グリッド）なら`4.0/12.0`、
    /// `span="33%"`なら`0.33`。同じ`<panels>`内の全`<panel>`の合計は常に1.0
    /// （parse.rsで検証・省略時は均等割りで解決済み。整数指定とパーセント指定の混在は不可）。
    pub span_fraction: f32,
    /// `repeat`ありパネルをN列グリッドで折り返す列数（`<panel repeat="..." columns="N">`）。
    /// `repeat`なしパネルには付けられない（parse.rsで検証済み）。省略時は従来通りの縦一列。
    pub columns: Option<usize>,
    pub body: Vec<BodyStmt>,
}

impl Panel {
    /// body中の`<row>`/`<stack>`木（`BodyStmt::Tree`）が必要とする最小幅の最大値。
    /// `<header>`（見出し・readout・jack）は可変幅のため対象外。
    fn body_content_min_width(&self) -> f32 {
        self.body
            .iter()
            .filter_map(|stmt| match stmt {
                BodyStmt::Tree(t) => Some(t.min_width()),
                _ => None,
            })
            .fold(0.0_f32, f32::max)
    }

    /// このパネル自身の中身（枠を含まない）が必要とする最小幅。
    /// `repeat`＋`columns`のグリッド化パネルは、1セルぶんの最小幅を`columns`個並べた
    /// 合計（`codegen.rs`の`gen_repeat_grid`と同じ`grid_overhead`計算の逆算）になる。
    pub fn min_width(&self, style: &Style, frame_stroke: f32) -> f32 {
        let cell = self.body_content_min_width();
        match self.columns {
            Some(cols) if self.repeat.is_some() && cols > 1 => {
                let grid_overhead =
                    style.panel_inner_margin.horizontal() + style.panel_outer_margin.horizontal() + frame_stroke * 2.0;
                cell * cols as f32 + (grid_overhead + style.panels_gap) * (cols as f32 - 1.0)
            }
            _ => cell,
        }
    }
}

/// `<panels>`。`<panel>`を1個以上持つ（1個なら実質フル幅の単独パネル、2個以上ならNカラム）。
#[derive(Clone, Debug)]
pub struct PanelsGroup {
    pub match_height: bool,
    pub panels: Vec<Panel>,
}

impl PanelsGroup {
    /// このグループの`usable`（`gen_panels_group`のパネル外形を引いた後の中身の合計幅）が
    /// 最低限必要とする値。各パネルについて「中身の最小幅 ÷ span比率」を計算し、
    /// 最も厳しい（最大の）値を採用する（他パネルはその`usable`なら余裕で収まる）。
    pub fn min_usable_width(&self, style: &Style, frame_stroke: f32) -> f32 {
        self.panels
            .iter()
            .map(|p| p.min_width(style, frame_stroke) / p.span_fraction.max(f32::EPSILON))
            .fold(0.0_f32, f32::max)
    }
}

/// `<layout>`ルート属性。省略時は現行どおり`draw_param_panel`/`PanelParams`/id_saltなしになる
/// （`ym38x6-ui`はこの既定値のまま、`op505-ui`は`fn="draw_op505_panel" params-type="Op505PanelParams"
/// scroll-id="op505_panel"`で上書きする）。
#[derive(Clone, Debug)]
pub struct Layout {
    pub groups: Vec<PanelsGroup>,
    pub style: Style,
    /// 生成する描画関数名（`<layout fn="...">`、省略時`"draw_param_panel"`）。
    pub fn_name: String,
    /// `params: &{params_type}`の型名（`<layout params-type="...">`、省略時`"PanelParams"`）。
    pub params_type: String,
    /// `egui::ScrollArea::vertical()`の`.id_salt(...)`（`<layout scroll-id="...">`、省略時なし）。
    pub scroll_id: Option<String>,
}

impl Layout {
    /// レイアウト全体が重なりなく描画できる最小の`full_width`。全グループの
    /// `usable + panels_gap*(n-1) + overhead*n`（`gen_panels_group`/`draw_panels_group`の
    /// 幅計算式の逆算）のうち最大値を返す。呼び出し側は
    /// `ui.available_width().max(layout.min_full_width(..))`として使い、これを下回る幅では
    /// 圧縮せず（`ui-layout`のflex_shrink:0.0と合わせて）横スクロールへ委ねる。
    pub fn min_full_width(&self, style: &Style, frame_stroke: f32) -> f32 {
        let overhead = style.panel_inner_margin.horizontal() + style.panel_outer_margin.horizontal() + frame_stroke * 2.0;
        self.groups
            .iter()
            .map(|g| {
                let n = g.panels.len();
                let usable = g.min_usable_width(style, frame_stroke);
                usable + style.panels_gap * (n.saturating_sub(1)) as f32 + overhead * n as f32
            })
            .fold(0.0_f32, f32::max)
    }
}
