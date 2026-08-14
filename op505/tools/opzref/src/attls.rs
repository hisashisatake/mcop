//! TX81Z Level Scaling(LS) → 実機TLレジスタ加算値の計算。
//!
//! 由来: ym38x6/tools/opz2x6/src/conv.rs の `attls_reg`（コミット b61ba7a 時点の複製、2026-08-13）。
//! op505版の`opz2op505::map`には対応する関数が無い（opz2op505は`level_scale`をop505パッチへ
//! 静的に埋め込むだけで、レジスタ加算値自体は計算しない設計のため）。レジスタ直書きの
//! opzrefにはこの計算そのものが必要なので複製する（fork-on-write）。

/// TX81Z Level Scaling: LS(0-99) × MIDIノート番号 → 実機TLレジスタ加算値(0-127, 0.75dB/step)。
///
/// 出典: nornandブログ「導出方法を考えてみた」のattLS実測疑似コード。ノート24が基準点で、
/// 3半音（1オクターブ/4）ごとにカーブテーブルのインデックスが1増える（高音ほど急峻に増加）。
pub fn attls_reg(ls: u8, note: u8) -> u8 {
    const CURVE: [u32; 29] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 14, 16, 19, 23, 28, 33, 39, 47, 57, 67, 80, 95, 113,
        134, 160, 190, 224, 255,
    ];
    let note = note as i32;
    let index = if note < 24 {
        0
    } else if note <= 108 {
        (note - 24) / 3
    } else if note < 121 {
        (note - 12 - 24) / 3
    } else {
        (note - 24 - 24) / 3
    };
    let index = index.clamp(0, CURVE.len() as i32 - 1) as usize;
    let depth = ls.min(99) as u32 * 165 / 64; // attLS内部スケール(0-255)。engineのlevel_scaleと同一の前処理
    ((depth * CURVE[index]) >> 8).min(127) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attls_reg_zero_ls_is_no_op() {
        assert_eq!(attls_reg(0, 60), 0);
        assert_eq!(attls_reg(0, 108), 0);
    }

    #[test]
    fn attls_reg_increases_with_pitch() {
        // GrandPiano OP2相当(LS=94)。低音ほど減衰小、高音ほど大（音域勾配）。
        let low = attls_reg(94, 44); // G#2
        let mid = attls_reg(94, 56); // G#3
        let high = attls_reg(94, 60); // C4
        assert!(low < mid && mid < high, "{low} < {mid} < {high}");
        // 検算値: ym38x6/tools/opz2x6の同名テストと同じ数値（複製元との一致確認）
        assert_eq!(mid, 10); // G#3: 7.5dB相当
        assert_eq!(high, 15); // C4: 11.2dB相当
    }
}
