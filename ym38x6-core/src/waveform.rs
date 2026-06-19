// ---------------------------------------------------------------------------
// OPZ準拠8波形（ymfm OPZ実装準拠、spec.md 波形8種類セクション参照）
//
// ymfmの実装パターン（全て対数減衰フォーマット）:
//   wf0: abs_sin | sign(bit9)                        → sin（フル）
//   wf1: min(wf0_log×2, silence) | sign(bit9)        → sin²（フル、符号付き）
//   wf2: bit9==1 ? silence : wf0                     → sin（前半のみ）
//   wf3: bit9==1 ? silence : wf1                     → sin²（前半のみ、符号付き）
//   wf4: bit9==1 ? silence : wf0[i×2]               → sin 2倍速（前半のみ）
//   wf5: bit9==1 ? silence : wf1[i×2]               → sin² 2倍速（前半のみ、符号付き）
//   wf6: bit9==1 ? silence : wf0[(i×2)&0x1ff]       → |sin| 2倍速（前半のみ）
//   wf7: bit9==1 ? silence : wf1[(i×2)&0x1ff]       → sin² 2倍速（前半のみ、常に正）
//
// 線形振幅での等価実装（gen_from_fn は線形 → 対数変換を行う）:
//   wf0: sin(2πp)
//   wf1: sin(2πp)·|sin(2πp)|
//   wf2: sin(2πp) for p∈[0,0.5), 0 for p∈[0.5,1)
//   wf3: sin(2πp)·|sin(2πp)| for p∈[0,0.5), 0
//   wf4: sin(4πp) for p∈[0,0.5), 0                  ← 2倍速の1周期まるごと+後半無音
//   wf5: sin(4πp)·|sin(4πp)| for p∈[0,0.5), 0
//   wf6: |sin(4πp)| for p∈[0,0.5), 0               ← ラップ効果で絶対値化
//   wf7: sin²(4πp) for p∈[0,0.5), 0                ← 常に正
// ---------------------------------------------------------------------------

use sound_core::{WaveTable, gen_from_fn};
use std::f32::consts::PI;

/// 0: サイン波（フル）
pub fn gen_op_sine() -> WaveTable {
    gen_from_fn(|p| (2.0 * PI * p).sin())
}

/// 1: sin²（フル、符号付き）— 奇数次倍音が緩く乗る柔らかい音
pub fn gen_op_sin2() -> WaveTable {
    gen_from_fn(|p| {
        let v = (2.0 * PI * p).sin();
        v * v.abs()
    })
}

/// 2: ハーフサイン（前半のみ、後半は無音）
pub fn gen_op_half_sine() -> WaveTable {
    gen_from_fn(|p| if p < 0.5 { (2.0 * PI * p).sin() } else { 0.0 })
}

/// 3: ハーフsin²（前半のみ、後半は無音）
pub fn gen_op_half_sin2() -> WaveTable {
    gen_from_fn(|p| {
        if p < 0.5 {
            let v = (2.0 * PI * p).sin(); // [0,0.5) では sin≥0 なので v*v.abs()=v²
            v * v
        } else {
            0.0
        }
    })
}

/// 4: 2倍速サイン前半（2倍速の1周期を前半に収め、後半は無音。正負両方含む）
pub fn gen_op_half_sine_2x() -> WaveTable {
    gen_from_fn(|p| if p < 0.5 { (4.0 * PI * p).sin() } else { 0.0 })
}

/// 5: 2倍速sin²前半（前半のみ、符号付き）
pub fn gen_op_half_sin2_2x() -> WaveTable {
    gen_from_fn(|p| {
        if p < 0.5 {
            let v = (4.0 * PI * p).sin();
            v * v.abs()
        } else {
            0.0
        }
    })
}

/// 6: 2倍速絶対値サイン前半（|sin|の2倍速、前半のみ。ymfmのラップ効果で常に正）
pub fn gen_op_half_abs_sine_2x() -> WaveTable {
    gen_from_fn(|p| if p < 0.5 { (4.0 * PI * p).sin().abs() } else { 0.0 })
}

/// 7: 2倍速正sin²前半（sin²の2倍速・常に正・前半のみ）
pub fn gen_op_half_pos_sin2_2x() -> WaveTable {
    gen_from_fn(|p| {
        if p < 0.5 {
            let v = (4.0 * PI * p).sin();
            v * v // sin²(4πp)、常に≥0
        } else {
            0.0
        }
    })
}

/// 波形番号0〜7に対応する波形テーブルを生成する。
pub fn gen_builtin_waveform(index: u8) -> WaveTable {
    match index {
        0 => gen_op_sine(),
        1 => gen_op_sin2(),
        2 => gen_op_half_sine(),
        3 => gen_op_half_sin2(),
        4 => gen_op_half_sine_2x(),
        5 => gen_op_half_sin2_2x(),
        6 => gen_op_half_abs_sine_2x(),
        _ => gen_op_half_pos_sin2_2x(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const WAVE_LEN: usize = 1024;

    #[test]
    fn all_waveforms_have_correct_length() {
        for i in 0..8u8 {
            let t = gen_builtin_waveform(i);
            assert_eq!(t.len(), WAVE_LEN, "waveform {i}");
        }
    }

    #[test]
    fn sine_representative_points() {
        let t = gen_op_sine();
        assert!((t.sample_at(0) - 0.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN / 4) - 1.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN / 2) - 0.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN * 3 / 4) - (-1.0)).abs() < 0.01);
    }

    #[test]
    fn sin2_has_same_sign_as_sine_and_smaller_midpoint_amplitude() {
        let t = gen_op_sin2();
        assert!((t.sample_at(0) - 0.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN / 4) - 1.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN / 2) - 0.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN * 3 / 4) - (-1.0)).abs() < 0.01);
        // p=0.125: sin(π/4)≈0.707 → sin²×sign ≈ 0.5 < sine の 0.707
        let mid = t.sample_at(WAVE_LEN / 8);
        assert!(mid > 0.0 && mid < 0.65, "sin2 midpoint {mid}");
        assert!(t.sample_at(WAVE_LEN * 5 / 8) < 0.0);
    }

    #[test]
    fn half_sine_second_half_is_silent() {
        let t = gen_op_half_sine();
        assert!((t.sample_at(WAVE_LEN / 4) - 1.0).abs() < 0.01);
        assert!(t.sample_at(WAVE_LEN * 3 / 4).abs() < 1e-3);
    }

    #[test]
    fn half_sin2_first_half_positive_second_silent() {
        let t = gen_op_half_sin2();
        assert!(t.sample_at(WAVE_LEN / 8) > 0.0);
        assert!((t.sample_at(WAVE_LEN / 4) - 1.0).abs() < 0.01);
        assert!(t.sample_at(WAVE_LEN * 3 / 4).abs() < 1e-3);
    }

    // --- wave4: 2倍速サイン前半（正負両方含む）---

    #[test]
    fn half_sine_2x_full_cycle_in_first_half_then_silent() {
        let t = gen_op_half_sine_2x();
        // p=0.125: sin(π/2)=1（正のピーク）
        assert!((t.sample_at(WAVE_LEN / 8) - 1.0).abs() < 0.01);
        // p=0.375: sin(3π/2)=-1（負のトラフ。旧実装は0だったが正しくは-1）
        assert!((t.sample_at(WAVE_LEN * 3 / 8) - (-1.0)).abs() < 0.01);
        // p=0.625: 後半 → 無音
        assert!(t.sample_at(WAVE_LEN * 5 / 8).abs() < 1e-3);
        assert!(t.sample_at(WAVE_LEN * 7 / 8).abs() < 1e-3);
    }

    // --- wave5: 2倍速sin²前半（符号付き）---

    #[test]
    fn half_sin2_2x_signed_in_first_half_then_silent() {
        let t = gen_op_half_sin2_2x();
        // p=0.125: sin²×sign = +1（正のピーク）
        assert!((t.sample_at(WAVE_LEN / 8) - 1.0).abs() < 0.01);
        // p=0.375: sin²×sign = -1（負のトラフ）
        assert!((t.sample_at(WAVE_LEN * 3 / 8) - (-1.0)).abs() < 0.01);
        // p=0.5以降: 無音
        assert!(t.sample_at(WAVE_LEN / 2).abs() < 1e-3);
        assert!(t.sample_at(WAVE_LEN * 3 / 4).abs() < 1e-3);
    }

    // --- wave6: 2倍速絶対値サイン前半（常に正）---

    #[test]
    fn half_abs_sine_2x_always_non_negative_and_second_half_silent() {
        let t = gen_op_half_abs_sine_2x();
        // 前半は常に正（絶対値）
        for i in 0..(WAVE_LEN / 2) {
            assert!(t.sample_at(i) >= -1e-4, "index {i}: {}", t.sample_at(i));
        }
        // p=0.125 と p=0.375 どちらもピーク≈1（wave4と異なり負にならない）
        assert!((t.sample_at(WAVE_LEN / 8) - 1.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN * 3 / 8) - 1.0).abs() < 0.01);
        // 後半: 無音
        assert!(t.sample_at(WAVE_LEN * 3 / 4).abs() < 1e-3);
    }

    // --- wave7: 2倍速正sin²前半（常に正）---

    #[test]
    fn half_pos_sin2_2x_always_non_negative_and_second_half_silent() {
        let t = gen_op_half_pos_sin2_2x();
        for i in 0..WAVE_LEN {
            assert!(t.sample_at(i) >= -1e-4, "index {i}: {}", t.sample_at(i));
        }
        assert!((t.sample_at(WAVE_LEN / 8) - 1.0).abs() < 0.01);
        assert!((t.sample_at(WAVE_LEN * 3 / 8) - 1.0).abs() < 0.01);
        assert!(t.sample_at(WAVE_LEN * 3 / 4).abs() < 1e-3);
    }
}
