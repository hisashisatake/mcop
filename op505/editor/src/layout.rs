//! PRESETSドロワー幅とエディタ最小幅の正本。VST/standalone両ホストが同じ値を使う
//! （幅がずれるとPRESETSドロワーの見た目がホスト間で食い違う）。
//! 高さは各ホストの事情で異なる（VSTは`ResizableWindow`の最小サイズ480px＋内部スクロール、
//! standaloneは鍵盤・Edit Channelセレクタぶんを含めた固定サイズ720px）ため、
//! [`editor_min_size`]は呼び出し側から高さを受け取る。

/// PRESETSドロワー（ハンバーガーメニューで開閉するオーバーレイ）の幅。
/// オーバーレイのためエディタ最小幅には加算しない（[`editor_min_width`]参照、コントロール一式に
/// 覆いかぶさる方式のため場所を確保する必要が無い）。
pub const PRESETS_SIDEBAR_WIDTH: f32 = 200.0;

/// `ResizableWindow`自身と`CentralPanel::default()`（`draw_op505_panel`を包む方）の
/// `inner_margin(8)`が左右で効く分の安全マージン（実測ではなく余裕を見た概算値）。
const WINDOW_CHROME_SLACK: f32 = 40.0;

/// エディタウィンドウの最小幅。`op505_ui::PANEL_MIN_WIDTH`（panel.xmlから算出した
/// 「ノブ等がtime-eg-editorへ食い込まずに収まる最小幅」）そのもの（+安全マージン）。
/// PRESETSドロワーはオーバーレイ表示のため場所を確保しない——常時表示のサイドバーだった頃は
/// ここに`PRESETS_SIDEBAR_WIDTH`を加算していたが、2026-09-03のオーバーレイ化で不要になった。
/// panel.xmlの内容が変わればここも自動追従する。
pub fn editor_min_width() -> f32 {
    op505_ui::PANEL_MIN_WIDTH + WINDOW_CHROME_SLACK
}

/// `editor_min_width()`と、呼び出し側が指定する`height`を組にする。
pub fn editor_min_size(height: f32) -> (f32, f32) {
    (editor_min_width(), height)
}
