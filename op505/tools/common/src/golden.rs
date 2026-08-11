//! ゴールデンテスト用の共通ヘルパー（FNV-1aフィンガープリント + ファイル照合）。
//!
//! 二層構成: [`Fingerprint`]は大量ケースの掃引を1個のハッシュへ凍結する「網羅性」担当、
//! [`assert_golden`]で照合する可読JSON/opテキストは「デバッグ可能性」担当（フィールド差分が
//! diffで見える）。詳細はspec-fm.md「デフォーク」節。
//!
//! [`Fingerprint::push`]は手書きバイトパッキングではなく`serde_json`のフィールド順シリアライズを
//! そのままハッシュ入力にする。struct にフィールドを追加したときゴールデン側の更新を忘れても
//! 自動的にハッシュへ反映されるため（`.claude/rules/shared-struct-field-additions.md`が警告する
//! 「追加箇所の反映漏れ」をこの層では構造的に避けられる）。

use std::env;
use std::fs;
use std::path::Path;

use serde::Serialize;

/// FNV-1a 64bit。
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// 大量ケースの掃引結果を1個のハッシュへ凍結する逐次フィンガープリント。
#[derive(Default)]
pub struct Fingerprint {
    buf: Vec<u8>,
    count: usize,
}

impl Fingerprint {
    pub fn new() -> Self {
        Self::default()
    }

    /// 1ケース分の値をハッシュ入力へ連結する。
    pub fn push<T: Serialize>(&mut self, value: &T) {
        let bytes = serde_json::to_vec(value).expect("golden fingerprint: serialize failed");
        self.buf.extend_from_slice(&bytes);
        self.buf.push(0); // レコード境界（値の跨ぎ誤結合を防ぐ）
        self.count += 1;
    }

    /// `version / case_count / hex digest` の3行テキストにする。
    /// `version`はケース生成ロジック（掃引範囲・ケース数）を変えたら上げる（ハッシュ不一致の
    /// 原因が「実装が壊れた」か「掃引を広げた」か区別するため）。
    pub fn finish(&self, version: u32) -> String {
        format!("{version}\n{}\n{:016x}\n", self.count, fnv1a64(&self.buf))
    }
}

/// ゴールデンファイルと照合する。`UPDATE_GOLDEN=1`なら書き込んで更新する。
/// 改行は`\n`へ正規化してから比較する（Editツール等によるCRLF/LF混在の影響を避けるため）。
pub fn assert_golden(path: &Path, actual: &str) {
    let actual = normalize_newlines(actual);
    if env::var_os("UPDATE_GOLDEN").is_some() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("golden: create_dir_all failed");
        }
        fs::write(path, &actual).unwrap_or_else(|e| panic!("golden write failed {path:?}: {e}"));
        return;
    }
    let expected = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "golden file missing: {path:?} ({e})\n\
             初回はこのコマンドで採取してください:\n\
             $env:UPDATE_GOLDEN=1; cargo test; Remove-Item Env:\\UPDATE_GOLDEN"
        )
    });
    let expected = normalize_newlines(&expected);
    assert_eq!(
        actual, expected,
        "golden mismatch: {path:?}\n\
         意図的な変更なら次のコマンドで更新してください:\n\
         $env:UPDATE_GOLDEN=1; cargo test; Remove-Item Env:\\UPDATE_GOLDEN"
    );
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_known_vector() {
        // FNV-1a 64bit の既知テストベクタ（空文字列）。
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
    }

    #[test]
    fn fingerprint_is_order_sensitive() {
        let mut a = Fingerprint::new();
        a.push(&1u8);
        a.push(&2u8);
        let mut b = Fingerprint::new();
        b.push(&2u8);
        b.push(&1u8);
        assert_ne!(a.finish(1), b.finish(1));
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let mut a = Fingerprint::new();
        a.push(&"hello");
        a.push(&42u32);
        let mut b = Fingerprint::new();
        b.push(&"hello");
        b.push(&42u32);
        assert_eq!(a.finish(1), b.finish(1));
    }
}
