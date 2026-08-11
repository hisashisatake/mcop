//! main.js（メイン画面のEngineセレクト）から、今どちらのエンジンがアクティブかをエディタへ
//! 伝えるためのブリッジ。`shift_keys.rs`/`program_sync.rs`と同じ「JSから`#[wasm_bindgen]`関数を
//! 直接呼ぶ」パターン。
//!
//! エディタは常時両方のパネル用状態（`state::EditorState`と`op505_state::Op505State`）を保持し、
//! `app.rs`の`ui()`がこのフラグを見てどちらのパネルを描くか・どちらのIPCコマンドへ送るかを
//! 切り替える（gesture-app/src-tauriのEngines同様、状態自体は両方生かしたまま演奏/表示だけ
//! 切り替える設計）。

use std::cell::Cell;
use wasm_bindgen::prelude::*;

thread_local! {
    /// 0=OP505 / 1=38x6。既定はOP505（main.jsの`<select id="engine-select">`初期値と一致）。
    static ACTIVE_ENGINE: Cell<u8> = const { Cell::new(0) };
}

/// `main.js`のEngineセレクトのchangeハンドラから呼ばれる。
#[wasm_bindgen]
pub fn notify_engine(engine_id: u8) {
    ACTIVE_ENGINE.with(|c| c.set(engine_id));
    crate::shift_keys::request_repaint();
}

/// `app.rs`側から、今どちらのパネルを描くか判定するために読む。
pub fn active_engine() -> u8 {
    ACTIVE_ENGINE.with(|c| c.get())
}
