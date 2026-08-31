//! フェーズ0スパイク専用の簡易ログ。`%TEMP%\op505mme-spike.log`へ追記する。
//! 「winmm経由でDriverProc/modMessageが実際に呼ばれ、MIDIバイト列が届くか」を
//! 目視で確認するためだけの一時的な仕組み（後続フェーズで名前付きパイプ経由の
//! 実処理へ置き換わり次第、このモジュールごと撤去する）。

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

fn log_file() -> Option<&'static Mutex<std::fs::File>> {
    static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let path = std::env::temp_dir().join("op505mme-spike.log");
        OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    })
    .as_ref()
}

pub fn log(msg: &str) {
    let Some(file) = log_file() else { return };
    let Ok(mut file) = file.lock() else { return };
    let _ = writeln!(file, "[{:?}] {msg}", std::time::SystemTime::now());
}
