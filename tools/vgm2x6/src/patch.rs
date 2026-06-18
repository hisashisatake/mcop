//! VGM から抽出したOPM音色を管理し、.38x6 バンクファイルを出力する。

use std::path::Path;

use opm2x6::{conv, parse::{OpmVoice, OperatorOrder}};
use ym38x6_core::{PresetEntry, PresetFile};

pub struct PatchBank {
    voices: Vec<OpmVoice>,
    entries: Vec<PresetEntry>,
}

impl PatchBank {
    pub fn new() -> Self {
        Self { voices: Vec::new(), entries: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.voices.len()
    }

    /// OpmVoice を登録し、重複していればそのインデックスを、新規なら追加してインデックスを返す。
    ///
    /// 音色同一性の判定: 音色パラメーター（KC/KF以外）が全て一致するか。
    pub fn find_or_insert(&mut self, voice: OpmVoice) -> usize {
        for (i, v) in self.voices.iter().enumerate() {
            if timbre_eq(v, &voice) {
                return i;
            }
        }
        let idx = self.voices.len();
        let mut entry = conv::voice_to_entry(&voice, OperatorOrder::Direct);
        entry.program = (idx % 128) as u8;
        entry.name = format!("patch{idx:03}");
        self.entries.push(entry);
        self.voices.push(voice);
        idx
    }

    /// .38x6 Presets バンクファイルを書き出す。
    pub fn write(&self, path: &Path, bank: u16) -> Result<(), String> {
        if self.entries.is_empty() {
            eprintln!("警告: 音色が抽出されませんでした");
            return Ok(());
        }
        let file = PresetFile::Presets { bank, presets: self.entries.clone() };
        let json = file.to_json().map_err(|e| format!(".38x6 シリアライズに失敗: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// 音色パラメーターが同一かどうかを比較する（KC/KF・名前・番号は除く）。
fn timbre_eq(a: &OpmVoice, b: &OpmVoice) -> bool {
    a.fl == b.fl
        && a.con == b.con
        && a.ams == b.ams
        && a.pms == b.pms
        && a.slot == b.slot
        && a.lfrq == b.lfrq
        && a.amd == b.amd
        && a.pmd == b.pmd
        && op_eq(&a.m1, &b.m1)
        && op_eq(&a.c1, &b.c1)
        && op_eq(&a.m2, &b.m2)
        && op_eq(&a.c2, &b.c2)
}

fn op_eq(a: &opm2x6::parse::OpmOpReg, b: &opm2x6::parse::OpmOpReg) -> bool {
    a.ar == b.ar
        && a.d1r == b.d1r
        && a.d2r == b.d2r
        && a.rr == b.rr
        && a.d1l == b.d1l
        && a.tl == b.tl
        && a.ks == b.ks
        && a.mul == b.mul
        && a.dt1 == b.dt1
        && a.dt2 == b.dt2
        && a.ams_en == b.ams_en
}
