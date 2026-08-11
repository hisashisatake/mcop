//! VGM から抽出したボイスを管理し、変換後パッチのバンクファイルを出力する。
//!
//! パッチ型は [PatchConverter] を実装した変換器（[X6Converter] 等）でジェネリック化されている
//! （vgm2x6本体は `.38x6` を出力する [X6Converter] を使う）。かつては`op505/tools/vgm2op505`が
//! 同じ[PatchBank]にOP505用変換器を差し込んで再利用していたが、op505デフォーク（2026-08-11）で
//! 独立複製済み（詳細はspec-fm.md 8章）。

use std::path::Path;

use mucom2x6::conv::OpnVoice;
use opm2x6::{conv, parse::{OpmVoice, OperatorOrder}};
use ym38x6_core::{PresetEntry, PresetFile, Ym38x6Patch};

/// ソースボイス → ターゲットパッチの変換器。vgm2x6本体は [X6Converter] を使う。
/// 戻り値の `Vec<String>` は変換警告（.38x6直接変換では常に空）。
pub trait PatchConverter {
    type Patch: Copy + Default + PartialEq;
    fn from_opm(&mut self, v: &OpmVoice) -> (Self::Patch, Vec<String>);
    fn from_opn(&mut self, v: &OpnVoice) -> (Self::Patch, Vec<String>);
    /// SSG合成パッチ（psg_patch/noise_patch/mix_patch、ssg.rs参照）用。これらはx6ネイティブ
    /// 定義の人工パッチであり実機レート由来ではないため、直接変換ツールもこの経路を使ってよい。
    fn from_x6(&mut self, p: &Ym38x6Patch) -> (Self::Patch, Vec<String>);
}

/// バンク内の1エントリー。
struct Entry<P> {
    /// OPM由来なら重複判定用にソースボイスを保持する（[timbre_eq] で比較）。
    opm_voice: Option<OpmVoice>,
    /// OPM以外（OPN・SSG固定パッチ）の重複判定キー。常に `.38x6` 形式で持つことで、
    /// x6変換で潰れる音色同士が変換器を問わず同じバンク番号に収束する
    /// （vgm2x6のバイト不変・直接変換ツールのSMFバイト一致の両方を成立させる要）。
    x6_key: Option<Ym38x6Patch>,
    name: String,
    patch: P,
    warnings: Vec<String>,
}

pub struct PatchBank<C: PatchConverter> {
    conv: C,
    entries: Vec<Entry<C::Patch>>,
}

impl<C: PatchConverter> PatchBank<C> {
    pub fn new(conv: C) -> Self {
        Self { conv, entries: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 指定インデックスの変換後パッチを返す（WAVレンダリング用）。
    pub fn patch_at(&self, idx: usize) -> C::Patch {
        self.entries[idx].patch
    }

    /// OpmVoice を登録し、重複していればそのインデックスを、新規なら追加してインデックスを返す。
    ///
    /// 音色同一性の判定: 音色パラメーター（KC/KF以外）が全て一致するか。
    pub fn find_or_insert(&mut self, voice: OpmVoice) -> usize {
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(v) = &e.opm_voice {
                if timbre_eq(v, &voice) {
                    return i;
                }
            }
        }
        let idx = self.entries.len();
        let (patch, warnings) = self.conv.from_opm(&voice);
        self.entries.push(Entry {
            opm_voice: Some(voice),
            x6_key: None,
            name: format!("patch{idx:03}"),
            patch,
            warnings,
        });
        idx
    }

    /// OpnVoice を登録し、重複していればそのインデックスを、新規なら追加してインデックスを返す。
    /// 重複判定キーは `voice.to_ym38x6_patch()`（[find_or_insert_fixed] と同一プールを共有）。
    pub fn find_or_insert_opn(&mut self, voice: &OpnVoice, name: &str) -> usize {
        let key = voice.to_ym38x6_patch();
        self.find_or_insert_x6_keyed(key, name, |c| c.from_opn(voice))
    }

    /// OPM由来でない固定パッチ（PSG矩形波など）を登録/再利用してインデックスを返す。
    /// 同一パッチが既に登録済みなら（`find_or_insert` の音色重複判定とは別に）そのまま再利用する。
    pub fn find_or_insert_fixed(&mut self, patch: Ym38x6Patch, name: &str) -> usize {
        self.find_or_insert_x6_keyed(patch, name, |c| c.from_x6(&patch))
    }

    /// [find_or_insert_opn]・[find_or_insert_fixed] 共通の「x6_keyで重複探索→無ければ変換して追加」処理。
    fn find_or_insert_x6_keyed(
        &mut self,
        key: Ym38x6Patch,
        name: &str,
        convert: impl FnOnce(&mut C) -> (C::Patch, Vec<String>),
    ) -> usize {
        for (i, e) in self.entries.iter().enumerate() {
            if e.x6_key == Some(key) {
                return i;
            }
        }
        let idx = self.entries.len();
        let (patch, warnings) = convert(&mut self.conv);
        self.entries.push(Entry {
            opm_voice: None,
            x6_key: Some(key),
            name: name.to_string(),
            patch,
            warnings,
        });
        idx
    }

    /// 全エントリーを (program番号, 名前, パッチ) の列で返す（外部クレートのバンク出力用。
    /// `.38x6`以外の出力形式は[X6Converter]専用の[write][PatchBank::write]を持たないため、
    /// vgm2op505等はこれで自前のバンクファイルを組み立てる）。
    pub fn entries(&self) -> impl Iterator<Item = (u8, &str, &C::Patch)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| ((i % 128) as u8, e.name.as_str(), &e.patch))
    }

    /// 変換警告が出たエントリーを (名前, 警告一覧) の列で返す（空エントリーは含まない）。
    pub fn warnings(&self) -> Vec<(&str, &[String])> {
        self.entries
            .iter()
            .filter(|e| !e.warnings.is_empty())
            .map(|e| (e.name.as_str(), e.warnings.as_slice()))
            .collect()
    }
}

/// `.38x6` 直接出力用の変換器。実機レート写像は行わず、既存のx6コンバーター関数を素通しする。
pub struct X6Converter;

impl PatchConverter for X6Converter {
    type Patch = Ym38x6Patch;

    fn from_opm(&mut self, v: &OpmVoice) -> (Ym38x6Patch, Vec<String>) {
        (conv::voice_to_patch(v, OperatorOrder::Direct), Vec::new())
    }
    fn from_opn(&mut self, v: &OpnVoice) -> (Ym38x6Patch, Vec<String>) {
        (v.to_ym38x6_patch(), Vec::new())
    }
    fn from_x6(&mut self, p: &Ym38x6Patch) -> (Ym38x6Patch, Vec<String>) {
        (*p, Vec::new())
    }
}

impl PatchBank<X6Converter> {
    /// .38x6 Presets バンクファイルを書き出す。
    pub fn write(&self, path: &Path, bank: u16) -> Result<(), String> {
        if self.entries.is_empty() {
            eprintln!("警告: 音色が抽出されませんでした");
            return Ok(());
        }
        let presets: Vec<PresetEntry> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| PresetEntry {
                program: (i % 128) as u8,
                name: e.name.clone(),
                patch: e.patch,
            })
            .collect();
        let file = PresetFile::Presets { bank, presets };
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
