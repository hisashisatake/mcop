//! OPN系（YM2612/YM2203/YM2608）FMレジスタ状態マシンと音程変換。
//!
//! OPM（[crate::opm]）と異なり、OPNは2ポート構成（6ch機はport0=ch0-2 / port1=ch3-5）で、
//! 音程は KC/KF ノートコードではなく F-Number(11bit) + Block(3bit) で表す。
//!
//! 由来: ym38x6/tools/vgm2x6/src/opn.rs（コミット535604f時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! vgm2x6側の修正は自動では反映されない（`git diff 535604f -- ym38x6/tools/vgm2x6/src/opn.rs`
//! で追従漏れを確認できる）。唯一の変更点は`use`先を`mucom2x6::conv`から`mucom2op505::map`
//! （フィールド構成が同一の独立コピー）へ差し替えたことのみ（音色変換の`to_ym38x6_patch()`
//! 呼び出しは行わない＝ここはレジスタ→OpnVoiceのスナップショット構築のみ）。

use mucom2op505::map::{OpnOperator, OpnVoice};

// ---------------------------------------------------------------------------
// F-Number→周波数式の分母 factor（= FMプリスケーラ起因の分周係数）
// ---------------------------------------------------------------------------
//
// freq = fnum * chip_clock / (FM_DIVISOR * 2^(20-block))
//
// 分母係数は「3ch系(YM2203/OPN)」と「6ch系(YM2608/OPNA・YM2612/OPN2)」で2倍異なる。
// 6ch系はFMプリスケーラが3ch系の2倍（同一fnum/blockに対しチップクロックも約2倍で運用される）。
// この差を分母に反映しないと6ch系チップが「ちょうど1オクターブ高く」変換される。
//
// アンカー: A4=440Hz は fnum≈1038/block4 で
//   - YM2203 4MHz: 1038*4e6/(144*2^16) ≈ 440  （実機検証済み: Ys PC-88曲をA=440基準で確認）
//   - YM2608 8MHz: 1038*8e6/(288*2^16) ≈ 440  （同じ fnum/block・クロック2倍を288で吸収）
pub const FM_DIVISOR_3CH: u32 = 144;
pub const FM_DIVISOR_6CH: u32 = 288;

// ---------------------------------------------------------------------------
// OPN レジスタ状態
// ---------------------------------------------------------------------------

/// OPN系のレジスタ状態。port0/port1 の2面を保持する（YM2203は port0 のみ使用）。
pub struct OpnState {
    pub regs: [[u8; 256]; 2],
}

impl OpnState {
    pub fn new() -> Self {
        Self { regs: [[0u8; 256]; 2] }
    }

    pub fn write(&mut self, port: usize, reg: u8, val: u8) {
        self.regs[port & 1][reg as usize] = val;
    }

    /// チャンネル番号(0-5) → (port, ポート内ch 0-2)。
    #[inline]
    fn split_ch(ch: usize) -> (usize, usize) {
        if ch < 3 { (0, ch) } else { (1, ch - 3) }
    }

    /// 1オペレーター分のレジスタを読み出す。
    /// `slot` はレジスタ上のスロット番号（0,1,2,3 = +0,+4,+8,+12）。
    fn read_op(&self, port: usize, cip: usize, slot: usize) -> OpnOperator {
        let i = slot * 4 + cip;
        let dt_mul = self.regs[port][0x30 + i];
        let tl     = self.regs[port][0x40 + i];
        let ks_ar  = self.regs[port][0x50 + i];
        let am_d1r = self.regs[port][0x60 + i];
        let d2r    = self.regs[port][0x70 + i];
        let d1l_rr = self.regs[port][0x80 + i];

        OpnOperator {
            tl:        tl & 0x7F,
            ar:        ks_ar & 0x1F,
            d1r:       am_d1r & 0x1F,
            d2r:       d2r & 0x1F,
            d1l:       (d1l_rr >> 4) & 0x0F,
            rr:        d1l_rr & 0x0F,
            mul:       dt_mul & 0x0F,
            dt1:       (dt_mul >> 4) & 0x07,
            ks:        (ks_ar >> 6) & 0x03,
            am_enable: (am_d1r >> 7) != 0,
        }
    }

    /// チャンネル(0-5)の OpnVoice スナップショットを生成する。
    /// レジスタスロット順[0,1,2,3]→アルゴリズム順[OP1,OP2,OP3,OP4]へ並べ替える
    /// （= reg[0,2,1,3]。mucom88 の並べ替えと同一）。
    pub fn build_voice(&self, ch: usize) -> OpnVoice {
        let (port, cip) = Self::split_ch(ch);
        let reg_ops = [
            self.read_op(port, cip, 0),
            self.read_op(port, cip, 1),
            self.read_op(port, cip, 2),
            self.read_op(port, cip, 3),
        ];
        let operators = [reg_ops[0], reg_ops[2], reg_ops[1], reg_ops[3]];
        let fb_alg = self.regs[port][0xB0 + cip];
        OpnVoice {
            operators,
            algorithm: fb_alg & 0x07,
            feedback: (fb_alg >> 3) & 0x07,
        }
    }

    /// チャンネル(0-5)の現在の F-Number(11bit) と Block(3bit) を返す。
    pub fn fnum_block(&self, ch: usize) -> (u16, u8) {
        let (port, cip) = Self::split_ch(ch);
        let lo = self.regs[port][0xA0 + cip] as u16;
        let hi = self.regs[port][0xA4 + cip] as u16;
        let fnum = ((hi & 0x07) << 8) | lo;
        let block = ((hi >> 3) & 0x07) as u8;
        (fnum, block)
    }
}

// ---------------------------------------------------------------------------
// キーオン/オフ レジスタ(0x28) のデコード
// ---------------------------------------------------------------------------

/// キーオンレジスタ(0x28)の値 → (チャンネル0-5, スロットマスク bits7-4)。
/// チャンネルエンコード: low3bit が 0/1/2 → ch0-2(port0)、4/5/6 → ch3-5(port1)。
/// 3/7 は無効。
pub fn decode_keyon(val: u8) -> Option<(usize, u8)> {
    let slots = (val >> 4) & 0x0F;
    let ch = match val & 0x07 {
        0 => 0,
        1 => 1,
        2 => 2,
        4 => 3,
        5 => 4,
        6 => 5,
        _ => return None,
    };
    Some((ch, slots))
}

// ---------------------------------------------------------------------------
// 音程変換
// ---------------------------------------------------------------------------

/// F-Number + Block + チップクロック → 周波数(Hz)。
///
/// `freq = fnum * chip_clock / (fm_divisor * 2^(20-block))`
pub fn fnum_block_to_freq(fnum: u16, block: u8, chip_clock: u32, fm_divisor: u32) -> f32 {
    if fnum == 0 || chip_clock == 0 {
        return 0.0;
    }
    let denom = fm_divisor as f64 * (1u64 << (20 - block.min(7) as u32)) as f64;
    (fnum as f64 * chip_clock as f64 / denom) as f32
}

/// 周波数(Hz) → 浮動小数MIDIノート番号（A4=440Hz=69）。
pub fn freq_to_midi(freq: f32) -> f32 {
    if freq <= 0.0 {
        return 0.0;
    }
    69.0 + 12.0 * (freq / 440.0).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_ch_maps_ports() {
        assert_eq!(OpnState::split_ch(0), (0, 0));
        assert_eq!(OpnState::split_ch(2), (0, 2));
        assert_eq!(OpnState::split_ch(3), (1, 0));
        assert_eq!(OpnState::split_ch(5), (1, 2));
    }

    #[test]
    fn keyon_channel_decode() {
        assert_eq!(decode_keyon(0xF0), Some((0, 0x0F))); // 全op, ch0
        assert_eq!(decode_keyon(0x16), Some((5, 0x01))); // op1のみ, ch5
        assert_eq!(decode_keyon(0xF2), Some((2, 0x0F)));
        assert_eq!(decode_keyon(0x03), None); // 無効ch
        assert_eq!(decode_keyon(0x07), None);
    }

    #[test]
    fn operator_reorder_slot_to_algo() {
        // レジスタスロット 0/1/2/3 に異なるTLを書き、algo順[0,2,1,3]へ並ぶことを確認。
        let mut s = OpnState::new();
        // port0 ch0: TL レジスタ 0x40 + slot*4 + 0
        s.write(0, 0x40 + 0, 10); // slot0 → OP1(algo idx0)
        s.write(0, 0x40 + 4, 30); // slot1 → OP3(algo idx2)
        s.write(0, 0x40 + 8, 20); // slot2 → OP2(algo idx1)
        s.write(0, 0x40 + 12, 40); // slot3 → OP4(algo idx3)
        let v = s.build_voice(0);
        assert_eq!(v.operators[0].tl, 10);
        assert_eq!(v.operators[1].tl, 20);
        assert_eq!(v.operators[2].tl, 30);
        assert_eq!(v.operators[3].tl, 40);
    }

    #[test]
    fn fb_algo_decode() {
        let mut s = OpnState::new();
        s.write(0, 0xB0, (6 << 3) | 4); // FB=6, ALG=4
        let v = s.build_voice(0);
        assert_eq!(v.algorithm, 4);
        assert_eq!(v.feedback, 6);
    }

    #[test]
    fn freq_to_midi_anchor() {
        assert!((freq_to_midi(440.0) - 69.0).abs() < 1e-4);
        assert!((freq_to_midi(880.0) - 81.0).abs() < 1e-4);
    }

    #[test]
    fn fnum_block_freq_is_positive_and_octave_doubles() {
        // 同じfnumでblockが1増えると周波数は約2倍。
        let f_b4 = fnum_block_to_freq(600, 4, 7_670_454, FM_DIVISOR_6CH);
        let f_b5 = fnum_block_to_freq(600, 5, 7_670_454, FM_DIVISOR_6CH);
        assert!(f_b4 > 0.0);
        assert!((f_b5 / f_b4 - 2.0).abs() < 1e-3);
    }
}
