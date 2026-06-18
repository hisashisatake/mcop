//! OPM（YM2151）レジスタ状態マシンと MIDI 音程変換。

use opm2x6::parse::{OpmOpReg, OpmVoice};

// ---------------------------------------------------------------------------
// OPM レジスタ状態
// ---------------------------------------------------------------------------

pub struct OpmState {
    pub regs: [u8; 256],
}

impl OpmState {
    pub fn new() -> Self {
        Self { regs: [0u8; 256] }
    }

    pub fn write(&mut self, reg: u8, val: u8) {
        self.regs[reg as usize] = val;
    }

    pub fn kc(&self, ch: usize) -> u8 {
        self.regs[0x28 + ch]
    }

    pub fn kf(&self, ch: usize) -> u8 {
        self.regs[0x30 + ch]
    }

    /// 8チャンネル分の OpmVoice スナップショットを生成する。
    /// `slot_byte` は 0x08（キーオン）レジスタ書き込み値の bits[6:3]（スロットマスク）。
    pub fn build_voice(&self, ch: usize, slot_byte: u8) -> OpmVoice {
        let rl_fb_con = self.regs[0x20 + ch];
        let pms_ams   = self.regs[0x38 + ch];

        OpmVoice {
            number: ch as u32,
            name: format!("ch{ch}"),
            lfrq: self.regs[0x00],
            // OPM: reg[0x18]=PMD, reg[0x19]=AMD（7ビット、上位ビット無視）
            pmd: self.regs[0x18] & 0x7F,
            amd: self.regs[0x19] & 0x7F,
            // reg[0x01] bits[1:0] = LFO 波形
            lfo_wf: self.regs[0x01] & 0x03,
            fl:  (rl_fb_con >> 3) & 0x07,
            con: rl_fb_con & 0x07,
            pms: (pms_ams >> 4) & 0x07,
            ams: pms_ams & 0x03,
            // キーオンレジスタのスロットビットをそのまま使う
            // bits[6:3] = C2/M2/C1/M1（opm2x6 の slot_muted() と同一レイアウト）
            slot: slot_byte & 0x78,
            // OPM レジスタ順: op_reg_idx 0=M1, 1=M2, 2=C1, 3=C2
            m1: self.build_op_reg(ch, 0),
            m2: self.build_op_reg(ch, 1),
            c1: self.build_op_reg(ch, 2),
            c2: self.build_op_reg(ch, 3),
        }
    }

    /// op_reg_idx = 0(M1) / 1(M2) / 2(C1) / 3(C2)
    fn build_op_reg(&self, ch: usize, op: usize) -> OpmOpReg {
        let i = op * 8 + ch;
        let dt1_mul = self.regs[0x40 + i];
        let tl      = self.regs[0x60 + i];
        let ks_ar   = self.regs[0x80 + i];
        let ame_d1r = self.regs[0xA0 + i];
        let dt2_d2r = self.regs[0xC0 + i];
        let d1l_rr  = self.regs[0xE0 + i];

        OpmOpReg {
            mul:    dt1_mul & 0x0F,
            dt1:    (dt1_mul >> 4) & 0x07,
            tl:     tl & 0x7F,
            ks:     (ks_ar >> 6) & 0x03,
            ar:     ks_ar & 0x1F,
            ams_en: (ame_d1r >> 7) != 0,
            d1r:    ame_d1r & 0x1F,
            dt2:    (dt2_d2r >> 6) & 0x03,
            d2r:    dt2_d2r & 0x1F,
            d1l:    (d1l_rr >> 4) & 0x0F,
            rr:     d1l_rr & 0x0F,
        }
    }
}

// ---------------------------------------------------------------------------
// KC → MIDI ノート番号変換
// ---------------------------------------------------------------------------

/// OPM KC レジスタ値 → MIDI ノート番号（0-127）。
///
/// KC[6:4] = オクターブ（0-7）、KC[3:0] = ノートコード。
///
/// 検証:
/// - octave=4, code=11（A）→ 4×12 + 9 + 12 = 69 = A4 ✓
/// - octave=3, code=14（C）→ 3×12 + 12 + 12 = 60 = C4（中央 C）✓
pub fn kc_to_midi_note(kc: u8) -> u8 {
    // ノートコード → C からの半音数
    // code 14/15 は次オクターブの C（= 12 半音）として扱う
    const NOTE_SEMITONES: [u8; 16] = [1, 1, 2, 2, 3, 4, 5, 5, 6, 7, 8, 9, 10, 11, 12, 12];
    let octave = (kc >> 4) & 0x07;
    let note   = (kc & 0x0F) as usize;
    let midi   = octave as u16 * 12 + NOTE_SEMITONES[note] as u16 + 12;
    midi.min(127) as u8
}

// ---------------------------------------------------------------------------
// KF → MIDI ピッチベンド
// ---------------------------------------------------------------------------

/// OPM KF レジスタ値 → MIDI ピッチベンド値（center=8192, ±2半音デフォルト範囲）。
///
/// KF は KC ノートより何 1/64 半音高いかを示す（常に正方向）。
/// MIDI デフォルト範囲 ±2半音: KF_6bit × 64 でベンド量を計算。
///
/// KF=0  → 8192（ベンドなし）
/// KF=63 → 8192 + 63×64 = 12224（≈ 0.98半音上）
pub fn kf_to_pitch_bend(kf: u8) -> i16 {
    let kf_6bit = (kf >> 2) & 0x3F;
    (8192u16 + kf_6bit as u16 * 64) as i16
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kc_a4_is_midi_69() {
        // A4 = octave 4, code 11 (A)
        assert_eq!(kc_to_midi_note(0x4B), 69);
    }

    #[test]
    fn kc_middle_c_is_midi_60() {
        // C4 = octave 3, code 14 (C of next octave boundary)
        assert_eq!(kc_to_midi_note(0x3E), 60);
    }

    #[test]
    fn kc_c_sharp4_is_midi_61() {
        // C#4 = octave 4, code 0
        assert_eq!(kc_to_midi_note(0x40), 61);
    }

    #[test]
    fn kf_zero_is_center() {
        assert_eq!(kf_to_pitch_bend(0), 8192);
    }

    #[test]
    fn kf_max_is_below_one_semitone() {
        // KF=0xFC (最大: bits[7:2]=63) → 8192 + 63*64 = 12224
        assert_eq!(kf_to_pitch_bend(0xFC), 12224);
        // 12224 < 16383 (MIDI最大) ✓
        assert!(kf_to_pitch_bend(0xFC) <= 16383);
    }
}
