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

/// `<row>`/`<stack>`直下の1葉（配置対象ウィジェット1個ぶん）の種別・構造化記述子。
#[derive(Clone, Debug)]
pub enum Widget {
    Knob { label: String, handle: String },
    Checkbox { label: String, handle: String },
    Waveform { handle: String, index: String },
    Enum { label: String, handle: String, names: String, salt: String },
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
    Stack { gap: Gap, grow: bool, children: Vec<TreeNode> },
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
            TreeNode::Stack { gap, children, .. } => {
                let kids: Vec<ui_layout::Node> = children.iter().map(|c| c.to_layout_node()).collect();
                ui_layout::stack(gap.numeric(), kids)
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
    /// 12分割グリッドでの占有幅（Bootstrap等のcol-span相当）。
    /// 同じ`<panels>`内の全`<panel>`のspan合計は常に12（parse.rsで検証・省略時は均等割りで解決済み）。
    pub span: u32,
    pub body: Vec<BodyStmt>,
}

/// `<panels>`。`<panel>`を1個以上持つ（1個なら実質フル幅の単独パネル、2個以上ならNカラム）。
#[derive(Clone, Debug)]
pub struct PanelsGroup {
    pub match_height: bool,
    pub panels: Vec<Panel>,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub groups: Vec<PanelsGroup>,
    pub style: Style,
}
