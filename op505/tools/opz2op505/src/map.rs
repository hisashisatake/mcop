//! TX81Z（OPZ/YM2414）レジスタ値 → 実機非依存スケールへの写像・非EGフィールド構築。
//!
//! 由来: ym38x6/tools/opz2x6/src/conv.rs（コミットe7c0fa7時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! opz2x6側の修正は自動では反映されない（`git diff e7c0fa7 -- ym38x6/tools/opz2x6/src/conv.rs`
//! で追従漏れを確認できる）。
//!
//! EGレート系の関数名（旧`_to_x6`）は`_to_eg_rate`/`_to_eg_level`へ改名した。実体は
//! `sound_core::eg::{ar_to_delta, decay_to_delta, rr_to_delta}`が解釈するレートスケール
//! （0〜255）であり、ym38x6固有ではない（旧名の"x6"は歴史的経緯にすぎない）。

use op505_core::Op505OperatorParams;
use sound_core::TimeEgParams;

use crate::parse::{OpzOpData, OpzVoice};

// ---------------------------------------------------------------------------
// キャリア判定テーブル（ym38x6-core/src/algorithm.rs から複製）
// index = algorithm (0-7), value = operators[] のキャリアインデックス一覧
// ---------------------------------------------------------------------------

pub const CARRIERS: [&[usize]; 8] = [
    &[3],          // 0: O1→O2→O3→O4
    &[3],          // 1: (O1+O2)→O3→O4
    &[3],          // 2: (O1+(O2→O3))→O4
    &[3],          // 3: ((O1→O2)+O3)→O4
    &[1, 3],       // 4: (O1→O2)+(O3→O4)
    &[1, 2, 3],    // 5: (O1→O2)+(O1→O3)+(O1→O4)
    &[1, 2, 3],    // 6: (O1→O2)+O3+O4
    &[0, 1, 2, 3], // 7: O1+O2+O3+O4（全並列）
];

/// TX81Z Aalg：アルゴリズムによる減衰量（実機のTLレジスタへの追加減衰、キャリアのみに適用）。
/// 出典: nornandブログ「TX81Zを解析した（Operator Output Level編）」の箇条書き。
const ALG_ATTEN_BY_CARRIER_COUNT: [u8; 5] = [0, 0, 8, 13, 16];

/// アルゴリズム(0-7)のキャリアに適用するAalg減衰量。
pub fn alg_atten(alg: u8) -> u8 {
    ALG_ATTEN_BY_CARRIER_COUNT[CARRIERS[alg.min(7) as usize].len().min(4)]
}

// ---------------------------------------------------------------------------
// スカラー変換
// ---------------------------------------------------------------------------

/// TX81Z Operator Output Level(OL, 0-99, 99=最大) → 実機のTLレジスタ加算値(Aol, 0-127, 0=最大)。
/// 出典: nornandブログ「TX81Zを解析した（Operator Output Level編）」実測値。
const OL_TO_AOL: [u8; 100] = [
    127, 122, 118, 114, 110, 107, 104, 102, 100, 98, // OL=0..9
    96, 94, 92, 90, 88, 86, 85, 84, 82, 81, // OL=10..19
    79, 78, 77, 76, 75, 74, 73, 72, 71, 70, // OL=20..29
    69, 68, 67, 66, 65, 64, 63, 62, 61, 60, // OL=30..39
    59, 58, 57, 56, 55, 54, 53, 52, 51, 50, // OL=40..49
    49, 48, 47, 46, 45, 44, 43, 42, 41, 40, // OL=50..59
    39, 38, 37, 36, 35, 34, 33, 32, 31, 30, // OL=60..69
    29, 28, 27, 26, 25, 24, 23, 22, 21, 20, // OL=70..79
    19, 18, 17, 16, 15, 14, 13, 12, 11, 10, // OL=80..89
    9, 8, 7, 6, 5, 4, 3, 2, 1, 0, // OL=90..99
];

pub fn ol_to_atten(ol: u8) -> u8 {
    OL_TO_AOL[ol.min(99) as usize]
}

/// Aol(0-127) → TL(0-255, 0=無音, 255=最大)。両者ともdB線形スケールなので向き反転+リスケール。
fn aol_to_tl(aol: u8) -> u8 {
    ((127 - aol.min(127)) as f32 / 127.0 * 255.0).round() as u8
}

/// OUT (TX81Z Output Level 0-99, 99=最大) → TL（キャリア用、0=無音, 255=最大）。
fn out_to_tl(out: u8, extra_atten: u8) -> u8 {
    aol_to_tl(ol_to_atten(out).saturating_add(extra_atten).min(127))
}

/// モジュレーター TL 天井のオプトイン値（既定は天井なし）。
pub const DEFAULT_MOD_TL_CAP: u8 = 180;

/// OUT → TL（モジュレーター用）。`cap`が`Some`なら上限を設ける。
fn out_to_tl_mod(out: u8, cap: Option<u8>) -> u8 {
    match cap {
        Some(cap) => {
            let aol = ol_to_atten(out);
            ((127 - aol.min(127)) as f32 / 127.0 * cap as f32).round() as u8
        }
        None => out_to_tl(out, 0),
    }
}

/// D1L/SL（TX81Zパネル値 4-bit 0-15）→ EGレベル(0-255)。dBリニア写像。
pub fn sl_to_eg_level(panel: u8) -> u8 {
    let reg = 15 - panel.min(15);
    let db: f32 = if reg >= 15 { -93.0 } else { -(3.0 * reg as f32) };
    (255.0 * (1.0 + db / 93.0)).round() as u8
}

/// TX81Z DET (0-6, 3=中心) → dt1（中心128）。
fn dt1_reg_to_detune(det: u8) -> u8 {
    const DT1_FROM_DET: [u8; 7] = [7, 6, 5, 0, 1, 2, 3];
    let dt1 = DT1_FROM_DET[det.min(6) as usize];
    const DT1_TO_SCALE: [u8; 8] = [128, 131, 134, 136, 128, 125, 122, 120];
    DT1_TO_SCALE[dt1 as usize]
}

/// TX81Z FREQ (coarse 0-63, fine 0-15) → 周波数比率(ratio) 実測テーブル。index = `16*coarse + fine`。
/// DXConvert実測テーブル（`fourop.py`の`freq_4op`）に基づく。詳細な出典・非単調誤記2セルの
/// 補正根拠はopz2x6::conv::FREQ_4OPのdocコメント参照（このファイルの由来コミット参照）。
#[rustfmt::skip]
const FREQ_4OP: [f32; 1024] = [
    0.50, 0.56, 0.62, 0.68, 0.75, 0.81, 0.87, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, 0.93, // coarse=0
    0.71, 0.79, 0.88, 0.96, 1.05, 1.14, 1.23, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, 1.32, // coarse=1
    0.78, 0.88, 0.98, 1.07, 1.17, 1.27, 1.37, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, 1.47, // coarse=2
    0.87, 0.97, 1.08, 1.18, 1.29, 1.40, 1.51, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, 1.62, // coarse=3
    1.00, 1.06, 1.12, 1.18, 1.25, 1.31, 1.37, 1.43, 1.50, 1.56, 1.62, 1.68, 1.75, 1.81, 1.87, 1.93, // coarse=4
    1.41, 1.49, 1.58, 1.67, 1.76, 1.85, 1.93, 2.02, 2.11, 2.20, 2.29, 2.37, 2.46, 2.55, 2.64, 2.73, // coarse=5
    1.57, 1.66, 1.76, 1.86, 1.96, 2.06, 2.15, 2.25, 2.35, 2.45, 2.55, 2.64, 2.74, 2.84, 2.94, 3.04, // coarse=6
    1.73, 1.83, 1.94, 2.05, 2.16, 2.27, 2.37, 2.48, 2.59, 2.70, 2.81, 2.91, 3.02, 3.13, 3.24, 3.35, // coarse=7
    2.00, 2.06, 2.12, 2.18, 2.25, 2.31, 2.37, 2.43, 2.50, 2.56, 2.62, 2.68, 2.75, 2.81, 2.87, 2.93, // coarse=8
    2.82, 2.90, 2.99, 3.08, 3.17, 3.26, 3.34, 3.43, 3.52, 3.61, 3.70, 3.78, 3.87, 3.96, 4.05, 4.14, // coarse=9
    3.00, 3.06, 3.12, 3.18, 3.25, 3.31, 3.37, 3.43, 3.50, 3.56, 3.62, 3.68, 3.75, 3.81, 3.87, 3.93, // coarse=10
    3.14, 3.23, 3.33, 3.43, 3.53, 3.63, 3.72, 3.82, 3.92, 4.02, 4.12, 4.21, 4.31, 4.41, 4.51, 4.61, // coarse=11
    3.46, 3.56, 3.67, 3.78, 3.89, 4.00, 4.10, 4.21, 4.32, 4.43, 4.54, 4.64, 4.75, 4.86, 4.97, 5.08, // coarse=12
    4.00, 4.06, 4.12, 4.18, 4.25, 4.31, 4.37, 4.43, 4.50, 4.56, 4.62, 4.68, 4.75, 4.81, 4.87, 4.93, // coarse=13
    4.24, 4.31, 4.40, 4.49, 4.58, 4.67, 4.75, 4.84, 4.93, 5.02, 5.11, 5.19, 5.28, 5.37, 5.46, 5.55, // coarse=14
    4.71, 4.80, 4.90, 5.00, 5.10, 5.20, 5.29, 5.39, 5.49, 5.59, 5.69, 5.78, 5.88, 5.98, 6.08, 6.18, // coarse=15
    5.00, 5.06, 5.12, 5.18, 5.25, 5.31, 5.37, 5.43, 5.50, 5.56, 5.62, 5.68, 5.75, 5.81, 5.87, 5.93, // coarse=16
    5.19, 5.29, 5.40, 5.51, 5.62, 5.73, 5.83, 5.94, 6.05, 6.16, 6.27, 6.37, 6.48, 6.59, 6.70, 6.81, // coarse=17
    5.65, 5.72, 5.81, 5.90, 5.99, 6.08, 6.16, 6.25, 6.34, 6.43, 6.52, 6.60, 6.69, 6.78, 6.87, 6.96, // coarse=18
    6.00, 6.06, 6.12, 6.18, 6.25, 6.31, 6.37, 6.43, 6.50, 6.56, 6.62, 6.68, 6.75, 6.81, 6.87, 6.93, // coarse=19
    6.28, 6.37, 6.47, 6.57, 6.67, 6.77, 6.86, 6.96, 7.06, 7.16, 7.26, 7.35, 7.45, 7.55, 7.65, 7.75, // coarse=20
    6.92, 7.02, 7.13, 7.24, 7.35, 7.46, 7.56, 7.67, 7.78, 7.89, 8.00, 8.10, 8.21, 8.32, 8.43, 8.54, // coarse=21
    7.00, 7.06, 7.12, 7.18, 7.25, 7.31, 7.37, 7.43, 7.50, 7.56, 7.62, 7.68, 7.75, 7.81, 7.87, 7.93, // coarse=22
    7.07, 7.13, 7.22, 7.31, 7.40, 7.49, 7.57, 7.66, 7.75, 7.84, 7.93, 8.01, 8.10, 8.19, 8.28, 8.37, // coarse=23
    7.85, 7.94, 8.04, 8.14, 8.24, 8.34, 8.43, 8.53, 8.63, 8.73, 8.83, 8.92, 9.02, 9.12, 9.22, 9.32, // coarse=24
    8.00, 8.06, 8.12, 8.18, 8.25, 8.31, 8.37, 8.43, 8.50, 8.56, 8.62, 8.68, 8.75, 8.81, 8.87, 8.93, // coarse=25
    8.48, 8.54, 8.63, 8.72, 8.81, 8.90, 8.98, 9.07, 9.16, 9.25, 9.34, 9.42, 9.51, 9.60, 9.69, 9.78, // coarse=26
    8.65, 8.75, 8.86, 8.97, 9.08, 9.19, 9.29, 9.40, 9.51, 9.62, 9.73, 9.83, 9.94, 10.05, 10.16, 10.27, // coarse=27
    9.00, 9.06, 9.12, 9.18, 9.25, 9.31, 9.37, 9.43, 9.50, 9.56, 9.62, 9.68, 9.75, 9.81, 9.87, 9.93, // coarse=28
    9.42, 9.51, 9.61, 9.71, 9.81, 9.91, 10.00, 10.10, 10.20, 10.30, 10.40, 10.49, 10.59, 10.69, 10.79, 10.89, // coarse=29
    9.89, 9.95, 10.04, 10.13, 10.22, 10.31, 10.39, 10.48, 10.57, 10.66, 10.75, 10.83, 10.92, 11.01, 11.10, 11.19, // coarse=30
    10.00, 10.06, 10.12, 10.18, 10.25, 10.31, 10.37, 10.43, 10.50, 10.56, 10.62, 10.68, 10.75, 10.81, 10.87, 10.93, // coarse=31
    10.38, 10.48, 10.59, 10.70, 10.81, 10.92, 11.02, 11.13, 11.24, 11.35, 11.46, 11.56, 11.67, 11.78, 11.89, 12.00, // coarse=32
    10.99, 11.08, 11.18, 11.28, 11.38, 11.48, 11.57, 11.67, 11.77, 11.87, 11.97, 12.06, 12.16, 12.26, 12.36, 12.46, // coarse=33
    11.00, 11.06, 11.12, 11.18, 11.25, 11.31, 11.37, 11.43, 11.50, 11.56, 11.62, 11.68, 11.75, 11.81, 11.87, 11.93, // coarse=34
    11.30, 11.36, 11.45, 11.54, 11.63, 11.72, 11.80, 11.89, 11.98, 12.07, 12.16, 12.24, 12.33, 12.42, 12.51, 12.60, // coarse=35
    12.00, 12.06, 12.12, 12.18, 12.25, 12.31, 12.37, 12.43, 12.50, 12.56, 12.62, 12.68, 12.75, 12.81, 12.87, 12.93, // coarse=36
    12.11, 12.21, 12.32, 12.43, 12.54, 12.65, 12.75, 12.86, 12.97, 13.08, 13.19, 13.29, 13.40, 13.51, 13.62, 13.73, // coarse=37
    12.56, 12.65, 12.75, 12.85, 12.95, 13.05, 13.14, 13.24, 13.34, 13.44, 13.54, 13.63, 13.73, 13.83, 13.93, 14.03, // coarse=38 (fine=12は原典13.37を非単調誤記と判断し前後平均13.73へ補正)
    12.72, 12.77, 12.86, 12.95, 13.04, 13.13, 13.21, 13.30, 13.39, 13.48, 13.57, 13.65, 13.74, 13.83, 13.92, 14.01, // coarse=39
    13.00, 13.06, 13.12, 13.18, 13.25, 13.31, 13.37, 13.43, 13.50, 13.56, 13.62, 13.68, 13.75, 13.81, 13.87, 13.93, // coarse=40
    13.84, 13.94, 14.05, 14.16, 14.27, 14.38, 14.48, 14.59, 14.70, 14.81, 14.92, 15.02, 15.13, 15.24, 15.35, 15.46, // coarse=41
    14.00, 14.06, 14.12, 14.18, 14.25, 14.31, 14.37, 14.43, 14.50, 14.56, 14.62, 14.68, 14.75, 14.81, 14.87, 14.93, // coarse=42
    14.10, 14.18, 14.27, 14.36, 14.45, 14.54, 14.62, 14.71, 14.80, 14.89, 14.98, 15.06, 15.15, 15.24, 15.33, 15.42, // coarse=43
    14.13, 14.22, 14.32, 14.42, 14.52, 14.62, 14.71, 14.81, 14.91, 15.01, 15.11, 15.20, 15.30, 15.40, 15.50, 15.60, // coarse=44
    15.00, 15.06, 15.12, 15.18, 15.25, 15.31, 15.37, 15.43, 15.50, 15.56, 15.62, 15.68, 15.75, 15.81, 15.87, 15.93, // coarse=45
    15.55, 15.59, 15.68, 15.77, 15.86, 15.95, 16.03, 16.12, 16.21, 16.30, 16.39, 16.47, 16.56, 16.65, 16.74, 16.83, // coarse=46
    15.57, 15.67, 15.78, 15.89, 16.00, 16.11, 16.21, 16.32, 16.43, 16.54, 16.65, 16.75, 16.86, 16.97, 17.08, 17.19, // coarse=47
    15.70, 15.79, 15.89, 15.99, 16.09, 16.19, 16.28, 16.38, 16.48, 16.58, 16.68, 16.77, 16.87, 16.97, 17.07, 17.17, // coarse=48
    16.96, 17.00, 17.09, 17.18, 17.27, 17.36, 17.44, 17.53, 17.62, 17.71, 17.80, 17.88, 17.97, 18.06, 18.15, 18.24, // coarse=49
    17.27, 17.36, 17.46, 17.56, 17.66, 17.76, 17.85, 17.95, 18.05, 18.15, 18.25, 18.35, 18.44, 18.54, 18.64, 18.74, // coarse=50
    17.30, 17.40, 17.51, 17.62, 17.73, 17.84, 17.94, 18.05, 18.16, 18.27, 18.38, 18.48, 18.59, 18.70, 18.81, 18.92, // coarse=51
    18.37, 18.41, 18.50, 18.59, 18.68, 18.77, 18.85, 18.94, 19.03, 19.12, 19.21, 19.29, 19.38, 19.47, 19.56, 19.65, // coarse=52
    18.84, 18.93, 19.03, 19.13, 19.23, 19.33, 19.42, 19.52, 19.62, 19.72, 19.82, 19.91, 20.01, 20.11, 20.21, 20.31, // coarse=53
    19.03, 19.13, 19.24, 19.35, 19.46, 19.57, 19.67, 19.78, 19.89, 20.00, 20.11, 20.21, 20.32, 20.43, 20.54, 20.65, // coarse=54
    19.78, 19.82, 19.91, 20.00, 20.09, 20.18, 20.26, 20.35, 20.44, 20.53, 20.62, 20.70, 20.79, 20.88, 20.97, 21.06, // coarse=55
    20.41, 20.50, 20.60, 20.70, 20.80, 20.90, 20.99, 21.09, 21.19, 21.29, 21.39, 21.48, 21.58, 21.68, 21.78, 21.88, // coarse=56
    20.76, 20.86, 20.97, 21.08, 21.19, 21.30, 21.40, 21.51, 21.62, 21.73, 21.84, 21.94, 22.05, 22.16, 22.27, 22.38, // coarse=57 (fine=10は原典21.48を非単調誤記と判断し前後平均21.84へ補正)
    21.20, 21.23, 21.32, 21.41, 21.50, 21.59, 21.67, 21.76, 21.85, 21.94, 22.03, 22.11, 22.20, 22.29, 22.38, 22.47, // coarse=58
    21.98, 22.07, 22.17, 22.27, 22.37, 22.47, 22.56, 22.66, 22.76, 22.86, 22.96, 23.05, 23.15, 23.25, 23.35, 23.45, // coarse=59
    22.49, 22.59, 22.70, 22.81, 22.92, 23.03, 23.13, 23.24, 23.35, 23.46, 23.57, 23.67, 23.78, 23.89, 24.00, 24.11, // coarse=60
    23.55, 23.64, 23.74, 23.84, 23.94, 24.04, 24.13, 24.23, 24.33, 24.43, 24.53, 24.62, 24.72, 24.82, 24.92, 25.02, // coarse=61
    24.22, 24.32, 24.43, 24.54, 24.65, 24.76, 24.86, 24.97, 25.08, 25.19, 25.30, 25.40, 25.51, 25.62, 25.73, 25.84, // coarse=62
    25.95, 26.05, 26.16, 26.27, 26.38, 26.49, 26.59, 26.70, 26.81, 26.92, 27.03, 27.13, 27.24, 27.35, 27.46, 27.57, // coarse=63
];

/// TX81Z FREQ coarse(0-63) + fine(0-15) → 周波数比率(ratio)。
pub fn coarse_fine_to_ratio(coarse: u8, fine: u8) -> f32 {
    FREQ_4OP[16 * coarse.min(63) as usize + fine.min(15) as usize]
}

/// 周波数比率(ratio) → (MUL 0-15, op_fine_tune 0-255)。最近傍の整数MULを選び、
/// 差分セントをop_fine_tuneに写像する。
pub fn ratio_to_mul_fine(ratio: f32) -> (u8, u8) {
    let mut best_mul = 0u8;
    let mut best_dist = f32::MAX;
    for m in 0u8..=15 {
        let mul_ratio = if m == 0 { 0.5_f32 } else { m as f32 };
        let dist = (ratio / mul_ratio).log2().abs();
        if dist < best_dist {
            best_dist = dist;
            best_mul = m;
        }
    }
    let mul_ratio = if best_mul == 0 { 0.5_f32 } else { best_mul as f32 };
    let cents = (ratio / mul_ratio).log2() * 1200.0;
    let oft = (128.0 + (cents * 127.0 / 1200.0)).round().clamp(0.0, 255.0) as u8;
    (best_mul, oft)
}

/// 半音値からオクターブ(12の倍数)を除いた非オクターブ成分（[-6, 6]に収まる）。
fn nonoctave_semitones(t: f32) -> f32 {
    t - 12.0 * (t / 12.0).round()
}

/// 音色の「音程正規化」係数を求める（TRPS方式）。詳細な設計根拠は
/// opz2x6::conv::voice_pitch_foldのdocコメント参照（このファイルの由来コミット参照）。
fn voice_pitch_fold(voice: &OpzVoice) -> f32 {
    2.0_f32.powf(nonoctave_semitones(voice.transpose as f32) / 12.0)
}

/// A4 相当のキーコード（KSR rate scaling の焼き込み量基準）。
const KEY_CODE_A4: u16 = 19;

/// rs(0-3) に応じた KSR(rate key scaling)の加算量。
fn ksr_add(rs: u8) -> u16 {
    let ksr_shift = 3u16.saturating_sub(rs.min(3) as u16);
    KEY_CODE_A4 >> ksr_shift
}

/// TX81Z RS(0-3、rate scaling)→ ksr(0-255、実行時の音域依存レート倍率)。
pub fn ks_to_ksr(rs: u8) -> u8 {
    const TABLE: [u8; 4] = [0, 64, 128, 255];
    TABLE[rs.min(3) as usize]
}

/// OPM型5-bitレート（D1R/D2R, 0-31）→ EGレート(0-255)。A4キーコードのKSR焼き込みを適用。
pub fn rate_to_eg_rate(rate: u8, rs: u8) -> u8 {
    if rate == 0 {
        return 0;
    }
    let eg_rate = (2 * rate as u16 + ksr_add(rs)).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// EGのアタック立ち上がりの聴感補正バイアス（`--attack bias`の既定経路）。
pub const ATTACK_ONSET_BIAS: u16 = 30;

/// OPM型5-bit AR（0-31）→ EGレート(0-255)。
pub fn ar_to_eg_rate(ar: u8, rs: u8) -> u8 {
    if ar == 0 {
        return 0;
    }
    (rate_to_eg_rate(ar, rs) as u16 + ATTACK_ONSET_BIAS).min(255) as u8
}

/// OPM型4-bitリリースレート（RR, 0-15）→ EGレート(0-255)。
pub fn rr_to_eg_rate(rr: u8, rs: u8) -> u8 {
    let eg_rate = (4 * rr as u16 + 2 + ksr_add(rs)).min(62);
    (1 + eg_rate.saturating_sub(2) * 254 / 60).min(255) as u8
}

/// TX81Z AMS (0-3) → ams(0-255)。
fn ams_reg_to_depth(reg: u8) -> u8 {
    if reg == 0 {
        return 0;
    }
    (1u16 + 127 * (reg.min(3) as u16 - 1)) as u8
}

/// TX81Z PMS (0-7) → pms(0-255)。
fn pms_reg_to_depth(reg: u8) -> u8 {
    if reg == 0 {
        return 0;
    }
    (1.0_f32 + 254.0 * (reg.min(7) - 1) as f32 / 6.0).round() as u8
}

/// AMD/PMD (0-99) → depth（0-255 線形スケール）。
fn lfo_depth_reg_to_depth(reg: u8) -> u8 {
    (reg as f32 * 255.0 / 99.0).round() as u8
}

/// TX81Z FB (0-7) → feedback（0-255、FB×36）。
fn fb_reg_to_feedback(fb: u8) -> u8 {
    fb.min(7) * 36
}

// ---------------------------------------------------------------------------
// EG形状の前段（実機レート→EGレートスケール）で共有する補正
// ---------------------------------------------------------------------------

/// EGT=1（sustain-less decay）のとき D2R を強制的に高値にしてリリース挙動を作る
/// (TX81Z は EGT=1 で D1L で止まらず一定レートで減衰する)。
pub fn effective_d2r(d2r: u8, egt: u8) -> u8 {
    if egt != 0 && d2r == 0 {
        20
    } else {
        d2r
    }
}

/// 味付け: キャリアのサステイン延長（実機忠実から意図的に離す）。
pub fn apply_carrier_sustain(d1l: &mut u8, d1r: &mut u8, d2r: &mut u8, carrier_sustain: f32) {
    let k = carrier_sustain.clamp(0.0, 1.0);
    let d1l_f = *d1l as f32;
    *d1l = (d1l_f + (255.0 - d1l_f) * 0.7 * k).round().clamp(0.0, 255.0) as u8;
    *d1r = (*d1r as f32 * (1.0 - 0.60 * k)).round().clamp(0.0, 255.0) as u8;
    *d2r = (*d2r as f32 * (1.0 - 0.85 * k)).round().clamp(0.0, 255.0) as u8;
}

// ---------------------------------------------------------------------------
// 変換オプション（音質追い込み用の上書き群、opz2x6::conv::ConvOptionsと同一）
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct ConvOptions {
    pub mod_tl_cap: Option<u8>,
    pub fb_override: Option<u8>,
    pub ksr_override: Option<u8>,
    pub carrier_sustain: f32,
    pub filter_cutoff: Option<u8>,
    pub pitch_normalize: bool,
}

impl Default for ConvOptions {
    fn default() -> Self {
        Self {
            mod_tl_cap: None,
            fb_override: None,
            ksr_override: None,
            carrier_sustain: 0.0,
            filter_cutoff: None,
            pitch_normalize: true,
        }
    }
}

// ---------------------------------------------------------------------------
// オペレーター・チャンネルの非EGフィールド構築
// ---------------------------------------------------------------------------

/// TX81Zオペレーター1個分の非EGフィールドを`Op505OperatorParams`へ直接構築する
/// （`eg`フィールドはこの関数では埋めない。呼び出し側が`direct_eg`の結果で上書きする）。
/// 数式はopz2x6::conv::convert_opと同一（このファイルの由来コミット参照）。
pub fn convert_op(op: &OpzOpData, is_carrier: bool, alg_atten: u8, opts: ConvOptions, pitch_fold: f32) -> Op505OperatorParams {
    let mod_tl_cap = opts.mod_tl_cap;
    let (mul, op_fine_tune) = ratio_to_mul_fine(coarse_fine_to_ratio(op.freq, op.fine) * pitch_fold);

    let vel_sens = if is_carrier { 0 } else { op.kvs.min(7) * 24 };

    let velocity_gain = if is_carrier {
        (op.kvs.min(7) as f32 * 255.0 / 7.0).round() as u8
    } else {
        255
    };

    Op505OperatorParams {
        tl: if is_carrier {
            out_to_tl(op.out, alg_atten)
        } else {
            out_to_tl_mod(op.out, mod_tl_cap).saturating_sub(vel_sens)
        },
        eg: TimeEgParams::default(),
        mul,
        dt1: dt1_reg_to_detune(op.det),
        ksr: opts.ksr_override.unwrap_or(ks_to_ksr(op.rs)),
        am_enable: op.ame,
        velocity_sensitivity: vel_sens,
        waveform: op.ow.min(7),
        op_fine_tune,
        eg_shift: op.egsft.min(3) * 85,
        level_scale: (op.ls as u16 * 165 / 64).min(255) as u8,
        velocity_gain,
    }
}

/// チャンネル単位の非EGフィールド一式（algorithm/feedback/chip_lfo_*/pms/ams/filter_cutoff）。
pub struct ChannelFields {
    pub algorithm: u8,
    pub feedback: u8,
    pub chip_lfo_freq: u8,
    pub chip_lfo_pmd: u8,
    pub chip_lfo_amd: u8,
    pub chip_lfo_delay: u8,
    pub pms: u8,
    pub ams: u8,
    pub filter_cutoff: u8,
}

pub fn convert_channel(voice: &OpzVoice, alg: u8, opts: ConvOptions) -> ChannelFields {
    ChannelFields {
        algorithm: alg,
        feedback: opts.fb_override.unwrap_or_else(|| fb_reg_to_feedback(voice.feedback)),
        chip_lfo_freq: (voice.lfo_spd as f32 * 255.0 / 99.0).round() as u8,
        chip_lfo_pmd: lfo_depth_reg_to_depth(voice.pmd),
        chip_lfo_amd: lfo_depth_reg_to_depth(voice.amd),
        chip_lfo_delay: (voice.lfo_dly as f32 * 255.0 / 99.0).round() as u8,
        pms: pms_reg_to_depth(voice.pms),
        ams: ams_reg_to_depth(voice.ams),
        filter_cutoff: opts.filter_cutoff.unwrap_or(255),
    }
}

/// 音程正規化(at pitch化)係数。`ConvOptions::pitch_normalize`がfalseなら1.0（額面比率のまま）。
pub fn pitch_fold_for(voice: &OpzVoice, opts: ConvOptions) -> f32 {
    if opts.pitch_normalize {
        voice_pitch_fold(voice)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_to_tl_polarity() {
        assert_eq!(out_to_tl(0, 0), 0);
        assert_eq!(out_to_tl(99, 0), 255);
        assert!(out_to_tl(50, 0) > 0 && out_to_tl(50, 0) < 255);
    }

    #[test]
    fn ol_to_atten_matches_reference_table() {
        assert_eq!(ol_to_atten(99), 0);
        assert_eq!(ol_to_atten(79), 20);
        assert_eq!(ol_to_atten(20), 79);
        assert_eq!(ol_to_atten(19), 81);
        assert_eq!(ol_to_atten(0), 127);
    }

    #[test]
    fn conv_options_default_has_no_modulator_cap() {
        assert_eq!(ConvOptions::default().mod_tl_cap, None);
        let op = OpzOpData { out: 99, freq: 4, det: 3, ar: 31, rr: 7, ..Default::default() };
        let p = convert_op(&op, false, 0, ConvOptions::default(), 1.0);
        assert_eq!(p.tl, out_to_tl(99, 0));
    }

    #[test]
    fn grandpiano_op2_maps_ls_to_level_scale() {
        let op = OpzOpData { out: 77, freq: 13, det: 3, ar: 20, rr: 7, ls: 94, kvs: 4, ..Default::default() };
        let p = convert_op(&op, false, 0, ConvOptions::default(), 1.0);
        assert_eq!(p.level_scale, (94u16 * 165 / 64) as u8);
    }
}
