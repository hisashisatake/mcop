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

// ---------------------------------------------------------------------------
// 基本波 × OPZ8変換 の汎用展開（waveform 8〜31）
//
// OPZの8変換（full / squared / half / half-squared / 2x-half / 2x-squared-half /
// 2x-abs-half / 2x-pos-squared-half）を、サイン以外の基本波（ノコギリ/矩形/三角）にも
// 同じ規則で適用する。波形番号 = 基本波index×8 + 変換index:
//   0–7  : サイン系（gen_op_* に委譲＝OPZ忠実、完全不変）
//   8–15 : ノコギリ系
//   16–23: 矩形系
//   24–31: 三角系
//   32–63: ノイズ（NOISE_WAVEFORM_BASE〜、テーブルではなくLFSRでランタイム生成）
//   64以降: ユーザー定義波形（op505-coreのset_user_wave、FIRST_USER_WAVE_SLOT以降）
// ---------------------------------------------------------------------------

/// ビルトイン波形の総数（4基本波 × 8変換）。スロット0〜31を占有。
pub const BUILTIN_WAVEFORM_COUNT: u8 = 32;

// ---------------------------------------------------------------------------
// ノイズ波形（波形番号 32〜63、実機SSG準拠32段階）
//
// テーブルルックアップ方式では32サンプルループの周期性でピッチが出てしまい
// ノイズにならない。そのため Operator::tick がこの番号レンジを検出すると、
// テーブル参照の代わりに17bit LFSR（AY-3-8910互換）でランタイム生成する。
//
// 実機SSGのノイズは「1種類の白色LFSRのクロックレートをノイズ周期NP(0〜31)で変える」もの。
// 色 = waveform - 32 = NP。LFSRのシフトレート（サンプル&ホールドの更新頻度）を
// noise_clock_rate(color) で決める：NP小=高速更新=広帯域（白）、NP大=低速更新=低域寄り。
// ---------------------------------------------------------------------------

/// ノイズ波形の開始番号。32〜63 がノイズ（color=NP 0〜31）。
pub const NOISE_WAVEFORM_BASE: u8 = 32;
/// ノイズの段階数（実機SSGのノイズ周期 NP 0〜31 に対応）。
pub const NOISE_LEVELS: u8 = 32;

/// ノイズ更新クロックの基準周波数（Hz）。YM2203/YM2608 の内部SSG標準クロック2MHz相当。
/// 実機ノイズ周波数 f = ssg_clock/(16×NP) を、エンジンが知らないチップ固有クロックに依存せず
/// 再現するための固定基準（spec-sound.md 参照）。
pub const NOISE_BASE_CLOCK: f32 = 2_000_000.0;

/// 波形番号がノイズレンジ（32〜63）かどうか。
#[inline]
pub fn is_noise_waveform(wf: u8) -> bool {
    wf >= NOISE_WAVEFORM_BASE && wf < NOISE_WAVEFORM_BASE + NOISE_LEVELS
}

/// ノイズ波形番号 → 色インデックス（=NP、0〜31）。レンジ外でも飽和して安全な値を返す。
#[inline]
pub fn noise_color(wf: u8) -> u8 {
    wf.saturating_sub(NOISE_WAVEFORM_BASE).min(NOISE_LEVELS - 1)
}

/// 色(=NP) → LFSR更新レート（Hz）。`NOISE_BASE_CLOCK / (16 × max(NP,1))`。
/// NP=0 は実機同様 1 扱い。NP小=高レート（広帯域/白）、NP大=低レート（低域寄り）。
#[inline]
pub fn noise_clock_rate(color: u8) -> f32 {
    NOISE_BASE_CLOCK / (16.0 * color.max(1) as f32)
}

/// ノコギリ波（1周期 [0,1) を -1→+1 の上昇ランプ）。
fn base_saw(p: f32) -> f32 {
    2.0 * p - 1.0
}

/// バイポーラパルス（duty デューティ比で +1、残りは -1）。duty=0.5 で矩形波。
fn bipolar_pulse(duty: f32, p: f32) -> f32 {
    if p < duty { 1.0 } else { -1.0 }
}

/// 矩形系ファミリー（波形16〜23、variant 0〜7）。
///
/// 矩形波は b=±1 のため OPZ の2乗/絶対値変換が縮退して重複する。代わりに
/// **パルス幅変調（PWM）の duty スイープ ＋ OPZ由来の half / 2x 形**で8種すべて別物にする
/// （ユーザー方針2026-06-21「矩形波はB」）。
/// - 0–5: バイポーラPWM デューティ 50/33/25/16.7/12.5/6.25%（太い→細い＝倍音が増える）
/// - 6  : ハーフ矩形（前半+1/後半0、ユニポーラ）
/// - 7  : 2倍速矩形の前半（+1,-1 を前半に収め後半無音）
fn gen_square_family(variant: u8) -> WaveTable {
    gen_from_fn(move |p| match variant {
        0 => bipolar_pulse(0.5, p),
        1 => bipolar_pulse(1.0 / 3.0, p),
        2 => bipolar_pulse(0.25, p),
        3 => bipolar_pulse(1.0 / 6.0, p),
        4 => bipolar_pulse(0.125, p),
        5 => bipolar_pulse(0.0625, p),
        6 => if p < 0.5 { 1.0 } else { 0.0 },
        _ => {
            if p < 0.25 {
                1.0
            } else if p < 0.5 {
                -1.0
            } else {
                0.0
            }
        }
    })
}

/// 三角波（サイン位相に揃える: 0→+1→0→-1→0、p=0.25で+1ピーク）。
fn base_triangle(p: f32) -> f32 {
    if p < 0.25 {
        4.0 * p
    } else if p < 0.75 {
        2.0 - 4.0 * p
    } else {
        4.0 * p - 4.0
    }
}

/// 基本波 `base`（1周期 [0,1)→[-1,1]）に OPZ8変換の `variant`（0〜7）を適用した瞬時値。
///
/// サイン基本波で評価すると `gen_op_*`（0〜7）と数式的に一致する（half区間で sin≥0 のため
/// `sq(x)=x·|x|` が x² と一致する点まで含めて等価）。
fn opz_variant(base: fn(f32) -> f32, variant: u8, p: f32) -> f32 {
    // 符号付き2乗（OPZ wf1/wf3/wf5 系: x·|x|）。
    let sq = |x: f32| x * x.abs();
    match variant {
        0 => base(p),                                       // full
        1 => sq(base(p)),                                   // squared（符号付き）
        2 => if p < 0.5 { base(p) } else { 0.0 },           // half
        3 => if p < 0.5 { sq(base(p)) } else { 0.0 },       // half-squared
        4 => if p < 0.5 { base(2.0 * p) } else { 0.0 },     // 2x-half
        5 => if p < 0.5 { sq(base(2.0 * p)) } else { 0.0 }, // 2x-squared-half（符号付き）
        6 => if p < 0.5 { base(2.0 * p).abs() } else { 0.0 }, // 2x-abs-half
        _ => if p < 0.5 { let v = base(2.0 * p); v * v } else { 0.0 }, // 2x-pos-squared-half（常に正）
    }
}

/// 波形番号 0〜31 に対応する波形テーブルを生成する（範囲外は0=サインへフォールバック）。
pub fn gen_builtin_waveform(index: u8) -> WaveTable {
    match index {
        // 0–7: サイン系は既存実装に委譲（OPZ忠実、完全不変）
        0 => gen_op_sine(),
        1 => gen_op_sin2(),
        2 => gen_op_half_sine(),
        3 => gen_op_half_sin2(),
        4 => gen_op_half_sine_2x(),
        5 => gen_op_half_sin2_2x(),
        6 => gen_op_half_abs_sine_2x(),
        7 => gen_op_half_pos_sin2_2x(),
        // 8–15: ノコギリ系（基本波 saw × OPZ8変換、独自拡張）
        8..=15 => {
            let variant = index - 8;
            gen_from_fn(move |p| opz_variant(base_saw, variant, p))
        }
        // 16–23: 矩形系（PWMファミリー、独自拡張。OPZ縮退回避のため専用セット）
        16..=23 => gen_square_family(index - 16),
        // 24–31: 三角系（基本波 triangle × OPZ8変換、独自拡張）
        24..=31 => {
            let variant = index - 24;
            gen_from_fn(move |p| opz_variant(base_triangle, variant, p))
        }
        _ => gen_op_sine(),
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
    fn noise_range_detection_and_color() {
        // ビルトイン(0-31)はノイズではない
        assert!(!is_noise_waveform(0));
        assert!(!is_noise_waveform(31));
        // 32〜63 がノイズ（color=NP 0〜31）
        assert!(is_noise_waveform(32));
        assert!(is_noise_waveform(63));
        assert!(!is_noise_waveform(64)); // 64以降はユーザー定義
        assert_eq!(noise_color(32), 0);
        assert_eq!(noise_color(63), 31);
    }

    #[test]
    fn noise_clock_rate_decreases_with_np() {
        // NP小=高レート（広帯域）、NP大=低レート（低域）。単調減少。
        assert!(noise_clock_rate(1) > noise_clock_rate(31));
        // NP=0 は 1 扱い（実機準拠）
        assert_eq!(noise_clock_rate(0), noise_clock_rate(1));
        // 基準2MHz, NP=1 → 125kHz
        assert!((noise_clock_rate(1) - 125_000.0).abs() < 1.0);
    }

    #[test]
    fn all_waveforms_have_correct_length() {
        for i in 0..BUILTIN_WAVEFORM_COUNT {
            let t = gen_builtin_waveform(i);
            assert_eq!(t.len(), WAVE_LEN, "waveform {i}");
        }
    }

    #[test]
    fn sine_block_unchanged_vs_gen_op_functions() {
        // 0–7 のサイン系は既存 gen_op_* に委譲＝OPZ忠実・完全不変であることを担保。
        let ops: [fn() -> WaveTable; 8] = [
            gen_op_sine, gen_op_sin2, gen_op_half_sine, gen_op_half_sin2,
            gen_op_half_sine_2x, gen_op_half_sin2_2x, gen_op_half_abs_sine_2x, gen_op_half_pos_sin2_2x,
        ];
        for (i, f) in ops.iter().enumerate() {
            let a = gen_builtin_waveform(i as u8);
            let b = f();
            for k in 0..WAVE_LEN {
                assert!((a.sample_at(k) - b.sample_at(k)).abs() < 1e-6, "wf{i} idx{k}");
            }
        }
    }

    #[test]
    fn opz_variant_matches_sine_base() {
        // 汎用トランスフォームをサイン基本波で評価すると gen_op_* と一致する。
        fn base_sine(p: f32) -> f32 { (2.0 * PI * p).sin() }
        let ops: [fn() -> WaveTable; 8] = [
            gen_op_sine, gen_op_sin2, gen_op_half_sine, gen_op_half_sin2,
            gen_op_half_sine_2x, gen_op_half_sin2_2x, gen_op_half_abs_sine_2x, gen_op_half_pos_sin2_2x,
        ];
        for v in 0..8u8 {
            let generic = gen_from_fn(move |p| opz_variant(base_sine, v, p));
            let expected = ops[v as usize]();
            for k in 0..WAVE_LEN {
                assert!((generic.sample_at(k) - expected.sample_at(k)).abs() < 1e-6, "variant{v} idx{k}");
            }
        }
    }

    #[test]
    fn saw_full_is_rising_ramp() {
        let t = gen_builtin_waveform(8); // saw v0
        assert!((t.sample_at(0) - (-1.0)).abs() < 0.02);          // 始点 ≈ -1
        assert!(t.sample_at(WAVE_LEN / 2).abs() < 0.02);          // 中点 ≈ 0
        assert!((t.sample_at(WAVE_LEN * 3 / 4) - 0.5).abs() < 0.02); // 3/4 ≈ +0.5
    }

    #[test]
    fn square_family_duty_narrows() {
        // 16=50%, 18=25%, 20=12.5%: +1区間が狭まる（立ち下がり位置で確認）。
        let sq50 = gen_builtin_waveform(16);
        let sq25 = gen_builtin_waveform(18);
        let sq12 = gen_builtin_waveform(20);
        assert!(sq50.sample_at(WAVE_LEN * 3 / 8) > 0.0); // 0.375 < 0.5 → +1
        assert!(sq25.sample_at(WAVE_LEN * 3 / 8) < 0.0); // 0.375 > 0.25 → -1
        assert!(sq12.sample_at(WAVE_LEN / 4) < 0.0);     // 0.25 > 0.125 → -1
        // 6=ハーフ矩形は後半が無音
        assert!(gen_builtin_waveform(22).sample_at(WAVE_LEN * 3 / 4).abs() < 1e-3);
    }

    #[test]
    fn triangle_full_peaks_at_quarter() {
        let t = gen_builtin_waveform(24); // triangle v0
        assert!(t.sample_at(0).abs() < 0.02);                       // 0 ≈ 0
        assert!((t.sample_at(WAVE_LEN / 4) - 1.0).abs() < 0.02);    // 1/4 ≈ +1
        assert!(t.sample_at(WAVE_LEN / 2).abs() < 0.02);            // 1/2 ≈ 0
        assert!((t.sample_at(WAVE_LEN * 3 / 4) - (-1.0)).abs() < 0.02); // 3/4 ≈ -1
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
