//! ファイルベースの簡易ログ（`%LOCALAPPDATA%\op505\standalone.log`）。
//!
//! `windows_subsystem = "windows"`化に伴いコンソールが無くなり、`println!`/`eprintln!`は
//! 標準出力ハンドルが無効な環境（Explorerからの起動等）で**パニックする**危険がある
//! （`print!`系マクロは書き込み失敗時に`panic!`する実装のため）。そのため実行時ログは
//! 全てここへ統一する。オーディオコールバック内のログ（旧`handle_midi_message`の
//! note on/off等）はファイル出力へ切り替えるのではなく削除した——ロック+ファイルI/Oは
//! 元々リアルタイム安全ではなかったため（`op505-mme-driver`の同名モジュールと同じ設計）。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn log_path() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("op505")
        .join("standalone.log")
}

fn log_file() -> Option<&'static Mutex<std::fs::File>> {
    static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let path = log_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    })
    .as_ref()
}

pub fn log(msg: &str) {
    let Some(file) = log_file() else { return };
    let Ok(mut file) = file.lock() else { return };
    let _ = writeln!(file, "[{:?}] {msg}", std::time::SystemTime::now());
}

/// ログファイルの絶対パスを返す（トレイメニューの「ログを開く」で使う）。
pub fn path() -> PathBuf {
    log_path()
}
