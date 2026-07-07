// ---------------------------------------------------------------------------
// パラメーターマッピング関数群（すべて純粋関数）
//
// 数式はすべて初期案（暫定）。CLAUDE.mdのテスト方針に従い、
// 実装後に音を聴いて係数を調整する。
// ---------------------------------------------------------------------------

/// MUL値(0〜15)→周波数比。OPM/OPN/OPQ/OPZ共通のMultiple(4bit、0=0.5倍、1〜15=等倍)に準拠。
/// 8bit統一の例外（[operator.rs](operator.rs)のOperatorParams::mul参照）。
pub fn mul_to_ratio(mul: u8) -> f32 {
    const TABLE: [f32; 16] = [
        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    TABLE[(mul as usize).min(15)]
}

/// DT1値(0〜255、中心128)→セント。中心128で±0、両端で±DETUNE_RANGE_CENTS（暫定50セント）。
pub fn dt1_to_cents(dt1: u8) -> f32 {
    const DETUNE_RANGE_CENTS: f32 = 50.0;
    (dt1 as f32 - 128.0) / 128.0 * DETUNE_RANGE_CENTS
}

/// op_fine_tune値(0〜255、中心128)→セント。中心128で±0、両端で±OP_FINE_TUNE_RANGE_CENTS（±1オクターブ=1200セント）。
/// DT1(±50セント)の範囲を超える広いデチューンや、インハーモニックなOP周波数比を音色として静的に表現するための拡張。
/// DT1とはセントで加算される（[crate::operator]の`effective_frequency`参照）。
pub fn op_fine_tune_to_cents(v: u8) -> f32 {
    const OP_FINE_TUNE_RANGE_CENTS: f32 = 1200.0;
    (v as f32 - 128.0) / 128.0 * OP_FINE_TUNE_RANGE_CENTS
}

/// TL値(0〜255)→リニアゲイン。実機OPM TL(7bit、0.75dB/step)のreg=0(0dB)〜
/// reg=127(-95.25dB)をtl=255〜0に厳密アンカーし、dB単位で線形補間する。
pub fn tl_to_gain(tl: u8) -> f32 {
    let db = -95.25 * (255 - tl) as f32 / 255.0;
    10f32.powf(db / 20.0)
}

// AR/D1R/D2R/RRの時定数マッピングはsound-core側（eg.rs）へ移設済み。
// operator.rsの`use crate::mapping::*;`がそのまま動くよう、ここで再エクスポートする。
pub use sound_core::eg::{ar_to_delta, decay_to_delta, rr_to_delta};

/// KSR値(0〜255)→A4(note=69)からのオクターブ差に対するレート倍率。
/// `exponent = ksr/255`の線形カーブで、ksr=255(実機KS=3相当)は1オクターブごとに
/// 2倍(exponent=1.0)、ksr=0は音域に依らず常に1.0（オクターブ依存なし）。
///
/// 【2026-07-02 二段階の改訂】旧実装（指数カーブ`0.125×2^(3ksr/255)`＋A4未満クランプ）を
/// 2回に分けて改訂した。
///
/// 1回目: クランプ撤去。opz2x6がキャリアのKSR焼き込みを廃止した結果、低音でKSRが
/// 実質消失する回帰が発生（opzref実機忠実レンダリングとのEG減衰スロープ比較で発覚。
/// GrandPiano D2で実機-13.5dB/sに対し38x6経由-3.8dB/sと約3.5倍遅かった）。
///
/// 2回目: カーブ自体を指数から線形へ変更。旧指数カーブは実機の絶対keycode法則
/// （eg_rate=2R+keycode>>(3-KS)）のKS=0〜3の4アンカー点で理論上厳密一致していたが
/// （0.125,0.25,0.5,1.0の2倍刻み）、過去の実機比較（mucom2x6、MUCOM88実機C2/C6）で
/// 「KS=0は音域依存がほぼ無い方が近い」という聴感結果が別途存在し、指数カーブの
/// KS=0値(0.125)とは矛盾していた。線形カーブはksr=0で厳密に0（フラット）になり、
/// GrandPiano検証済みのKS=2アンカー（ksr=128→exponent≈0.5）とKS=3の理論値
/// （ksr=255→exponent=1.0）は保持したまま、KS=0のみ聴感どおりフラットにできる。
/// 実機KS(0-3)→ksrの対応表（0,64,128,255）は各コンバーター（opz2x6/opm2x6/mucom2x6/
/// psr2x6の`ks_to_ksr`）側が持ち、エンジンはKS概念に依存しない汎用線形カーブに徹する
/// （層分離: エンジンは忠実な物理法則、コンバーターが実機との対応・味付けを決める）。
pub fn ksr_rate_multiplier(ksr: u8, note: u8) -> f32 {
    let octave_diff = (note as f32 - 69.0) / 12.0;
    let exponent = ksr as f32 / 255.0;
    2f32.powf(octave_diff * exponent)
}

/// 実効TL = clamp(TLベース値 + (Velocity/127) × VelocitySensitivity, 0, 255)
/// Velocity Sensitivityは「音の明るさ」専用：モジュレーターのTL（=変調量）にのみ作用させる。
/// キャリアの音量はこの式ではなく`velocity_to_volume_gain`に一本化する（spec-sound.md参照）。
pub fn effective_tl(base_tl: u8, velocity: u8, sensitivity: u8) -> u8 {
    let add = (velocity as f32 / 127.0) * sensitivity as f32;
    (base_tl as f32 + add).round().clamp(0.0, 255.0) as u8
}

/// ベロシティ(0〜127)→チャンネル出力にかかる音量ゲイン（音色は一切変えない）。
/// 通常のMIDI楽器と同じく、ベロシティは音量だけに作用する常時ONの挙動。
/// velocity=127で1.0（フル）、低ベロシティほど小さくなる線形振幅カーブ。
/// （聴感カーブは実装後に調整する前提の第一案）
pub fn velocity_to_volume_gain(velocity: u8) -> f32 {
    velocity as f32 / 127.0
}

/// フィードバック値(0〜255)→自己変調の強さ（位相オフセット換算、単位：サイクル）。
/// OPM/OPNのFB(3bit、0=オフ・1〜7は1段ごとに2倍)を踏まえ、feedback=0は実機FB=0と
/// 同じ「フィードバックなし」の特殊値。feedback=1〜255は7オクターブ
/// （約36刻みごとに2倍）の指数カーブにマッピングする。
///
/// 最大値は1.8（feedback=255で1.8サイクル、FB=7相当のfeedback=252で約1.7サイクル）。
/// OPN実機の理論値は±0.5サイクルだが、38x6のフィードバックは直近1サンプル帰還で
/// 実機の2サンプル平均より自己変調の効きが弱く、0.5ではハイハット等のFBリッチな音色の
/// ノイズ成分を再現できなかった。fmgen原音(@113 HI_HAT/@1/@77)と聴き比べてM=1.8に調整。
pub fn feedback_to_scale(feedback: u8) -> f32 {
    feedback_to_scale_with_max(feedback, 1.8)
}

/// [feedback_to_scale]の最大値を可変にした版（フィードバック帰還方式の実験・測定用）。
/// 通常の発音は`feedback_to_scale`（max=1.8）を使う。
pub fn feedback_to_scale_with_max(feedback: u8, max: f32) -> f32 {
    if feedback == 0 {
        return 0.0;
    }
    max * 2.0f32.powf(7.0 * (feedback as f32 / 255.0 - 1.0))
}

/// オペレーター間FM変調の深さスケール（固定の内部定数、暫定値）。
/// 実機FM音源にチャンネル単位の「PM感度」相当のパラメーターは無く、
/// モジュレーターのTL（出力レベル）がそのまま変調量になる。
/// このスケールは出力波形の振幅(-1.0〜1.0)を位相オフセット量に変換するための係数。
pub const FM_MODULATION_INDEX_SCALE: f32 = 4.0;

/// 周波数(Hz)→近似MIDIノート番号（KSR計算用）。
pub fn frequency_to_note(frequency: f32) -> u8 {
    (69.0 + 12.0 * (frequency / 440.0).log2())
        .round()
        .clamp(0.0, 127.0) as u8
}

/// 13bit F-Number(0〜8191)の中心値（2^12）。OP単位F-Number上書き(NRPN 0,18〜21)で
/// 比率1.0（上書きなし、Note-On時と同じ周波数）を表す基準値（暫定）。
pub const F_NUMBER_CENTER: u16 = 4096;

/// F-Number(0〜8191)→周波数比。F_NUMBER_CENTERで1.0倍（上書きなし）、
/// 全域で約0.0〜2.0倍（約2オクターブ分）の可変範囲を持つ（暫定）。
pub fn f_number_to_ratio(f_number: u16) -> f32 {
    f_number as f32 / F_NUMBER_CENTER as f32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_to_ratio_bounds() {
        assert_eq!(mul_to_ratio(0), 0.5);
        assert_eq!(mul_to_ratio(15), 15.0);
        assert_eq!(mul_to_ratio(1), 1.0);
        // 範囲外(16〜255)は15にクランプ
        assert_eq!(mul_to_ratio(255), 15.0);
    }

    #[test]
    fn dt1_to_cents_center_and_bounds() {
        assert_eq!(dt1_to_cents(128), 0.0);
        assert_eq!(dt1_to_cents(0), -50.0);
        assert!((dt1_to_cents(255) - 49.609375).abs() < 1e-3);
    }

    #[test]
    fn op_fine_tune_to_cents_center_and_bounds() {
        assert_eq!(op_fine_tune_to_cents(128), 0.0);
        assert_eq!(op_fine_tune_to_cents(0), -1200.0);
        // (255-128)/128*1200 = 1190.625
        assert!((op_fine_tune_to_cents(255) - 1190.625).abs() < 1e-3);
    }

    #[test]
    fn tl_to_gain_bounds_and_monotonic() {
        // tl=255はreg=0相当（0dB、減衰なし）
        assert!((tl_to_gain(255) - 1.0).abs() < 1e-6);
        // tl=0はreg=127相当（-95.25dB）
        assert!((tl_to_gain(0) - 10f32.powf(-95.25 / 20.0)).abs() < 1e-9);
        assert!(tl_to_gain(0) > 0.0 && tl_to_gain(0) < tl_to_gain(255));
        assert!(tl_to_gain(128) < tl_to_gain(255));
    }

    #[test]
    fn ksr_rate_multiplier_bounds() {
        // note=69（A4）ならオクターブ差0で常に1.0
        assert!((ksr_rate_multiplier(0, 69) - 1.0).abs() < 1e-6);
        assert!((ksr_rate_multiplier(255, 69) - 1.0).abs() < 1e-6);
        // ksr=0はフラット（音域に依らず常に1.0、実機KS=0相当）
        assert!((ksr_rate_multiplier(0, 81) - 1.0).abs() < 1e-6);
        assert!((ksr_rate_multiplier(0, 57) - 1.0).abs() < 1e-6);
        // ksr=128(実機KS=2相当)、1オクターブ上はexponent=128/255≈0.5倍速
        // （GrandPianoキャリア実機比較で検証済みのアンカー）
        assert!((ksr_rate_multiplier(128, 81) - 2f32.powf(128.0 / 255.0)).abs() < 1e-6);
        // ksr=255、1オクターブ上（note=81）は実機KSR=3相当（2倍速）
        assert!((ksr_rate_multiplier(255, 81) - 2.0).abs() < 1e-6);
        // A4未満はクランプせず1.0未満（低音ほど減衰が遅くなる、実機keycode相当）
        assert!(ksr_rate_multiplier(128, 57) < 1.0);
        assert!(ksr_rate_multiplier(255, 57) < ksr_rate_multiplier(128, 57));
    }

    #[test]
    fn effective_tl_clamps() {
        assert_eq!(effective_tl(250, 127, 255), 255);
        assert_eq!(effective_tl(0, 0, 255), 0);
        assert_eq!(effective_tl(100, 0, 100), 100);
    }

    #[test]
    fn feedback_to_scale_bounds() {
        assert_eq!(feedback_to_scale(0), 0.0);
        assert!((feedback_to_scale(255) - 1.8).abs() < 1e-3);
        // 指数カーブ：feedback=0(オフ)以外は全域で滑らかに増加する
        assert!(feedback_to_scale(1) > 0.0);
        assert!(feedback_to_scale(64) < feedback_to_scale(128));
        assert!(feedback_to_scale(128) < feedback_to_scale(192));
        assert!(feedback_to_scale(192) < feedback_to_scale(255));
    }

    #[test]
    fn frequency_to_note_reference_points() {
        assert_eq!(frequency_to_note(440.0), 69);
        assert_eq!(frequency_to_note(880.0), 81);
        assert_eq!(frequency_to_note(220.0), 57);
    }

    #[test]
    fn f_number_to_ratio_bounds() {
        assert_eq!(f_number_to_ratio(F_NUMBER_CENTER), 1.0);
        assert_eq!(f_number_to_ratio(0), 0.0);
        assert!((f_number_to_ratio(8191) - 1.99976).abs() < 1e-4);
    }
}
