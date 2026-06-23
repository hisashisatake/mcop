//! YM2203/YM2608 内蔵 SSG(PSG, AY-3-8910互換) のレジスタ状態と PSG 変換。
//!
//! SSG は OPN の port0 レジスタ `0x00-0x0F` に存在する。トーン3ch（A/B/C）を
//! 矩形波音色（[crate] の `psg_patch()`）で鳴らし、音量レジスタ(0x08-0x0A)の
//! ソフトエンベロープは SMF では CC11、WAV ではキャリアTLへ反映する。

/// SSGクロックの分周値（`ssg_clock = chip_clock / SSG_CLOCK_DIVISOR`）。
/// YM2203/YM2608内蔵SSGはマスタークロックを/2した周波数でAY-3-8910互換回路を駆動する。
/// トーン周波数 `f = ssg_clock / (16 * period)` の絶対音程に効く。
pub const SSG_CLOCK_DIVISOR: u32 = 2;

/// SSG レジスタ状態（0x00-0x0F）。
pub struct SsgState {
    pub regs: [u8; 16],
}

impl SsgState {
    pub fn new() -> Self {
        Self { regs: [0u8; 16] }
    }

    pub fn write(&mut self, reg: u8, val: u8) {
        let r = reg as usize;
        if r < 16 {
            self.regs[r] = val;
        }
    }

    /// トーンch(0=A/1=B/2=C)の周期（12bit）。
    pub fn tone_period(&self, ch: usize) -> u16 {
        let lo = self.regs[ch * 2] as u16;
        let hi = self.regs[ch * 2 + 1] as u16;
        ((hi & 0x0F) << 8) | lo
    }

    /// トーンch がミキサー(0x07)で有効か（bit 0-2 が0=有効）。
    pub fn tone_enabled(&self, ch: usize) -> bool {
        (self.regs[0x07] >> ch) & 1 == 0
    }

    /// ノイズch がミキサー(0x07)で有効か（bit 3-5 が0=有効）。
    pub fn noise_enabled(&self, ch: usize) -> bool {
        (self.regs[0x07] >> (ch + 3)) & 1 == 0
    }

    /// ノイズ周期（5bit、reg 0x06 bits[4:0]）。0のとき実質1と同等。
    pub fn noise_period(&self) -> u8 {
        self.regs[0x06] & 0x1F
    }

    /// 音量(0-15)。レジスタ 0x08+ch の bits0-3。
    pub fn volume(&self, ch: usize) -> u8 {
        self.regs[0x08 + ch] & 0x0F
    }

    /// ハードウェアエンベロープモード（0x08+ch の bit4）。
    pub fn envelope_mode(&self, ch: usize) -> bool {
        (self.regs[0x08 + ch] >> 4) & 1 != 0
    }

    /// 発音上の実効音量(0-15)。エンベロープモード時は最大(15)とみなす（v1簡易対応）。
    pub fn effective_volume(&self, ch: usize) -> u8 {
        if self.envelope_mode(ch) {
            15
        } else {
            self.volume(ch)
        }
    }
}

/// SSG周期 → 周波数(Hz)。`f = ssg_clock / (16 * period)`。
pub fn period_to_freq(period: u16, ssg_clock: u32) -> f32 {
    if period == 0 {
        return 0.0;
    }
    ssg_clock as f32 / (16.0 * period as f32)
}

/// 実効音量(0-15) → CC11(エクスプレッション, 0-127) 線形マップ。
pub fn volume_to_cc11(vol: u8) -> u8 {
    (vol.min(15) as u16 * 127 / 15) as u8
}

/// 実効音量(0-15) → 矩形波キャリアOP1のTL(0-255) 線形マップ（WAV直描画用）。
pub fn volume_to_tl(vol: u8) -> u8 {
    (vol.min(15) as u16 * 255 / 15) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_period_combines_lo_hi() {
        let mut s = SsgState::new();
        s.write(0x00, 0x34); // A lo
        s.write(0x01, 0x02); // A hi (4bit)
        assert_eq!(s.tone_period(0), 0x234);
    }

    #[test]
    fn mixer_enable_is_active_low() {
        let mut s = SsgState::new();
        s.write(0x07, 0b111_110); // ch0 tone有効(bit0=0)、ch1/2無効
        assert!(s.tone_enabled(0));
        assert!(!s.tone_enabled(1));
        assert!(!s.tone_enabled(2));
    }

    #[test]
    fn noise_enable_bits_3_to_5() {
        let mut s = SsgState::new();
        // bits0-2=111: tone全無効 / bit3=0: noise ch0有効 / bit4=1, bit5=1: noise ch1,2無効
        s.write(0x07, 0b00_110_111); // = 0x37
        assert!(!s.tone_enabled(0));
        assert!(!s.tone_enabled(1));
        assert!(!s.tone_enabled(2));
        assert!(s.noise_enabled(0));
        assert!(!s.noise_enabled(1));
        assert!(!s.noise_enabled(2));
    }

    #[test]
    fn noise_period_is_5bit() {
        let mut s = SsgState::new();
        s.write(0x06, 0xFF); // 上位3bitは無効、下位5bit=31
        assert_eq!(s.noise_period(), 31);
        s.write(0x06, 0x00);
        assert_eq!(s.noise_period(), 0);
    }

    #[test]
    fn volume_and_envelope_mode() {
        let mut s = SsgState::new();
        s.write(0x08, 0x0A); // vol=10, env off
        assert_eq!(s.volume(0), 10);
        assert!(!s.envelope_mode(0));
        assert_eq!(s.effective_volume(0), 10);
        s.write(0x09, 0x1F); // bit4=env mode
        assert!(s.envelope_mode(1));
        assert_eq!(s.effective_volume(1), 15);
    }

    #[test]
    fn period_freq_octave_relation() {
        let f = period_to_freq(284, 2_000_000); // ≈440Hz @ ssg_clock 2MHz
        assert!((f - 440.0).abs() < 5.0);
        // 周期半分で1オクターブ上
        let f2 = period_to_freq(142, 2_000_000);
        assert!((f2 / f - 2.0).abs() < 0.05);
    }

    #[test]
    fn volume_maps() {
        assert_eq!(volume_to_cc11(15), 127);
        assert_eq!(volume_to_cc11(0), 0);
        assert_eq!(volume_to_tl(15), 255);
        assert_eq!(volume_to_tl(0), 0);
    }
}
