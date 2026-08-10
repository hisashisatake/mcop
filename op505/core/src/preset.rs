//! `.op505`バンクファイルの内容。`.38x6`（ym38x6-core/src/preset.rs）の
//! `PresetEntry`/`PresetFile`と同形のJSON構造を採用し、将来の`PresetBank`相当の
//! ローダー実装（gesture-app配線）を阻害しないようにする。ローダー自体は今回は実装しない
//! （op505_probe等の単発変換ツールと、opz2op505のようなバンク一括変換ツールが書き手）。

use serde::{Deserialize, Serialize};

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
}
