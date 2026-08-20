//! main.js（メイン画面のBank/Program欄）から、選択中のbank/programが変わったことを
//! エディタへ伝えるためのブリッジ。`shift_keys.rs`/`program_sync.rs`と同じ
//! 「JSから`#[wasm_bindgen]`関数を直接呼ぶ」パターン。

use std::cell::Cell;
use wasm_bindgen::prelude::*;

thread_local! {
    /// main.js側でBank/Program欄の変更が起きた際に立てるフラグ。`app.rs`の`ui()`が毎フレーム
    /// 冒頭でこれを見て、立っていれば`read_program_fields()`を読み直し`handle_navigate`する
    /// （PRESETSサイドバーを常にメイン画面のBank/Program欄と一致させるための唯一の経路）。
    static SELECTION_STALE: Cell<bool> = const { Cell::new(false) };
}

/// `main.js`のBank欄/Program欄のinputハンドラ（`applyProgram()`）から呼ばれる。
#[wasm_bindgen]
pub fn notify_selection_changed() {
    SELECTION_STALE.with(|c| c.set(true));
    crate::shift_keys::request_repaint();
}

/// `app.rs::ui()`が毎フレーム冒頭で呼ぶ。フラグが立っていれば消費してtrueを返す
/// （立っていなければ通常フレームのコストはゼロに近い）。
pub fn take_selection_stale() -> bool {
    SELECTION_STALE.with(|c| c.replace(false))
}
