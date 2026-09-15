// 独自プロジェクトファイル（.gap505）のOpen/Save/Save As。
//
// ファイルの中身（コード履歴・リズムパターン・メロディノート等）はgesture-appの
// エンジン非依存な状態であり、実データはフロントエンド（JS）側にしか無い。ここでは
// JSが組み立てたJSON文字列をそのまま読み書きするだけで、スキーマには一切関知しない。
//
// rfdのファイルダイアログはブロッキングAPIのため`spawn_blocking`で別スレッド実行し、
// Tauriのasync fnコマンドとしてそのままawaitで結果を返す（op505-editorのegui版が使う
// 「別スレッド+mpscポーリング」パターンは、Tauri側は直接returnできるため不要）。

use std::fs;
use std::path::PathBuf;

#[derive(serde::Serialize)]
pub struct ProjectFileDto {
    path: String,
    json: String,
}

fn pick_open_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("op505 Project", &["gap505"])
        .pick_file()
}

fn pick_save_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("op505 Project", &["gap505"])
        .set_file_name("untitled.gap505")
        .save_file()
}

/// ファイルを開くダイアログを出し、選ばれたファイルの内容を読んで返す。
/// キャンセル時・読み込み失敗時は`None`。
#[tauri::command]
pub async fn open_project() -> Option<ProjectFileDto> {
    let path = tauri::async_runtime::spawn_blocking(pick_open_path)
        .await
        .ok()??;
    let json = fs::read_to_string(&path).ok()?;
    Some(ProjectFileDto {
        path: path.to_string_lossy().into_owned(),
        json,
    })
}

/// 保存先を選ぶダイアログを出し、選ばれたパスへ書き込む（Save As）。
/// 書き込めたパスを返す（キャンセル・失敗時は`None`）。
#[tauri::command]
pub async fn save_project_as(json: String) -> Option<String> {
    let path = tauri::async_runtime::spawn_blocking(pick_save_path)
        .await
        .ok()??;
    fs::write(&path, json).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// 既知のパスへ上書き保存する（Save、ダイアログ無し）。
#[tauri::command]
pub fn save_project_to(path: String, json: String) -> bool {
    fs::write(&path, json).is_ok()
}
