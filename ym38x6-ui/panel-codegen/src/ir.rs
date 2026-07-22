//! `panel.xml`のパース結果（中間表現）。
//!
//! `tools/xml-panel-dsl/index.html`のJSロジック（旧`parseLayout`〜`buildLeafInfo`）の
//! 構造をそのまま1:1でRust化したもの。フィールド名・分岐は極力JS版と対応させてある。

/// 固定サイズ（ウィジェットの自然サイズ）。
#[derive(Clone, Copy, Debug)]
pub struct Size {
    pub w: f32,
    pub h: f32,
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

/// `<row>`/`<stack>`直下の1葉（配置対象ウィジェット1個ぶん）。
#[derive(Clone, Debug)]
pub struct LeafInfo {
    pub size: Size,
    /// `layout::place`のクロージャ内に埋め込むRust文（`enabled-if`があれば`ui.add_enabled_ui`でラップ済み）。
    pub rust_stmt: String,
    pub preview_label: String,
    pub preview_type: String,
}

/// `<row>`/`<stack>`の木構造。
#[derive(Clone, Debug)]
pub enum TreeNode {
    Leaf(LeafInfo),
    Row { justify: String, gap: Gap, grow: bool, children: Vec<TreeNode> },
    Stack { gap: Gap, grow: bool, children: Vec<TreeNode> },
}

impl TreeNode {
    /// 木全体の高さ（JS版`maxHeight`と同一の再帰計算。leafはh、rowは子の最大、stackは子の合計+gap）。
    pub fn max_height(&self) -> f32 {
        match self {
            TreeNode::Leaf(l) => l.size.h,
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

    /// 実行時と同じ`panel_layout::Node`へ変換する（プレビューの本物taffy解決に使う）。
    /// `justify`が未知の値の場合はStartへフォールバックする（コード生成側は検証せず
    /// そのまま`Justify::{capitalize}`を出力しRustコンパイルエラーに委ねるが、
    /// プレビュー側はブラウザ内でパニックさせないためのフォールバック）。
    pub fn to_layout_node(&self) -> panel_layout::Node {
        match self {
            TreeNode::Leaf(l) => panel_layout::leaf(l.size.w, l.size.h),
            TreeNode::Row { justify, gap, grow, children } => {
                let kids: Vec<panel_layout::Node> = children.iter().map(|c| c.to_layout_node()).collect();
                let j = justify_from_str(justify);
                if *grow {
                    panel_layout::row_grow(j, gap.numeric(), kids)
                } else {
                    panel_layout::row(j, gap.numeric(), kids)
                }
            }
            TreeNode::Stack { gap, children, .. } => {
                let kids: Vec<panel_layout::Node> = children.iter().map(|c| c.to_layout_node()).collect();
                panel_layout::stack(gap.numeric(), kids)
            }
        }
    }
}

fn justify_from_str(s: &str) -> panel_layout::Justify {
    use panel_layout::Justify;
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
    Raw(String),
    Jack(Jack),
}

#[derive(Clone, Debug)]
pub enum BodyStmt {
    Let { name: String, expr: String },
    Header { items: Vec<HeaderItem> },
    Tree(TreeNode),
    Space { size: f32 },
    Jack(Jack),
    Raw(String),
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub repeat: Option<String>,
    pub as_: Option<String>,
    pub index: String,
    pub title: String,
    pub body: Vec<BodyStmt>,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub width: f32,
    pub title: String,
    pub body: Vec<BodyStmt>,
}

#[derive(Clone, Debug)]
pub struct Columns {
    pub match_height: bool,
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug)]
pub enum Item {
    Panel(Panel),
    Columns(Columns),
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub items: Vec<Item>,
}
