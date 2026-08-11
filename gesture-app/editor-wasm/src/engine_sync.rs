//! main.js（メイン画面のEngineセレクト・Bank/Program欄）から、今どちらのエンジンがアクティブか、
//! そして選択中のbank/programが変わったことをエディタへ伝えるためのブリッジ。`shift_keys.rs`/
//! `program_sync.rs`と同じ「JSから`#[wasm_bindgen]`関数を直接呼ぶ」パターン。
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
    /// main.js側でエンジン切替またはBank/Program欄の変更が起きた際に立てるフラグ。`app.rs`の
    /// `ui()`が毎フレーム冒頭でこれを見て、立っていれば現在の`patch_target()`
    /// （切替後の新しいアクティブエンジン）で`read_program_fields()`を読み直し`handle_navigate`する
    /// （PRESETSサイドバーを常にメイン画面のBank/Program欄と一致させるための唯一の経路。
    /// 旧`op505_state::PATCH_STALE`はこれに統合された）。
    static SELECTION_STALE: Cell<bool> = const { Cell::new(false) };
}

/// `main.js`のEngineセレクトのchangeハンドラから呼ばれる。
#[wasm_bindgen]
pub fn notify_engine(engine_id: u8) {
    ACTIVE_ENGINE.with(|c| c.set(engine_id));
    SELECTION_STALE.with(|c| c.set(true));
    crate::shift_keys::request_repaint();
}

/// `main.js`のBank欄/Program欄のinputハンドラ（`applyProgram()`）から呼ばれる。
#[wasm_bindgen]
pub fn notify_selection_changed() {
    SELECTION_STALE.with(|c| c.set(true));
    crate::shift_keys::request_repaint();
}

/// `app.rs`側から、今どちらのパネルを描くか判定するために読む。
pub fn active_engine() -> u8 {
    ACTIVE_ENGINE.with(|c| c.get())
}

/// `app.rs::ui()`が毎フレーム冒頭で呼ぶ。フラグが立っていれば消費してtrueを返す
/// （立っていなければ通常フレームのコストはゼロに近い）。
pub fn take_selection_stale() -> bool {
    SELECTION_STALE.with(|c| c.replace(false))
}
