//! 左右Ctrlキーでオクターブを移動するためのブリッジ。
//!
//! egui(eframe 0.34のweb backend)は`event.code`（物理キー、ControlLeft/ControlRightの区別）を
//! 取得しない（`Event::Key::physical_key`のドキュメントに "eframe does not (yet) implement
//! this on web" と明記されている）。`Modifiers::ctrl`も左右を区別しない単一フラグのため、
//! egui標準の入力経路では左右Ctrlを判別できない。
//! そのため`main.js`側でブラウザの`KeyboardEvent.code`を直接見て、このモジュールが
//! `#[wasm_bindgen]`で公開する`notify_shift()`をJSから直接呼び出す形で橋渡しする
//! （関数名は`notify_shift`のままだが、実際のトリガーはCtrlキー。左右どちらを押したかだけが
//! 意味を持つため命名はそのままにしている）。

use std::cell::{Cell, RefCell};
use wasm_bindgen::prelude::*;

thread_local! {
    static LEFT_EDGE: Cell<bool> = const { Cell::new(false) };
    static RIGHT_EDGE: Cell<bool> = const { Cell::new(false) };
    /// 直近フレームの`egui::Context`。`notify_shift()`から即座に`request_repaint()`するために保持する
    /// （Ctrlのkeydownはeguiのcanvas入力経路を経由しないため、何もしないと次の自然な再描画まで
    /// フラグが反映されず体感的に「効かない」状態になる）。
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// `app.rs`の`ui()`が毎フレーム呼ぶ。`egui::Context`を最新に保つ。
pub fn register_context(ctx: &egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
}

/// `main.js`から呼ばれる。`side`は`"left"`/`"right"`。
#[wasm_bindgen]
pub fn notify_shift(side: &str) {
    match side {
        "left" => LEFT_EDGE.with(|c| c.set(true)),
        "right" => RIGHT_EDGE.with(|c| c.set(true)),
        _ => return,
    }
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

/// 左Ctrlの押下エッジを消費する（一度読んだらfalseに戻る）。
pub fn take_left_edge() -> bool {
    LEFT_EDGE.with(|c| c.replace(false))
}

/// 右Ctrlの押下エッジを消費する（一度読んだらfalseに戻る）。
pub fn take_right_edge() -> bool {
    RIGHT_EDGE.with(|c| c.replace(false))
}
