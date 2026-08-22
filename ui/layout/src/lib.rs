//! taffyベースのレイアウト計算（egui非依存の純粋部分）。
//!
//! `ym38x6-ui/src/layout.rs`（eguiとの橋渡し）と、ブラウザツール`tools/xml-panel-dsl`の
//! wasmプレビュー（`panel-codegen`経由）の双方から同一コードとして共有される。
//! ここにはegui型を一切持ち込まず、taffyのみに依存する。

use taffy::prelude::*;

/// 2次元ベクトル（egui非依存）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 矩形（egui非依存、左上原点+サイズ）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// CSS風の4辺マージン（egui非依存）。`panel-codegen`（IRの`LeafInfo::margin`）と
/// `ym38x6-ui`（`layout::place`での描画時インセット）の双方が同じ型を共有する。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Margin {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Margin {
    pub const ZERO: Self = Self { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };

    pub const fn same(v: f32) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// 横並び（Row）での主軸方向の分配方式。CSS `justify-content`に対応。
#[derive(Clone, Copy)]
pub enum Justify {
    /// 左詰め（隙間は`gap`のみ）。
    Start,
    /// 両端フラッシュ＋等間隔（旧`justified_row`と同一）。
    Between,
    /// 各要素の周囲に等しい余白。
    Around,
    /// 要素間・両端すべて等しい余白。
    Evenly,
    /// 中央寄せ。
    Center,
    /// 右詰め。
    End,
}

impl Justify {
    fn to_taffy(self) -> JustifyContent {
        match self {
            Justify::Start => JustifyContent::Start,
            Justify::Between => JustifyContent::SpaceBetween,
            Justify::Around => JustifyContent::SpaceAround,
            Justify::Evenly => JustifyContent::SpaceEvenly,
            Justify::Center => JustifyContent::Center,
            Justify::End => JustifyContent::End,
        }
    }
}

/// レイアウト木のノード。
/// - [`Node::Leaf`]: 固定サイズのウィジェット1個（配置対象。`solve`の戻り値にDFS順で1個ずつ現れる）
/// - [`Node::Row`]: 横並びコンテナ
/// - [`Node::Stack`]: 縦積みコンテナ
pub enum Node {
    Leaf { size: Vec2 },
    Row { justify: Justify, gap: f32, grow: f32, children: Vec<Node> },
    /// `align_center`は横方向（クロス軸）の揃え。既定false＝左詰め（子の最大幅に合わせて
    /// 左から積む）。trueにすると`<knob>`(62px)のような子の幅が`<enum>`(100px)より狭い場合、
    /// 幅の狭い子を中央寄せする（taffyの`align_items: Center`）。
    Stack { gap: f32, grow: f32, align_center: bool, children: Vec<Node> },
}

/// 固定サイズの葉（配置対象ウィジェット1個ぶん）。
pub fn leaf(width: f32, height: f32) -> Node {
    Node::Leaf { size: Vec2::new(width, height) }
}

/// 横並びコンテナ（親の余剰幅を取りにいかない＝内容幅ぶんに収まる）。
pub fn row(justify: Justify, gap: f32, children: Vec<Node>) -> Node {
    Node::Row { justify, gap, grow: 0.0, children }
}

/// 横並びコンテナ（親の余剰幅を`grow`比で受け取って広がる。OPパネルの
/// 「eg_previewの右の残り幅を均等割り付け」のような入れ子の残余側に使う）。
pub fn row_grow(justify: Justify, gap: f32, children: Vec<Node>) -> Node {
    Node::Row { justify, gap, grow: 1.0, children }
}

/// 縦積みコンテナ（親の余剰幅を内容幅ぶんに収める＝子の最大幅）。子は左詰め。
pub fn stack(gap: f32, children: Vec<Node>) -> Node {
    Node::Stack { gap, grow: 0.0, align_center: false, children }
}

/// 縦積みコンテナ（親の余剰幅を`grow`比で受け取って広がる。子（`row`/`row_grow`）は
/// 既定の`align-items: stretch`でこの幅いっぱいに引き伸ばされる。`repeat`パネルの
/// 「グラフの右をN行の折り返しノブ群にする」のような、横幅の要る内容を複数行へ
/// 折り返す用途に使う）。子は左詰め。
pub fn stack_grow(gap: f32, children: Vec<Node>) -> Node {
    Node::Stack { gap, grow: 1.0, align_center: false, children }
}

/// [`stack`]の中央寄せ版。子の幅が揃わない（`<enum>`100pxと`<knob>`62px混在等）とき、
/// 幅の狭い子をスタックの幅（＝最大幅の子）の中央へ揃える。
pub fn stack_centered(gap: f32, children: Vec<Node>) -> Node {
    Node::Stack { gap, grow: 0.0, align_center: true, children }
}

/// [`stack_grow`]の中央寄せ版。
pub fn stack_grow_centered(gap: f32, children: Vec<Node>) -> Node {
    Node::Stack { gap, grow: 1.0, align_center: true, children }
}

fn build(tree: &mut TaffyTree<()>, node: &Node) -> NodeId {
    match node {
        Node::Leaf { size } => tree
            .new_leaf(Style {
                size: Size { width: length(size.x), height: length(size.y) },
                // taffy既定のflex_shrink=1.0だと、コンテナがleafの合計幅より狭いとき
                // taffyが計算する`layout.size`（内部の割当幅）が縮む。しかし`collect()`は
                // 縮んだ`layout.size`を使わずleafの宣言サイズをそのまま矩形として返すため
                // （このファイル下部`collect()`参照）、位置(`layout.location`)だけが
                // 縮んだ想定でずれ、ウィジェットの実描画サイズと食い違って重なって見える。
                // 0にして「縮まない」ことを保証し、狭ければ素直にコンテナからあふれさせる
                // （あふれた分はScrollAreaで見せる、呼び出し側の設計）。
                flex_shrink: 0.0,
                ..Default::default()
            })
            .unwrap(),
        Node::Row { justify, gap, grow, children } => {
            let kids: Vec<NodeId> = children.iter().map(|c| build(tree, c)).collect();
            tree.new_with_children(
                Style {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    justify_content: Some(justify.to_taffy()),
                    gap: Size { width: length(*gap), height: zero() },
                    flex_grow: *grow,
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                &kids,
            )
            .unwrap()
        }
        Node::Stack { gap, grow, align_center, children } => {
            let kids: Vec<NodeId> = children.iter().map(|c| build(tree, c)).collect();
            tree.new_with_children(
                Style {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    align_items: if *align_center { Some(AlignItems::Center) } else { None },
                    gap: Size { width: zero(), height: length(*gap) },
                    flex_grow: *grow,
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                &kids,
            )
            .unwrap()
        }
    }
}

fn collect(tree: &TaffyTree<()>, id: NodeId, parent_origin: Vec2, node: &Node, out: &mut Vec<Rect>) {
    let layout = tree.layout(id).unwrap();
    let origin = Vec2::new(parent_origin.x + layout.location.x, parent_origin.y + layout.location.y);
    match node {
        Node::Leaf { size } => {
            out.push(Rect { x: origin.x, y: origin.y, w: size.x, h: size.y });
        }
        Node::Row { children, .. } | Node::Stack { children, .. } => {
            let child_ids = tree.children(id).unwrap();
            for (cid, child) in child_ids.iter().zip(children) {
                collect(tree, *cid, origin, child, out);
            }
        }
    }
}

/// `container`サイズの枠内で木を解き、[`Node::Leaf`]の矩形をDFS順（＝木を書いた順）で返す。
/// 矩形はコンテナ左上を原点(0,0)とする相対座標。呼び出し側で実際の描画原点を足して使う。
pub fn solve(container: Vec2, root: &Node) -> Vec<Rect> {
    let mut tree: TaffyTree<()> = TaffyTree::new();
    let root_id = build(&mut tree, root);
    // rootには明示的な幅を与える。flex_grow(row_grow)は「親の余剰スペース」を分配する仕組みなので、
    // root自身がAuto(content-fit)サイズのままだと余剰が生まれずgrowが効かない
    // （compute_layoutのavailable_spaceは制約であって、root自身の幅指定の代わりにはならない）。
    let mut root_style = tree.style(root_id).unwrap().clone();
    root_style.size = Size { width: length(container.x), height: length(container.y) };
    tree.set_style(root_id, root_style).unwrap();
    tree.compute_layout(
        root_id,
        Size {
            width: AvailableSpace::Definite(container.x),
            height: AvailableSpace::Definite(container.y),
        },
    )
    .unwrap();
    let mut out = Vec::new();
    collect(&tree, root_id, Vec2::new(0.0, 0.0), root, &mut out);
    out
}
