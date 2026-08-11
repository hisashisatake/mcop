//! VGM から抽出したボイスを管理し、変換後パッチのバンクを構築する。
//!
//! 由来: ym38x6/tools/vgm2x6/src/patch.rs（コミット535604f時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//!
//! **C2-2でジェネリクスを畳み込み済み**: vgm2x6は`.38x6`出力（`X6Converter`）とOP505直接
//! 出力（`Op505Converter`）を同じ`PatchBank<C: PatchConverter>`で使い分けていたが、
//! vgm2op505は`Op505Converter`しか使わないため、`PatchConverter`トレイトを撤廃し
//! `PatchBank`を`Op505Patch`/`Op505Converter`に対して具体化した（C2-1時点は複製の正しさを
//! レビューしやすくするためジェネリクスを保持していた）。
//!
//! 旧実装との最大の違いは重複判定キー。旧実装は`x6_key: Option<Ym38x6Patch>`
//! （OPNボイス・SSG固定パッチの両方を`.38x6`へ変換した結果を構造体等値比較する共有プール）
//! を使っていたが、ym38x6型に触れられなくなったため[`DedupKey`]（案B）へ置き換えた。
//!
//! [`DedupKey::Opn`]は`Ym38x6Patch`が実際に比較していたフィールド（実機レートを
//! `mucom2op505::map`の写像関数へ通した後の値。定数フィールドは比較に寄与しないため
//! 省いた）を素直な構造体として持つ。これにより「dt1=0と4は無デチューンへ収束する
//! 冗長エンコード」のような旧`x6_key`の衝突（`tests/golden.rs`の
//! `dt1_zero_and_four_collide_under_dedup_key`が回帰テスト）を1ビットも変えずに再現する。
//! [`DedupKey::Fixed`]はSSG人工パッチ（[crate::ssg::psg_patch]等）の識別子で、
//! enumのバリアントが異なるため`Opn`側と構造的に絶対衝突しない
//! （旧実装が「OPNのx6パッチは常にwaveform=0、SSG人工パッチはwaveform=16/32+np」という
//! 値の作り分けで担保していた不衝突性を、型で保証する形に置き換えた）。

use mucom2op505::map::{self, OpnVoice};
use op505_core::Op505Patch;
use opm2op505::parse::{OpmOpReg, OpmVoice};

use crate::convert::Op505Converter;

// ---------------------------------------------------------------------------
// 重複判定キー（案B: ym38x6型に触れないバイト等価キー）
// ---------------------------------------------------------------------------

/// OPNオペレーター1個分の、重複判定に寄与するフィールドのみを持つ構造体。
/// `mucom2op505::map`の写像関数を通した後の値（旧`to_ym38x6_patch`が生成していた
/// `OperatorParams`の可変フィールドと同じ集合）。`velocity_sensitivity`等の定数フィールドは
/// どのボイスでも同じ値になるため比較に寄与せず、意図的に省いている。
#[derive(Clone, Copy, PartialEq)]
struct OpnDedupOp {
    tl: u8,
    ar: u8,
    d1r: u8,
    d2r: u8,
    d1l: u8,
    rr: u8,
    mul: u8,
    dt1: u8,
    ksr: u8,
    am_enable: bool,
}

/// OPNボイス1個分の重複判定キー。
#[derive(Clone, Copy, PartialEq)]
pub struct OpnDedupKey {
    operators: [OpnDedupOp; 4],
    algorithm: u8,
    feedback: u8,
}

impl OpnDedupKey {
    fn from_voice(v: &OpnVoice) -> Self {
        let operators = std::array::from_fn(|i| {
            let op = &v.operators[i];
            OpnDedupOp {
                tl: map::tl_reg_to_level(op.tl),
                ar: map::ar_to_eg_rate(op.ar, op.ks),
                d1r: map::rate_to_eg_rate(op.d1r, op.ks),
                d2r: map::rate_to_eg_rate(op.d2r, op.ks),
                d1l: map::sl_to_eg_level(op.d1l),
                rr: map::rr_to_eg_rate(op.rr, op.ks),
                mul: op.mul.min(15),
                dt1: map::dt1_reg_to_detune(op.dt1),
                ksr: map::ks_to_ksr(op.ks),
                am_enable: op.am_enable,
            }
        });
        Self {
            operators,
            algorithm: v.algorithm.min(7),
            feedback: map::scale_3bit(v.feedback.min(7)),
        }
    }
}

/// SSG人工パッチ（[crate::ssg]）の識別子。`np`込みで比較するため、ノイズ周期違いは
/// 別エントリーになる（旧実装がwaveform=32+npの違いで担保していたのと同じ粒度）。
#[derive(Clone, Copy, PartialEq)]
pub enum SsgPatchId {
    Psg,
    Noise(u8),
    Mix(u8),
}

#[derive(Clone, Copy, PartialEq)]
enum DedupKey {
    Opn(OpnDedupKey),
    Fixed(SsgPatchId),
}

// ---------------------------------------------------------------------------
// バンク
// ---------------------------------------------------------------------------

/// バンク内の1エントリー。
struct Entry {
    /// OPM由来なら重複判定用にソースボイスを保持する（[timbre_eq] で比較）。
    opm_voice: Option<OpmVoice>,
    /// OPM以外（OPN・SSG固定パッチ）の重複判定キー。
    key: Option<DedupKey>,
    name: String,
    patch: Op505Patch,
    warnings: Vec<String>,
}

pub struct PatchBank {
    conv: Op505Converter,
    entries: Vec<Entry>,
}

impl PatchBank {
    pub fn new(conv: Op505Converter) -> Self {
        Self { conv, entries: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 指定インデックスの変換後パッチを返す（WAVレンダリング用）。
    pub fn patch_at(&self, idx: usize) -> Op505Patch {
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
            key: None,
            name: format!("patch{idx:03}"),
            patch,
            warnings,
        });
        idx
    }

    /// OpnVoice を登録し、重複していればそのインデックスを、新規なら追加してインデックスを返す。
    /// 重複判定キーは [`OpnDedupKey`]（[find_or_insert_fixed][Self::find_or_insert_fixed]と
    /// 同一プールを共有）。
    pub fn find_or_insert_opn(&mut self, voice: &OpnVoice, name: &str) -> usize {
        let key = DedupKey::Opn(OpnDedupKey::from_voice(voice));
        for (i, e) in self.entries.iter().enumerate() {
            if e.key == Some(key) {
                return i;
            }
        }
        let idx = self.entries.len();
        let (patch, warnings) = self.conv.from_opn(voice);
        self.entries.push(Entry { opm_voice: None, key: Some(key), name: name.to_string(), patch, warnings });
        idx
    }

    /// SSG人工パッチ（[crate::ssg]が返すOP505ネイティブ表現をそのまま登録する）を
    /// 登録/再利用してインデックスを返す。
    pub fn find_or_insert_fixed(&mut self, patch: Op505Patch, id: SsgPatchId, name: &str) -> usize {
        let key = DedupKey::Fixed(id);
        for (i, e) in self.entries.iter().enumerate() {
            if e.key == Some(key) {
                return i;
            }
        }
        let idx = self.entries.len();
        self.entries.push(Entry { opm_voice: None, key: Some(key), name: name.to_string(), patch, warnings: Vec::new() });
        idx
    }

    /// 全エントリーを (program番号, 名前, パッチ) の列で返す（バンク出力用）。
    pub fn entries(&self) -> impl Iterator<Item = (u8, &str, &Op505Patch)> {
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

fn op_eq(a: &OpmOpReg, b: &OpmOpReg) -> bool {
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
