//! standalone/vst共通のUI設定（`%APPDATA%\op505\ui.json`）。
//!
//! `op505-editor`はserde非依存が意図的な防御（`Op505EgBank`をうっかり移設できない構造上の
//! 防御、詳細は`.claude/rules/crate-dependency-guard.md`）のため、設定ファイルの読み書きは
//! ここ（両ホストが既に依存しているserde+ファイルI/O持ちの`op505-core`）に置く。
//! standalone専用設定（MIDI入力ポート名等）を持つ`standalone.json`とは意味論が異なるため
//! 別ファイルにする（VSTがMIDI入力ポート設定を読むのは筋が合わない）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UiConfig {
    /// レベルメーターの描画更新レート（fps）。`None`または読み込み失敗時は
    /// `DEFAULT_METER_FPS`にフォールバックする。
    #[serde(default)]
    pub meter_fps: Option<u32>,
}

/// 設定ファイルが読めない・壊れている・未設定のときの更新レート。
/// レベルメーターは即時性を要求しないため低めに倒し、実機の負荷を見て
/// 調整できるよう定数一箇所に置く。
pub const DEFAULT_METER_FPS: u32 = 10;

/// メーター更新レートの許容範囲（下限1fps・上限60fps）。
const METER_FPS_MIN: u32 = 1;
const METER_FPS_MAX: u32 = 60;

fn config_path() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("op505")
        .join("ui.json")
}

/// 設定ファイルを読み込む。存在しない・壊れている場合は既定値（全フィールド未設定）を返す
/// （`standalone::config::load`と同じ「設定ファイルが無くても正常に起動できる」方針）。
pub fn load() -> UiConfig {
    load_from_path(&config_path())
}

fn load_from_path(path: &Path) -> UiConfig {
    let Ok(text) = std::fs::read_to_string(path) else {
        return UiConfig::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// `cfg.meter_fps`から実際に使う更新レートを決定する。`None`ならフォールバックし、
/// 範囲外の値は`[1, 60]`へクランプする（0fps＝無限待ちや、極端に高い値でのCPU浪費を防ぐ）。
pub fn meter_fps(cfg: &UiConfig) -> u32 {
    cfg.meter_fps.unwrap_or(DEFAULT_METER_FPS).clamp(METER_FPS_MIN, METER_FPS_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト間でファイルが衝突しないよう、呼び出しごとに一意な一時ファイルパスを作る。
    fn temp_config_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("op505_ui_config_test_{name}_{}.json", std::process::id()))
    }

    #[test]
    fn missing_file_returns_default() {
        let path = temp_config_path("missing");
        assert!(!path.exists());
        assert_eq!(load_from_path(&path), UiConfig::default());
    }

    #[test]
    fn corrupted_json_returns_default() {
        let path = temp_config_path("corrupted");
        std::fs::write(&path, "{ this is not valid json").unwrap();
        assert_eq!(load_from_path(&path), UiConfig::default());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn valid_json_round_trips() {
        let path = temp_config_path("valid");
        std::fs::write(&path, r#"{"meter_fps": 30}"#).unwrap();
        let cfg = load_from_path(&path);
        assert_eq!(cfg.meter_fps, Some(30));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_json_object_leaves_meter_fps_none() {
        let path = temp_config_path("empty_object");
        std::fs::write(&path, "{}").unwrap();
        let cfg = load_from_path(&path);
        assert_eq!(cfg.meter_fps, None);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn meter_fps_falls_back_when_unset() {
        let cfg = UiConfig { meter_fps: None };
        assert_eq!(meter_fps(&cfg), DEFAULT_METER_FPS);
    }

    #[test]
    fn meter_fps_uses_configured_value_within_range() {
        let cfg = UiConfig { meter_fps: Some(30) };
        assert_eq!(meter_fps(&cfg), 30);
    }

    #[test]
    fn meter_fps_clamps_zero_to_minimum() {
        let cfg = UiConfig { meter_fps: Some(0) };
        assert_eq!(meter_fps(&cfg), METER_FPS_MIN);
    }

    #[test]
    fn meter_fps_clamps_excessive_value_to_maximum() {
        let cfg = UiConfig { meter_fps: Some(1000) };
        assert_eq!(meter_fps(&cfg), METER_FPS_MAX);
    }
}
