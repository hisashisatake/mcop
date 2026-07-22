//! `panel.xml`のパース結果（中間表現）。
//!
//! ウィジェットは生成用文字列ではなく**構造化記述子**（[`Widget`]）として保持する。
//! `codegen.rs`はこれをRust文字列へ、将来のインタープリタ（フェーズB）は実egui呼び出しへ変換する。
//! 両者が同じ記述子を消費するため、ウィジェット選択のズレは原理的に発生しない。
//!
//! 生Rust注入（`<raw>`）は最終手段として文法上は残すが、`panel.xml`本体では使用しない
//! （`<title>`/`<readout>`/`enabled-if`の閉じた語彙で代替する）。

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
    /// `(label, handle)`の並び。
    CheckboxStack { items: Vec<(String, String)> },
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

/// パネル/カラムの見出し。`<title/>`（空）は親の`title=`属性を使う。
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
    pub size: Size,
    pub widget: Widget,
    pub enabled_if: Option<Predicate>,
    pub preview_label: String,
    pub preview_type: String,
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
    /// ラップなしの見出し単体（`<column>`のtitle自動出力用。旧実装が裸の`ui.label`だったことの再現）。
    Title(Title),
    Tree(TreeNode),
    Space { size: f32 },
    Jack(Jack),
    /// 生Rustの最終手段（`panel.xml`本体では未使用、文法としてのみ温存）。
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
