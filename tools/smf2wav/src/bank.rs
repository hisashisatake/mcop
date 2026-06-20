//! プログラム番号 → パッチの解決テーブル。
//!
//! SMF のプログラムチェンジ（0〜127）から音色を引くため、128 スロットを事前に
//! 埋めた配列を持つ。未定義スロットは「直下（番号が小さい側）の最も近い定義済み
//! 番号」へフォールバックし、それより小さい番号が無ければ最小の定義済み番号を使う。

use ym38x6_core::{PresetFile, Ym38x6Patch};

/// 128 プログラム分のパッチテーブル。
pub struct PatchBank {
    table: Box<[Ym38x6Patch; 128]>,
}

impl PatchBank {
    /// `(program, patch)` の列から構築する。`program` は 0〜127（範囲外は破棄）。
    /// 同一 program が複数あれば後勝ち。空の場合はエラー。
    pub fn from_entries(entries: Vec<(u8, Ym38x6Patch)>) -> Result<Self, String> {
        let mut slots: [Option<Ym38x6Patch>; 128] = std::array::from_fn(|_| None);
        for (p, patch) in entries {
            if p < 128 {
                slots[p as usize] = Some(patch); // 後勝ち
            }
        }
        let Some(first) = slots.iter().position(|s| s.is_some()) else {
            return Err("パッチが1件もありません".to_string());
        };

        // 前方フィル：各スロットに「直下の最も近い定義済み」を割り当てる。
        // 先頭側の空きは最小の定義済み（first）へフォールバックする。
        let mut last: Option<Ym38x6Patch> = None;
        let table: Box<[Ym38x6Patch; 128]> = Box::new(std::array::from_fn(|i| match &slots[i] {
            Some(p) => {
                last = Some(p.clone());
                p.clone()
            }
            None => last
                .clone()
                .unwrap_or_else(|| slots[first].clone().expect("first は定義済み")),
        }));
        Ok(Self { table })
    }

    /// `.38x6`（`PresetFile`）から構築する。`Presets`/`Programs` どちらの形式でも
    /// 各エントリーの `program` 番号でパッチを配置する。
    pub fn from_preset_file(file: &PresetFile) -> Result<Self, String> {
        let entries = match file {
            PresetFile::Presets { presets, .. } => presets,
            PresetFile::Programs { programs, .. } => programs,
        };
        let list: Vec<(u8, Ym38x6Patch)> =
            entries.iter().map(|e| (e.program, e.patch.clone())).collect();
        Self::from_entries(list)
    }

    /// 連番のパッチ列から構築する（変換ツールの「ボイス番号＝プログラム番号」用）。
    /// `patches[i]` をプログラム番号 `i` に割り当てる（128 件目以降は破棄）。
    pub fn from_patches(patches: &[Ym38x6Patch]) -> Result<Self, String> {
        let list: Vec<(u8, Ym38x6Patch)> = patches
            .iter()
            .take(128)
            .enumerate()
            .map(|(i, p)| (i as u8, p.clone()))
            .collect();
        Self::from_entries(list)
    }

    /// プログラム番号（下位 7bit）に対応するパッチを返す。
    pub fn patch(&self, program: u8) -> &Ym38x6Patch {
        &self.table[(program & 0x7F) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `channel.feedback` をマーカーにしたパッチを作る。
    fn marked(fb: u8) -> Ym38x6Patch {
        let mut p = Ym38x6Patch::default();
        p.channel.feedback = fb;
        p
    }

    #[test]
    fn empty_is_error() {
        assert!(PatchBank::from_entries(vec![]).is_err());
    }

    #[test]
    fn exact_and_lower_fallback() {
        // program 2 と 5 のみ定義。
        let bank = PatchBank::from_entries(vec![(2, marked(20)), (5, marked(50))]).unwrap();
        // 2未満は最小（=2）へフォールバック。
        assert_eq!(bank.patch(0).channel.feedback, 20);
        assert_eq!(bank.patch(1).channel.feedback, 20);
        // 完全一致。
        assert_eq!(bank.patch(2).channel.feedback, 20);
        // 2〜4は直下の2へ。
        assert_eq!(bank.patch(3).channel.feedback, 20);
        assert_eq!(bank.patch(4).channel.feedback, 20);
        assert_eq!(bank.patch(5).channel.feedback, 50);
        // 5超は直下の5へ。
        assert_eq!(bank.patch(6).channel.feedback, 50);
        assert_eq!(bank.patch(127).channel.feedback, 50);
    }

    #[test]
    fn duplicate_program_last_wins() {
        let bank = PatchBank::from_entries(vec![(3, marked(30)), (3, marked(33))]).unwrap();
        assert_eq!(bank.patch(3).channel.feedback, 33);
    }

    #[test]
    fn from_patches_indexes_by_position() {
        let bank = PatchBank::from_patches(&[marked(0), marked(11), marked(22)]).unwrap();
        assert_eq!(bank.patch(0).channel.feedback, 0);
        assert_eq!(bank.patch(1).channel.feedback, 11);
        assert_eq!(bank.patch(2).channel.feedback, 22);
        // 範囲外は直下（=2）へ。
        assert_eq!(bank.patch(9).channel.feedback, 22);
    }
}
