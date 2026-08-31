//! 常駐設定の永続化。`%APPDATA%\op505\standalone.json`（無ければ既定値で起動する）。
//! stdinでの対話選択を廃止した代わりに、選択したMIDI入力ポート名をここへ保存する想定
//! （フェーズ3のトレイメニューが書き込む。インデックスではなく名前で持つのは、
//! 機器の抜き差しでインデックスがずれるため）。`op505_presets_dir()`（op505-core）と
//! 同じ`%APPDATA%\op505\`配下に置く。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct StandaloneConfig {
    /// 起動時に自動接続するmidir入力ポート名（完全一致）。未設定なら
    /// 「ポートがちょうど1個だけの場合に限り自動接続」にフォールバックする
    /// （[`crate::sources::midir_src::connect`]参照）。
    #[serde(default)]
    pub midi_in_port: Option<String>,
}

fn config_path() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("op505")
        .join("standalone.json")
}

/// 設定ファイルを読み込む。存在しない・壊れている場合は既定値（全フィールド未設定）を返す
/// （設定ファイルが無くても正常に起動できることが前提のため、エラーにはしない）。
pub fn load() -> StandaloneConfig {
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return StandaloneConfig::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        crate::log::log(&format!("設定ファイル {} の読み込みに失敗しました（既定値を使用）: {err}", path.display()));
        StandaloneConfig::default()
    })
}

/// 設定ファイルを保存する。ディレクトリが無ければ作成する（タスクトレイのメニュー操作
/// で選択したMIDI入力ポート名を永続化するために使う）。
pub fn save(config: &StandaloneConfig) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(dir) {
            crate::log::log(&format!("設定ディレクトリ {} の作成に失敗しました: {err}", dir.display()));
            return;
        }
    }
    match serde_json::to_string_pretty(config) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                crate::log::log(&format!("設定ファイル {} の保存に失敗しました: {err}", path.display()));
            }
        }
        Err(err) => crate::log::log(&format!("設定のシリアライズに失敗しました: {err}")),
    }
}

/// 設定ファイルの絶対パスを返す（トレイメニューの「設定フォルダを開く」で使う）。
pub fn file_path() -> PathBuf {
    config_path()
}
