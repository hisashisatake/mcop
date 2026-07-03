//! main.js（メイン画面のBank/Program欄）へ、エディタが今編集しているbank/programを
//! 伝えるためのブリッジ。`shift_keys.rs`と同じ「JSから`#[wasm_bindgen]`関数を直接呼ぶ」パターン。
//!
//! リアルタイム同期はしない。main.jsはエディタを**閉じる瞬間**にだけ`get_current_program()`を
//! 呼び、`#program-bank`/`#program-num`欄を追従させる（ユーザー確認済みの仕様）。

use std::cell::RefCell;
use wasm_bindgen::prelude::*;

thread_local! {
    static CURRENT: RefCell<(u16, u8)> = const { RefCell::new((0, 0)) };
}

/// `app.rs`側から、bank/programが変わるたびに呼ぶ（JSには公開しない）。
pub fn set_current(bank: u16, program: u8) {
    CURRENT.with(|c| *c.borrow_mut() = (bank, program));
}

/// `main.js`のtoggleEditor()がエディタを閉じる瞬間に呼ぶ。JSONで`{"bank":..,"program":..}`を返す。
#[wasm_bindgen]
pub fn get_current_program() -> String {
    let (bank, program) = CURRENT.with(|c| *c.borrow());
    serde_json::to_string(&serde_json::json!({ "bank": bank, "program": program })).unwrap()
}
