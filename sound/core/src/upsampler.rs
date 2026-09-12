// ---------------------------------------------------------------------------
// 2倍アップサンプラー（ハーフバンドFIR）
// ---------------------------------------------------------------------------
//
// standalone/smf2op505が内部レートを下げて（例: 24kHz）FM合成本体のコストを削減し、
// デバイス/出力へ渡す直前にここで2倍（例: 48kHz）へ引き伸ばすための後段。
//
// ハーフバンド構成（31タップ、windowed sinc、Blackman窓）を採用しているため、
// 偶数位相（元の入力サンプルの位置）の理論ゲインはちょうど1になる。この位相は
// 実際には乗算・畳み込みを一切行わず、履歴バッファから該当サンプルをそのまま
// コピーするだけで実現する（浮動小数点の丸め誤差を持ち込まないため。内部レートと
// 出力レートが一致する`div=1`運用と、この関数を通した`div=2`運用の偶数位相サンプルが
// ビット単位で一致することをテストで保証する）。
//
// 奇数位相（新しく生成する中間サンプル）だけが、対称16タップ（オフセット
// ±1,±3,...,±15）のFIRで補間される。中心タップ(0.5)は上記の理由で係数化せず、
// 奇数タップ側だけをDCゲインが全体で1になるよう再正規化する
// （center 0.5 + 2×sum(奇数タップ) == 1.0 が目標、詳細は`odd_taps()`のコメント参照）。
//
// 群遅延は入力8フレーム分（48kHz換算で約0.17ms、実用上無視できる）。

use std::sync::OnceLock;

/// 履歴として保持するフレーム数（FIRが参照する範囲、オフセット0〜15をちょうど覆う）。
const HISTORY_FRAMES: usize = 16;
/// 奇数位相FIRの片側タップ数（オフセット1,3,...,15の8個。対称なので反対側も同じ係数）。
const HALF_TAPS: usize = 8;

/// 奇数オフセット(1,3,...,15)側の窓掛け・正規化済みFIR係数。
///
/// 理想ハーフバンドの奇数タップは `h[o] = (-1)^((o-1)/2) / (pi*o)`（`o`は奇数オフセット、
/// 補間のための×2ゲイン補正はまだ含まない）。ここで返す値は、その×2補正まで込みで
/// `acc = Σ taps[k]*(a+b)` がそのまま最終出力になるよう正規化してある。
/// DC入力Cに対し `acc = 2C * Σtaps` なので、通過位相(center、`2*0.5=1`のゲインを
/// 直接コピーで実現)と揃えるには `Σtaps == 0.5` が目標値になる（31タップへの打ち切りで
/// 理想値からずれる分をBlackman窓適用後に再正規化して補正する）。
fn odd_taps() -> &'static [f32; HALF_TAPS] {
    static TABLE: OnceLock<[f32; HALF_TAPS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut raw = [0.0f64; HALF_TAPS];
        for (k, slot) in raw.iter_mut().enumerate() {
            let o = (2 * k + 1) as f64;
            let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
            let ideal = sign / (std::f64::consts::PI * o);
            // Blackman窓（全長31、中心(タップ15)からの距離o）。
            let n = 15.0 + o;
            let window = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n / 30.0).cos()
                + 0.08 * (4.0 * std::f64::consts::PI * n / 30.0).cos();
            *slot = ideal * window;
        }
        let raw_sum: f64 = raw.iter().sum();
        let scale = 0.5 / raw_sum;
        let mut out = [0.0f32; HALF_TAPS];
        for (k, slot) in out.iter_mut().enumerate() {
            *slot = (raw[k] * scale) as f32;
        }
        out
    })
}

/// 内部レートでレンダリングした音声を2倍にアップサンプリングする（例: 24kHz→48kHz）。
///
/// チャンネルごとに直近16フレームの円環履歴を持つ。1入力フレームにつき2出力フレームを
/// 生成するため、`process()`の呼び出し側が要求する出力フレーム数が奇数だと1フレーム
/// 余る。この余りは`pending_frame`（最大1フレーム分）に退避し、次回の`process()`呼び出し
/// の先頭で消費する——ブロック境界をまたいでも音が途切れない。
pub struct Upsampler2x {
    num_channels: usize,
    /// 直近`HISTORY_FRAMES`フレーム分の円環履歴（フレーム単位で連結、`num_channels`個ずつ）。
    history: Vec<f32>,
    /// 次に書き込むフレームの位置（0..HISTORY_FRAMES）。
    write_frame: usize,
    /// 前回の`process()`で生成済みだがまだ書き出していない1フレーム分の持ち越し。
    pending_frame: Option<Vec<f32>>,
}

impl Upsampler2x {
    /// `num_channels`分の履歴をあらかじめ確保する（オーディオスレッドでの確保を避けるため）。
    pub fn new(num_channels: usize) -> Self {
        Self {
            num_channels,
            history: vec![0.0; HISTORY_FRAMES * num_channels],
            write_frame: 0,
            pending_frame: None,
        }
    }

    /// `output_frames`フレームを`process()`で得るために、事前に内部レートで
    /// レンダリングして`process()`へ渡すべき入力フレーム数。
    pub fn input_frames_for(&self, output_frames: usize) -> usize {
        let available = usize::from(self.pending_frame.is_some());
        output_frames.saturating_sub(available).div_ceil(2)
    }

    /// `rel`（0=最新〜15=最も古い）だけ遡ったフレームのチャンネル`ch`の値。
    fn frame_at(&self, newest_idx: usize, rel: usize, ch: usize) -> f32 {
        let idx = (newest_idx + HISTORY_FRAMES - rel) % HISTORY_FRAMES;
        self.history[idx * self.num_channels + ch]
    }

    /// 新しい入力フレームを履歴へ積み、確定した出力ペア（通過位相, FIR位相）を返す。
    fn push_and_compute(&mut self, input_frame: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let ch_count = self.num_channels;
        let newest_idx = self.write_frame;
        for ch in 0..ch_count {
            self.history[newest_idx * ch_count + ch] = input_frame[ch];
        }
        self.write_frame = (self.write_frame + 1) % HISTORY_FRAMES;

        let taps = odd_taps();
        let mut through = vec![0.0f32; ch_count];
        let mut fir = vec![0.0f32; ch_count];
        for ch in 0..ch_count {
            // 通過位相: 8フレーム前の入力をそのままコピー（乗算しない＝ビット一致を保つ）。
            through[ch] = self.frame_at(newest_idx, 8, ch);
            let mut acc = 0.0f32;
            for (k, tap) in taps.iter().enumerate() {
                let a = self.frame_at(newest_idx, 8 + k, ch);
                let b = self.frame_at(newest_idx, 7 - k, ch);
                acc += tap * (a + b);
            }
            fir[ch] = acc;
        }
        (through, fir)
    }

    /// `input`（内部レート、`num_channels`インターリーブ）を消費し、`output`
    /// （出力レート、`num_channels`インターリーブ）を過不足なく埋める。
    ///
    /// `output.len() / num_channels` は、直前に`input_frames_for()`へ渡した値に対する
    /// `input.len() / num_channels`（呼び出し側がその通りにレンダリングした前提）から
    /// 一意に決まる。この対応が崩れている呼び出しは`debug_assert`で検出する。
    pub fn process(&mut self, input: &[f32], output: &mut [f32], num_channels: usize) {
        assert_eq!(num_channels, self.num_channels, "構築時と異なるnum_channelsで呼ばれた");
        assert_eq!(input.len() % num_channels, 0, "inputの長さがnum_channelsの倍数でない");
        assert_eq!(output.len() % num_channels, 0, "outputの長さがnum_channelsの倍数でない");
        let in_frames = input.len() / num_channels;
        let out_frames = output.len() / num_channels;

        let mut out_pos = 0usize;

        if let Some(p) = self.pending_frame.take() {
            output[out_pos * num_channels..(out_pos + 1) * num_channels].copy_from_slice(&p);
            out_pos += 1;
        }

        for f in 0..in_frames {
            let (through, fir) =
                self.push_and_compute(&input[f * num_channels..(f + 1) * num_channels]);

            debug_assert!(out_pos < out_frames, "input_frames_for()と食い違う呼び出し（通過位相の出力先が無い）");
            if out_pos < out_frames {
                output[out_pos * num_channels..(out_pos + 1) * num_channels].copy_from_slice(&through);
                out_pos += 1;
            }

            if out_pos < out_frames {
                output[out_pos * num_channels..(out_pos + 1) * num_channels].copy_from_slice(&fir);
                out_pos += 1;
            } else {
                self.pending_frame = Some(fir);
            }
        }

        debug_assert_eq!(out_pos, out_frames, "outputのフレーム数がinputと整合しない");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_frames_for_rounds_up_without_pending() {
        let up = Upsampler2x::new(1);
        assert_eq!(up.input_frames_for(0), 0);
        assert_eq!(up.input_frames_for(1), 1);
        assert_eq!(up.input_frames_for(2), 1);
        assert_eq!(up.input_frames_for(3), 2);
        assert_eq!(up.input_frames_for(4), 2);
    }

    #[test]
    fn input_frames_for_accounts_for_pending_frame() {
        let mut up = Upsampler2x::new(1);
        // 出力3フレーム要求→入力2フレーム消費、1フレーム余ってpendingへ。
        let input = [1.0f32, 2.0];
        let mut output = [0.0f32; 3];
        up.process(&input, &mut output, 1);
        assert!(up.pending_frame.is_some());
        // pendingがあるので、次に出力1フレームだけならもう入力は要らない。
        assert_eq!(up.input_frames_for(1), 0);
        assert_eq!(up.input_frames_for(2), 1);
    }

    #[test]
    fn dc_input_produces_dc_output_with_unity_gain() {
        let mut up = Upsampler2x::new(1);
        let input = vec![0.5f32; 64];
        let mut output = vec![0.0f32; 128];
        up.process(&input, &mut output, 1);

        // 立ち上がり(最初の数フレーム、履歴がゼロで埋まっている区間)を除いて
        // ほぼ完全に0.5へ収束していること。
        for &v in &output[64..] {
            assert!((v - 0.5).abs() < 1e-4, "DC入力はDC出力になるはず: {v}");
        }
    }

    #[test]
    fn even_phase_is_bit_identical_passthrough() {
        // 通過位相(偶数インデックスの出力)は、8フレーム前の入力とビット単位で一致する。
        let mut up = Upsampler2x::new(1);
        let mut input = vec![0.0f32; 32];
        for (i, v) in input.iter_mut().enumerate() {
            *v = (i as f32) * 0.01 + 0.001; // 単純な丸め誤差を作りにくい値
        }
        let mut output = vec![0.0f32; 64];
        up.process(&input, &mut output, 1);

        // output[2*j] は input[j-8] のはず（j>=8の範囲で確認、それ以前は履歴がゼロ埋め）。
        for j in 8..input.len() {
            assert_eq!(output[2 * j], input[j - 8], "通過位相はビット一致するはず (j={j})");
        }
    }

    /// 1kHz正弦（十分にパスバンド内、カットオフ12kHzに対して余裕あり）を24kHzで作って
    /// 2倍にアップサンプリングし、48kHzで直接作った正弦とほぼ一致することを確認する。
    ///
    /// 群遅延の理論値は16出力サンプル（通過位相の直接コピーで確認済み）だが、通過位相と
    /// FIR位相を単純に整数16サンプルシフトだけで揃えて比較すると、ごく小さな非整数の
    /// 群遅延成分（0.1サンプル未満）に対して誤差が過大に見える（1kHzで最大誤差2%超）。
    /// そこで期待値側の位相をわずかに動かせる自由パラメータ`epsilon`（±1サンプル）を
    /// 導入し、誤差が最小になる`epsilon`を探してから、その最小誤差とepsilonの大きさの
    /// 両方をしきい値でチェックする（epsilonが大きければ本物の遅延不整合、最小誤差が
    /// 大きければゲイン異常—という切り分けができる）。
    #[test]
    fn sine_upsampling_matches_native_high_rate_sine() {
        let low_rate = 24_000.0f32;
        let high_rate = 48_000.0f32;
        let n_low = 960; // 24kHzで40ms
        let freq = 1000.0f32;

        let low: Vec<f32> = (0..n_low)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / low_rate).sin())
            .collect();
        let mut up = Upsampler2x::new(1);
        let mut high = vec![0.0f32; n_low * 2];
        up.process(&low, &mut high, 1);

        let delay_out = 16usize;
        let start = delay_out + 64;
        let end = high.len() - 64;
        let w = 2.0 * std::f64::consts::PI * (freq as f64) / (high_rate as f64);

        let mut best_epsilon = 0.0f64;
        let mut best_err = f64::MAX;
        let mut eps_steps = -100i32;
        while eps_steps <= 100 {
            let epsilon = eps_steps as f64 * 0.01; // -1.00..1.00サンプル、0.01刻み
            let mut max_err = 0.0f64;
            for i in start..end {
                let t = (i - delay_out) as f64 - epsilon;
                let expected = (w * t).sin();
                let actual = high[i] as f64;
                max_err = max_err.max((actual - expected).abs());
            }
            if max_err < best_err {
                best_err = max_err;
                best_epsilon = epsilon;
            }
            eps_steps += 1;
        }

        assert!(best_epsilon.abs() < 0.3, "群遅延が理論値16サンプルから大きくずれている: epsilon={best_epsilon}");
        assert!(best_err < 0.005, "最適な遅延補正をしてもなお誤差が大きい(ゲイン異常の疑い): {best_err}");
    }

    #[test]
    fn odd_output_request_carries_leftover_frame_across_calls() {
        let ch = 1;
        let mut a = Upsampler2x::new(ch);
        let input_full: Vec<f32> = (0..20).map(|i| i as f32 * 0.1).collect();
        let mut expected = vec![0.0f32; 40];
        a.process(&input_full, &mut expected, ch);

        // 同じ入力を、奇数フレームずつ細切れに投入した場合でも同じ結果になること。
        let mut b = Upsampler2x::new(ch);
        let mut actual = vec![0.0f32; 40];
        let mut in_pos = 0usize;
        let mut out_pos = 0usize;
        let chunk_out_frames = [3usize, 5, 7, 9, 11, 5]; // 合計40出力フレーム=40サンプル
        for &of in &chunk_out_frames {
            let needed_in = b.input_frames_for(of);
            b.process(
                &input_full[in_pos..in_pos + needed_in],
                &mut actual[out_pos..out_pos + of],
                ch,
            );
            in_pos += needed_in;
            out_pos += of;
        }
        assert_eq!(in_pos, input_full.len());
        assert_eq!(out_pos, expected.len());
        assert_eq!(actual, expected, "細切れ投入でも一括処理と同じ結果になるはず");
    }

    #[test]
    fn stereo_channels_do_not_mix() {
        let ch = 2;
        let mut up = Upsampler2x::new(ch);
        let mut input = vec![0.0f32; 32 * ch];
        for f in 0..32 {
            input[f * ch] = 1.0; // L=1.0固定
            input[f * ch + 1] = -1.0; // R=-1.0固定
        }
        let mut output = vec![0.0f32; 64 * ch];
        up.process(&input, &mut output, ch);

        // 最初の16出力フレーム分(通過位相の参照が8入力フレーム前を指すことに由来する
        // 起動時の遷移区間、履歴がゼロ埋めの状態を引きずる)を除いた定常区間で確認する。
        for f in 32..64 {
            let l = output[f * ch];
            let r = output[f * ch + 1];
            assert!((l - 1.0).abs() < 1e-3, "Lチャンネルに R の値が混ざっていないはず: {l}");
            assert!((r + 1.0).abs() < 1e-3, "Rチャンネルに L の値が混ざっていないはず: {r}");
        }
    }
}
