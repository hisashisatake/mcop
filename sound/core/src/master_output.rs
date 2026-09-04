// ---------------------------------------------------------------------------
// マスターボリューム + レベル計測（全エフェクトスロット合算後の最終段）
// ---------------------------------------------------------------------------

use crate::AudioProcessor;

/// GUIへ渡す計測スナップショット。フィールドを増やすだけで将来の拡張
/// （例: オシロスコープ用の波形バッファ）に対応できる形にしてある。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measurement {
    pub peak_l: f32,
    pub peak_r: f32,
    pub clipped: bool,
}

/// 全エフェクトスロット合算後の最終段。マスターボリューム適用・ピーク計測・
/// クリップ検出を担う（`MasterEffects`はスロットごとに複数個持たれるため、
/// 全体で1個であるべきこれらの値はここに置く）。
pub struct MasterOutput {
    volume: u8,
    peak_l: f32,
    peak_r: f32,
    clipped: bool,
}

impl MasterOutput {
    pub fn new() -> Self {
        Self { volume: 255, peak_l: 0.0, peak_r: 0.0, clipped: false }
    }

    /// マスターボリューム（0〜255、既定255＝無補正）。GM2準拠のチャンネルボリューム
    /// と同じ二乗カーブ（`(v/255)^2`）を使う（`channel_gain()`と同じ考え方）。
    pub fn set_volume(&mut self, value: u8) {
        self.volume = value;
    }

    /// 直近の計測値を読み出し、内部状態をリセットする（次の計測区間へ備える）。
    pub fn take_measurement(&mut self) -> Measurement {
        let m = Measurement { peak_l: self.peak_l, peak_r: self.peak_r, clipped: self.clipped };
        self.peak_l = 0.0;
        self.peak_r = 0.0;
        self.clipped = false;
        m
    }
}

impl Default for MasterOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioProcessor for MasterOutput {
    fn process(&mut self, buffer: &mut [f32], num_channels: usize) {
        // 既定値(255)ならゲイン乗算はスキップする（既存の値と厳密に同一のビット列を保ち、
        // MasterEffectsの「両センド0なら早期リターン」と同じくゴールデンテストのビット
        // 一致を壊さないため）。ピーク計測は既定値でも行う。
        let apply_gain = self.volume != 255;
        let gain = (self.volume as f32 / 255.0).powi(2);

        for frame in buffer.chunks_exact_mut(num_channels) {
            if apply_gain {
                for s in frame.iter_mut() {
                    *s *= gain;
                }
            }
            let (l, r) = if num_channels >= 2 { (frame[0], frame[1]) } else { (frame[0], frame[0]) };
            self.peak_l = self.peak_l.max(l.abs());
            self.peak_r = self.peak_r.max(r.abs());
            if l.abs() > 1.0 || r.abs() > 1.0 {
                self.clipped = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_volume_is_max() {
        let output = MasterOutput::new();
        assert_eq!(output.volume, 255);
    }

    /// 既定値(255)ではサンプル値が一切変化しないこと（ビット一致維持の要）。
    #[test]
    fn default_volume_is_bit_identical() {
        let mut output = MasterOutput::new();
        let mut buffer = vec![0.0f32; 2 * 100];
        for (i, chunk) in buffer.chunks_exact_mut(2).enumerate() {
            chunk[0] = (i as f32 * 0.1).sin();
            chunk[1] = (i as f32 * 0.1).cos();
        }
        let original = buffer.clone();

        output.process(&mut buffer, 2);

        assert_eq!(original, buffer, "既定ボリューム255では出力が完全に不変であるはず");
    }

    /// 二乗カーブ: 半分の生値でも振幅は1/4になる（`channel_gain()`と同じ考え方）。
    #[test]
    fn volume_uses_squared_curve() {
        let mut output = MasterOutput::new();
        output.set_volume(128); // 約 0.502 -> gain ≈ 0.252

        let mut buffer = vec![1.0f32; 2];
        output.process(&mut buffer, 2);

        let expected_gain = (128.0f32 / 255.0).powi(2);
        assert!((buffer[0] - expected_gain).abs() < 1e-6);
        assert!((buffer[1] - expected_gain).abs() < 1e-6);
    }

    #[test]
    fn volume_zero_silences_output() {
        let mut output = MasterOutput::new();
        output.set_volume(0);

        let mut buffer = vec![1.0f32; 2];
        output.process(&mut buffer, 2);

        assert_eq!(buffer, vec![0.0, 0.0]);
    }

    #[test]
    fn peak_detection_tracks_max_absolute_value() {
        let mut output = MasterOutput::new();
        let mut buffer = vec![0.3, -0.7, 0.5, 0.2];
        output.process(&mut buffer, 2);

        let m = output.take_measurement();
        assert!((m.peak_l - 0.5).abs() < 1e-6, "L: max(|0.3|, |0.5|) = 0.5");
        assert!((m.peak_r - 0.7).abs() < 1e-6, "R: max(|-0.7|, |0.2|) = 0.7");
        assert!(!m.clipped);
    }

    #[test]
    fn take_measurement_resets_state() {
        let mut output = MasterOutput::new();
        let mut buffer = vec![0.9, 0.9];
        output.process(&mut buffer, 2);
        let first = output.take_measurement();
        assert!(first.peak_l > 0.0);

        let second = output.take_measurement();
        assert_eq!(second, Measurement::default(), "読み出し後はリセットされているはず");
    }

    #[test]
    fn clip_detection_triggers_above_unity() {
        let mut output = MasterOutput::new();
        let mut buffer = vec![1.5, -0.2];
        output.process(&mut buffer, 2);

        let m = output.take_measurement();
        assert!(m.clipped, "|1.5| > 1.0 はクリップ扱いになるはず");
    }

    #[test]
    fn clip_flag_does_not_persist_across_measurements() {
        let mut output = MasterOutput::new();
        let mut clipping_buffer = vec![2.0, 2.0];
        output.process(&mut clipping_buffer, 2);
        assert!(output.take_measurement().clipped);

        let mut quiet_buffer = vec![0.1, 0.1];
        output.process(&mut quiet_buffer, 2);
        assert!(!output.take_measurement().clipped, "クリップフラグは計測区間ごとにリセットされるはず");
    }

    #[test]
    fn mono_channel_uses_same_sample_for_l_and_r_peak() {
        let mut output = MasterOutput::new();
        let mut buffer = vec![0.6, -0.4];
        output.process(&mut buffer, 1);

        let m = output.take_measurement();
        assert!((m.peak_l - 0.6).abs() < 1e-6);
        assert!((m.peak_r - 0.6).abs() < 1e-6, "モノラルはL=Rとして計測されるはず");
    }
}
