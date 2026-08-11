//! `.op505`バンクファイルの内容。`.38x6`（ym38x6-core/src/preset.rs）の
//! `PresetEntry`/`PresetFile`と同形のJSON構造を採用する。`Op505PresetBank`は
//! `ym38x6_core::PresetBank`と同じ意味論の独立実装（ym38x6-coreへは一切依存しない）。
//! ソースの書き手はop505_probe等の単発変換ツールと、opz2op505のようなバンク一括変換ツール。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::Op505Patch;

/// `.op505`ファイル内の1プリセット。`bank`は`Op505PresetFile`側で指定され、
/// `program`（Program Change、0〜127）でアドレスを持つ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op505PresetEntry {
    pub program: u8,
    pub name: String,
    pub patch: Op505Patch,
}

/// `.op505`ファイルの内容。`bank`（Bank Select相当、CC0×128+CC32、0〜16383）と、
/// `presets`/`programs`いずれかのエントリー配列を持つ（`ym38x6_core::PresetFile`と同じ意味論）。
/// - `Presets`（`{"bank":..,"presets":[...]}`）: ロード時にこの`bank`のプリセットのみ
///   初期化して、これらのエントリーで再構築する（他bankは保持される）
/// - `Programs`（`{"bank":..,"programs":[...]}`）: 初期化せず、(bank,program)単位で
///   これらのエントリーを上書きマージする
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Op505PresetFile {
    Presets { bank: u16, presets: Vec<Op505PresetEntry> },
    Programs { bank: u16, programs: Vec<Op505PresetEntry> },
}

impl Op505PresetFile {
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

/// ユーザープリセットの読み込み元ディレクトリ（`ym38x6_core::presets_dir`のOP505版）。
/// `%APPDATA%\op505\presets`が存在すればそちらを使い、無ければExplorerで見つけやすい
/// `%USERPROFILE%\Documents\op505\presets`にフォールバックする。
pub fn op505_presets_dir() -> PathBuf {
    let appdata_dir = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("op505")
        .join("presets");
    if appdata_dir.is_dir() {
        return appdata_dir;
    }
    std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Documents")
        .join("op505")
        .join("presets")
}

/// 1音色分のプリセット（名前 + パッチ本体）。`Op505PresetBank`が`(bank, program)`ごとに
/// 保持する内部値型（`ym38x6_core::Preset`のOP505版）。
#[derive(Clone, Debug)]
pub struct Op505Preset {
    pub name: String,
    pub patch: Op505Patch,
}

/// ディレクトリから読み込んだ`.op505`プリセット集合。Bank Select + Program Changeでの
/// ルックアップに使う（`ym38x6_core::PresetBank`と同じ意味論）。
/// `patch_for_program`のようなフォールバックチェーンは持たない。`.38x6`と違い
/// 波形メモリ/GM2/プレースホルダーの代替パッチが無いため、`Op505Patch::default()`
/// （`tl=0`で無音）を黙って返すより「見つからない」を`get`のNoneで素直に伝える。
#[derive(Clone, Debug, Default)]
pub struct Op505PresetBank {
    presets: HashMap<(u16, u8), Op505Preset>,
}

impl Op505PresetBank {
    /// 指定ディレクトリ内の`.op505`ファイルをファイル名昇順で読み込み、
    /// `Op505PresetFile::Presets`/`Programs`の意味に従ってプリセット集合を構築する。
    /// - `Presets`: そのbankのプリセットのみ初期化し、これらのエントリーで再構築する
    /// - `Programs`: 初期化せず、(bank,program)単位でエントリーを上書きマージする
    /// 同じ(bank,program)が複数回指定された場合は、後から読み込まれたものが優先する。
    /// ディレクトリが存在しない・読めない場合は空の集合を返す。
    pub fn load_from_dir(dir: &Path) -> Self {
        let mut presets = HashMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self { presets };
        };

        let mut paths: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("op505"))
            .collect();
        paths.sort();

        for path in paths {
            let Ok(json) = std::fs::read_to_string(&path) else { continue };
            let Ok(file) = Op505PresetFile::from_json(&json) else { continue };
            match file {
                Op505PresetFile::Presets { bank, presets: list } => {
                    presets.retain(|&(b, _), _| b != bank);
                    for entry in list {
                        presets.insert((bank, entry.program), Op505Preset { name: entry.name, patch: entry.patch });
                    }
                }
                Op505PresetFile::Programs { bank, programs: list } => {
                    for entry in list {
                        presets.insert((bank, entry.program), Op505Preset { name: entry.name, patch: entry.patch });
                    }
                }
            }
        }
        Self { presets }
    }

    pub fn get(&self, bank: u16, program: u8) -> Option<&Op505Preset> {
        self.presets.get(&(bank, program))
    }

    /// 全プリセットを (bank, program) 昇順でソートして返す（GUI一覧表示用）。
    pub fn sorted_entries(&self) -> Vec<((u16, u8), Op505Preset)> {
        let mut entries: Vec<_> = self.presets.iter().map(|(&k, v)| (k, v.clone())).collect();
        entries.sort_by_key(|(k, _)| *k);
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_patch(tl: u8) -> Op505Patch {
        let mut patch = Op505Patch::default();
        patch.operators[0].tl = tl;
        patch
    }

    #[test]
    fn preset_file_presets_json_round_trip() {
        let file = Op505PresetFile::Presets {
            bank: 1,
            presets: vec![
                Op505PresetEntry { program: 0, name: "A".to_string(), patch: sample_patch(200) },
                Op505PresetEntry { program: 1, name: "B".to_string(), patch: sample_patch(100) },
            ],
        };
        let json = file.to_json().expect("serialize");
        assert!(json.contains("\"bank\""));
        assert!(json.contains("\"presets\""));
        let loaded = Op505PresetFile::from_json(&json).expect("deserialize");
        assert_eq!(loaded, file);
    }

    #[test]
    fn preset_file_programs_json_round_trip() {
        let file = Op505PresetFile::Programs {
            bank: 1,
            programs: vec![Op505PresetEntry { program: 5, name: "C".to_string(), patch: sample_patch(255) }],
        };
        let json = file.to_json().expect("serialize");
        assert!(json.contains("\"bank\""));
        assert!(json.contains("\"programs\""));
        let loaded = Op505PresetFile::from_json(&json).expect("deserialize");
        assert_eq!(loaded, file);
    }

    /// `ym38x6_core::preset`のテストと同じ一時ディレクトリ運用（一意な名前・都度クリーンアップ）。
    fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("op505_preset_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn preset_bank_load_from_dir_reads_matching_files_and_ignores_others() {
        let dir = unique_temp_dir("basic");

        let file = Op505PresetFile::Presets {
            bank: 1,
            presets: vec![Op505PresetEntry { program: 64, name: "My Preset".to_string(), patch: sample_patch(64) }],
        };
        std::fs::write(dir.join("preset.op505"), file.to_json().unwrap()).unwrap();
        std::fs::write(dir.join("ignore.txt"), "not a preset").unwrap();

        let bank = Op505PresetBank::load_from_dir(&dir);
        std::fs::remove_dir_all(&dir).expect("cleanup temp dir");

        let loaded = bank.get(1, 64).expect("preset should be loaded");
        assert_eq!(loaded.name, "My Preset");
        assert_eq!(loaded.patch, sample_patch(64));
        assert!(bank.get(0, 0).is_none());
    }

    #[test]
    fn preset_bank_load_from_dir_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("op505_preset_test_nonexistent_dir");
        let bank = Op505PresetBank::load_from_dir(&dir);
        assert!(bank.get(0, 0).is_none());
    }

    #[test]
    fn preset_bank_load_from_dir_presets_reset_scoped_to_bank_and_programs_merge() {
        let dir = unique_temp_dir("merge");

        let bank1_base = Op505PresetFile::Presets {
            bank: 1,
            presets: vec![
                Op505PresetEntry { program: 0, name: "A".to_string(), patch: sample_patch(1) },
                Op505PresetEntry { program: 1, name: "B".to_string(), patch: sample_patch(2) },
            ],
        };
        let bank2_base = Op505PresetFile::Presets {
            bank: 2,
            presets: vec![Op505PresetEntry { program: 0, name: "X".to_string(), patch: sample_patch(3) }],
        };
        let bank1_override = Op505PresetFile::Programs {
            bank: 1,
            programs: vec![Op505PresetEntry { program: 1, name: "B2".to_string(), patch: sample_patch(4) }],
        };
        std::fs::write(dir.join("00_bank1_base.op505"), bank1_base.to_json().unwrap()).unwrap();
        std::fs::write(dir.join("01_bank2_base.op505"), bank2_base.to_json().unwrap()).unwrap();
        std::fs::write(dir.join("02_bank1_override.op505"), bank1_override.to_json().unwrap()).unwrap();

        let bank = Op505PresetBank::load_from_dir(&dir);
        assert_eq!(bank.get(1, 0).unwrap().name, "A");
        assert_eq!(bank.get(1, 1).unwrap().name, "B2");
        assert_eq!(bank.get(2, 0).unwrap().name, "X");

        let bank1_reset = Op505PresetFile::Presets {
            bank: 1,
            presets: vec![Op505PresetEntry { program: 5, name: "C".to_string(), patch: sample_patch(5) }],
        };
        std::fs::write(dir.join("03_bank1_reset.op505"), bank1_reset.to_json().unwrap()).unwrap();

        let bank = Op505PresetBank::load_from_dir(&dir);
        std::fs::remove_dir_all(&dir).expect("cleanup temp dir");

        assert!(bank.get(1, 0).is_none());
        assert!(bank.get(1, 1).is_none());
        assert_eq!(bank.get(1, 5).unwrap().name, "C");
        assert_eq!(bank.get(2, 0).unwrap().name, "X");
    }

    #[test]
    fn sorted_entries_are_ordered_by_bank_then_program() {
        let dir = unique_temp_dir("sorted");
        let file = Op505PresetFile::Presets {
            bank: 1,
            presets: vec![
                Op505PresetEntry { program: 5, name: "Five".to_string(), patch: sample_patch(5) },
                Op505PresetEntry { program: 0, name: "Zero".to_string(), patch: sample_patch(0) },
            ],
        };
        std::fs::write(dir.join("a.op505"), file.to_json().unwrap()).unwrap();

        let bank = Op505PresetBank::load_from_dir(&dir);
        std::fs::remove_dir_all(&dir).expect("cleanup temp dir");

        let entries = bank.sorted_entries();
        assert_eq!(entries.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec![(1, 0), (1, 5)]);
    }
}
