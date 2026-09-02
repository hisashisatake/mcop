//! PRESETSサイドバー幅とエディタ最小幅の正本。VST/standalone両ホストが同じ値を使う
//! （幅がずれるとPRESETSパネルの折り返し方等の見た目がホスト間で食い違う）。
//! 高さは各ホストの事情で異なる（VSTは`ResizableWindow`の最小サイズ480px＋内部スクロール、
//! standaloneは鍵盤・Edit Channelセレクタぶんを含めた固定サイズ720px）ため、
//! [`editor_min_size`]は呼び出し側から高さを受け取る。

/// PRESETSサイドバー（固定幅）の幅。Open/Save/Save Asの3ボタンが折り返さず並ぶ幅。
pub const PRESETS_SIDEBAR_WIDTH: f32 = 200.0;

/// `ResizableWindow`自身と`CentralPanel::default()`（`draw_op505_panel`を包む方）の
/// `inner_margin(8)`が左右で効く分の安全マージン（実測ではなく余裕を見た概算値）。
const WINDOW_CHROME_SLACK: f32 = 40.0;

/// エディタウィンドウの最小幅。`op505_ui::PANEL_MIN_WIDTH`（panel.xmlから算出した
/// 「ノブ等がtime-eg-editorへ食い込まずに収まる最小幅」）にPRESETSサイドバーぶんを足したもの。
/// panel.xmlの内容が変わればここも自動追従する。
pub fn editor_min_width() -> f32 {
    op505_ui::PANEL_MIN_WIDTH + PRESETS_SIDEBAR_WIDTH + WINDOW_CHROME_SLACK
}

/// `editor_min_width()`と、呼び出し側が指定する`height`を組にする。
pub fn editor_min_size(height: f32) -> (f32, f32) {
    (editor_min_width(), height)
}
