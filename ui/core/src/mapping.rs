//! OPヘッダの実効周波数比（MUL×FINE）表示専用の極小マッピング。
//!
//! `sound-fm::mapping`（音源エンジン本体）と同じ表・同じ式を意図的にインライン複製している
//! （UI表示専用の値であり、`ui-core`はnice-plug/Tauri非依存のためエンジンcoreクレートに
//! 依存できない設計方針、`ym38x6-ui`/`op505-ui`双方が共有する）。内部エンジンは無改変。

/// MUL値(0〜15)→周波数比。`0`=0.5倍（サブ）、`1`=基音、`2〜15`=等倍
/// （OPM/OPN/OPQ/OPZ共通のMultiple 4bitに準拠）。
pub fn mul_to_ratio(mul: u8) -> f32 {
    const TABLE: [f32; 16] = [
        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    TABLE[(mul as usize).min(15)]
}

/// op_fine_tune値(0〜255、中心128)→セント。中心128で±0、両端±1200セント。
pub fn op_fine_tune_to_cents(v: u8) -> f32 {
    const OP_FINE_TUNE_RANGE_CENTS: f32 = 1200.0;
    (v as f32 - 128.0) / 128.0 * OP_FINE_TUNE_RANGE_CENTS
}

/// MUL＋FINE（op_fine_tune）による実効周波数比（DT1は除く）。OPヘッダの読み取り表示専用
/// （`<readout compute="mul-fine-ratio">`の実行時計算、`ui-codegen`が裸の名前で呼ぶ）。
pub fn mul_fine_ratio(mul: u8, op_fine_tune: u8) -> f32 {
    mul_to_ratio(mul) * 2f32.powf(op_fine_tune_to_cents(op_fine_tune) / 1200.0)
}
