// ---------------------------------------------------------------------------
// マスターエフェクト（Reverb / Chorus）
// ---------------------------------------------------------------------------

pub mod chorus;
pub mod reverb;

pub use chorus::{Chorus, ChorusType};
pub use reverb::{Reverb, ReverbType};

use crate::AudioProcessor;

/// `Vco::render()`が出力するインターリーブ済みdryバッファに対し、
/// 後段でReverb/Chorusセンドを適用するマスターエフェクト。
///
/// 信号フロー（spec.md マスターエフェクトセクション参照）:
/// ```text
/// chorus_out = chorus.process(dry × chorus_send/255)
/// reverb_in  = dry × reverb_send/255 + chorus_out × chorus_send_to_reverb/255
/// reverb_out = reverb.process(reverb_in)
/// out        = dry + reverb_out + chorus_out
/// ```
pub struct MasterEffects {
    reverb: Reverb,
    chorus: Chorus,
    reverb_send: u8,
    chorus_send: u8,
    chorus_send_to_reverb: u8,
    /// Reverb/Chorusのwet内部状態（ディレイライン等）に一度でも信号が入った可能性があるか。
    /// 両センドが0のまま一度もwetが作られていなければ、wet出力は厳密に0＝出力はdryそのもの
    /// なので、`process()`はバッファに触れず即リターンできる（FDNリバーブ8ライン+コーラスの
    /// 毎サンプル計算を丸ごと省く。エフェクト未使用のVST/smf2wavで常時効く）。
    /// 一度センドが非0になったら以降は常時処理する（センドを0に戻してもテールが残るため）。
    wet_state_active: bool,
}

impl MasterEffects {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            reverb: Reverb::new(sample_rate),
            chorus: Chorus::new(sample_rate),
            reverb_send: 0,
            chorus_send: 0,
            chorus_send_to_reverb: 0,
            wet_state_active: false,
        }
    }

    /// CC91相当：マスターのReverb送りレベル。
    pub fn set_reverb_send(&mut self, value: u8) {
        self.reverb_send = value;
    }

    /// CC93相当：マスターのChorus送りレベル。
    pub fn set_chorus_send(&mut self, value: u8) {
        self.chorus_send = value;
    }

    /// NRPN(0,8)：Chorus出力からReverbへのセンドレベル。
    pub fn set_chorus_send_to_reverb(&mut self, value: u8) {
        self.chorus_send_to_reverb = value;
    }

    /// NRPN(0,2)：Reverb Type。
    pub fn set_reverb_type(&mut self, reverb_type: ReverbType) {
        self.reverb.set_type(reverb_type);
    }

    /// NRPN(0,4)：Reverb Time。
    pub fn set_reverb_time(&mut self, value: u8) {
        self.reverb.set_time(value);
    }

    /// NRPN(0,36)：Delay/Panning DelayタイプのReverb Sync（0=OFF/1以上=ON）。
    /// Room1〜Plateタイプには効果がない。
    pub fn set_reverb_delay_sync(&mut self, value: u8) {
        self.reverb.set_delay_sync(value);
    }

    /// NRPN(0,37)：Delay/Panning Delayのテンポ同期先レート（TimeEgの`sync_rate`と同じ
    /// 0〜255連続値、20音価にアンカーされる）。
    pub fn set_reverb_delay_sync_rate(&mut self, value: u8) {
        self.reverb.set_delay_sync_rate(value);
    }

    /// テンポ（BPM）を設定する。Delay/Panning Delayが同期有効のときのみ使う。
    pub fn set_tempo(&mut self, bpm: f32) {
        self.reverb.set_tempo(bpm);
    }

    /// NRPN(0,3)：Chorus Type。
    pub fn set_chorus_type(&mut self, chorus_type: ChorusType) {
        self.chorus.set_type(chorus_type);
    }

    /// NRPN(0,5)：Chorus Mod Rate。
    pub fn set_chorus_mod_rate(&mut self, value: u8) {
        self.chorus.set_mod_rate(value);
    }

    /// NRPN(0,6)：Chorus Mod Depth。
    pub fn set_chorus_mod_depth(&mut self, value: u8) {
        self.chorus.set_mod_depth(value);
    }

    /// NRPN(0,7)：Chorus Feedback。
    pub fn set_chorus_feedback(&mut self, value: u8) {
        self.chorus.set_feedback(value);
    }
}

impl AudioProcessor for MasterEffects {
    /// インターリーブ済み出力バッファに対し、フレーム単位でReverb/Chorusを適用する。
    /// `num_channels == 1`の場合はモノラルとして扱い、L=Rで処理した結果を平均する。
    fn process(&mut self, buffer: &mut [f32], num_channels: usize) {
        // 完全ドライバイパス: 両センドが0で、かつwet状態が一度も作られていなければ
        // wet出力は厳密に0（ディレイラインはすべて無音）なので何もしない。
        // chorus_send_to_reverbはコーラス入力（dry×chorus_send=0）経由のため単独では影響しない。
        if self.reverb_send != 0 || self.chorus_send != 0 {
            self.wet_state_active = true;
        } else if !self.wet_state_active {
            return;
        }
        let reverb_send = self.reverb_send as f32 / 255.0;
        let chorus_send = self.chorus_send as f32 / 255.0;
        let chorus_send_to_reverb = self.chorus_send_to_reverb as f32 / 255.0;

        for frame in buffer.chunks_exact_mut(num_channels) {
            let (dry_l, dry_r) = if num_channels >= 2 { (frame[0], frame[1]) } else { (frame[0], frame[0]) };

            let (chorus_out_l, chorus_out_r) =
                self.chorus.process(dry_l * chorus_send, dry_r * chorus_send);

            let reverb_in_l = dry_l * reverb_send + chorus_out_l * chorus_send_to_reverb;
            let reverb_in_r = dry_r * reverb_send + chorus_out_r * chorus_send_to_reverb;
            let (reverb_out_l, reverb_out_r) = self.reverb.process(reverb_in_l, reverb_in_r);

            let out_l = dry_l + reverb_out_l + chorus_out_l;
            let out_r = dry_r + reverb_out_r + chorus_out_r;

            if num_channels >= 2 {
                frame[0] = out_l;
                frame[1] = out_r;
            } else {
                frame[0] = (out_l + out_r) * 0.5;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_when_sends_zero() {
        let mut effects = MasterEffects::new(44100.0);

        let mut buffer = vec![0.0f32; 2 * 100];
        for (i, chunk) in buffer.chunks_exact_mut(2).enumerate() {
            chunk[0] = (i as f32 * 0.1).sin();
            chunk[1] = (i as f32 * 0.1).cos();
        }
        let original = buffer.clone();

        effects.process(&mut buffer, 2);

        for (a, b) in original.iter().zip(buffer.iter()) {
            assert!((a - b).abs() < 1e-9, "送りレベル0のときdry信号がそのまま通過するはず: {a} vs {b}");
        }
    }

    /// 一度センドを非0にしてwet状態を作った後は、センドを0に戻してもテール（残響）が
    /// 出力され続けることを確認する（完全ドライバイパスの`wet_state_active`ラッチが
    /// テールを切り落とさないことの回帰テスト）。
    #[test]
    fn dry_bypass_does_not_cut_reverb_tail_after_send_returns_to_zero() {
        let mut effects = MasterEffects::new(44100.0);
        effects.set_reverb_send(255);

        // インパルスを入れてwet状態を作る
        let mut buffer = vec![0.0f32; 2 * 2048];
        buffer[0] = 1.0;
        buffer[1] = 1.0;
        effects.process(&mut buffer, 2);

        // センドを0へ戻し、無音入力を処理してもテールが出続ける
        effects.set_reverb_send(0);
        let mut tail_buffer = vec![0.0f32; 2 * 4096];
        effects.process(&mut tail_buffer, 2);
        let tail_energy: f32 = tail_buffer.iter().map(|x| x * x).sum();
        assert!(tail_energy > 0.0, "センドを0に戻してもwetテールは残るはず");
    }

    #[test]
    fn reverb_send_adds_wet_signal() {
        let mut effects = MasterEffects::new(44100.0);
        effects.set_reverb_send(255);

        // 拡散リバーブのコムフィルターは最大1640サンプル程度のディレイを持つため、
        // テールが現れるまで十分な長さのバッファを用意する。
        let mut buffer = vec![0.0f32; 2 * 4096];
        buffer[0] = 1.0;
        buffer[1] = 1.0;
        effects.process(&mut buffer, 2);

        let tail_energy: f32 = buffer[2..].iter().map(|x| x * x).sum();
        assert!(tail_energy > 0.0, "リバーブのテールが出力に含まれていないはず");
    }

    #[test]
    fn mono_channel_handling() {
        let mut effects = MasterEffects::new(44100.0);
        effects.set_reverb_send(255);
        effects.set_chorus_send(255);

        let mut buffer = vec![0.0f32; 100];
        buffer[0] = 1.0;
        effects.process(&mut buffer, 1);

        for &s in &buffer {
            assert!(s.is_finite());
        }
    }

    #[test]
    fn no_nan_long_run() {
        let mut effects = MasterEffects::new(44100.0);
        effects.set_reverb_send(255);
        effects.set_chorus_send(255);
        effects.set_chorus_send_to_reverb(255);
        effects.set_chorus_feedback(255);
        effects.set_reverb_time(255);

        let mut buffer = vec![0.0f32; 2 * 44100];
        for (i, chunk) in buffer.chunks_exact_mut(2).enumerate() {
            if i % 4410 == 0 {
                chunk[0] = 1.0;
                chunk[1] = -1.0;
            }
        }
        effects.process(&mut buffer, 2);

        for &s in &buffer {
            assert!(s.is_finite() && s.abs() < 100.0, "発散している: {s}");
        }
    }
}
