//! taffyベースのレイアウト計算ヘルパー（XML UIレイアウトDSLの実行基盤）。
//!
//! ym38x6-uiのウィジェットは全て固定サイズ（knob=62×66 / eg_preview=84×66 /
//! waveform_selector=130×66 等）なので、それらの自然サイズをtaffyへ与えて**位置だけ**を
//! CSS Flexbox仕様準拠のアルゴリズムで計算させ、egui側は算出済み矩形に各ウィジェットを配置する。
//! これにより`justified_row`の手作業widths配列や、CHANNEL/CHIP LFO 2カラムの手動高さ合わせ
//! （`set_min_height`）といった、パネルごとに散らばっていた場当たり的なレイアウト算数を
//! 単一のエンジンへ集約する。
//!
//! 生成コードは[`Node`]で木を組み、[`solve`]で葉の矩形を得て、[`place`]で各ウィジェットを置く。
//!
//! 純粋な計算部分（木の構築・taffy呼び出し）は`panel-layout`クレートに committed
//! している（egui非依存、`build.rs`とブラウザ用wasmツールが同一コードとして共有する）。
//! ここではeguiとの橋渡し（[`solve`]/[`place`]の型変換）のみを行う。

pub use panel_layout::{leaf, row, row_grow, stack, Justify, Node};

/// [`panel_layout::solve`]を呼び、egui型（[`egui::Vec2`]/[`egui::Rect`]）で受け渡しする薄いラッパー。
pub fn solve(container: egui::Vec2, root: &Node) -> Vec<egui::Rect> {
    let container = panel_layout::Vec2::new(container.x, container.y);
    panel_layout::solve(container, root)
        .into_iter()
        .map(|r| egui::Rect::from_min_size(egui::pos2(r.x, r.y), egui::vec2(r.w, r.h)))
        .collect()
}

/// [`solve`]が返した相対矩形`r`を、描画原点`origin`ぶん平行移動した位置へウィジェットを配置する。
/// 中身は矩形左上詰め（`scope_builder`は親レイアウト＝top-down/leftを継承）で描かれ、
/// 現行の各セル（top-left配置）と同じ見え方になる。
pub fn place(ui: &mut egui::Ui, origin: egui::Pos2, r: egui::Rect, add: impl FnOnce(&mut egui::Ui)) {
    let abs = r.translate(origin.to_vec2());
    ui.scope_builder(egui::UiBuilder::new().max_rect(abs), |ui| add(ui));
}
