//! OPM（YM2151）レジスタ状態マシンと MIDI 音程変換。
//!
//! 由来: ym38x6/tools/vgm2x6/src/opm.rs（コミット535604f時点の複製、2026-08-11）。
//! ym38x6依存排除（デフォーク）に伴う複製。以後は独立に進化させる（fork-on-write）。
//! vgm2x6側の修正は自動では反映されない（`git diff 535604f -- ym38x6/tools/vgm2x6/src/opm.rs`
//! で追従漏れを確認できる）。唯一の変更点は`use`先を`opm2x6::parse`から`opm2op505::parse`
//! （フィールド構成が同一の独立コピー）へ差し替えたことのみ。

use opm2op505::parse::{OpmOpReg, OpmVoice};

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
    // ノートコード → C からの半音数（C#=1 起点）。
    // 実機YM2151は「3半音×4グループ」構造：下位2bitが 0/1/2 で半音、3 は欠番。
    //   code 0,1,2   = C#,D,D#（グループ0）
    //   code 4,5,6   = E,F,F#  （グループ1）
    //   code 8,9,10  = G,G#,A  （グループ2）
    //   code 12,13,14= A#,B,C  （グループ3）
    // 欠番コード(3,7,11,15)は直前の有効コードへclampする。
    const NOTE_SEMITONES: [u8; 16] = [1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 9, 9, 10, 11, 12, 12];
    let octave = (kc >> 4) & 0x07;
    let note   = (kc & 0x0F) as usize;
    let midi   = octave as u16 * 12 + NOTE_SEMITONES[note] as u16 + 12;
    midi.min(127) as u8
}

// ---------------------------------------------------------------------------
// KC + KF → MIDI ピッチベンド（±12半音感度）
// ---------------------------------------------------------------------------

/// ピッチベンド感度（半音数）。RPN 0 でDAW側にも同じ値を設定する。
pub const PB_SENSITIVITY: u8 = 12;

/// KC/KF の現在値と基準KC から MIDI ピッチベンド値を計算する。
///
/// OPM KC は「オクターブ(3bit) + ノートコード(4bit)」で、バイト差は非線形
/// （1オクターブ = KC差16 ≠ 12半音、C#/D/F のコード重複あり）。
/// kc_to_midi_note() で実際の半音差に変換してから計算する。
///
/// 戻り値: ピッチベンド値（0-16383）と、範囲外かどうかのフラグ。
/// 範囲外（|delta| > 感度）の場合は呼び出し元で Note Off + On を行う。
pub fn compute_pitch_bend(current_kc: u8, base_kc: u8, kf: u8) -> (i16, bool) {
    let semitone_delta =
        kc_to_midi_note(current_kc) as i16 - kc_to_midi_note(base_kc) as i16;
    let kf_semitones = ((kf >> 2) & 0x3F) as f32 / 64.0;
    let delta = semitone_delta as f32 + kf_semitones;
    let out_of_range = delta.abs() > PB_SENSITIVITY as f32;
    let bend = (8192.0 + delta / PB_SENSITIVITY as f32 * 8192.0)
        .round()
        .clamp(0.0, 16383.0) as i16;
    (bend, out_of_range)
}

// ---------------------------------------------------------------------------
// 診断用：実機YM2151 canonical変換（--dump-pitch）
// ---------------------------------------------------------------------------

/// 実機YM2151 KC/KF → 浮動小数MIDIノート番号（真値リファレンス）。
///
/// 実機OPMのノートコード（KC下位4bit）は下位2bitが `11`（=3,7,11,15）が欠番で、
/// 残り12個が半音ずつに対応する。canonicalな半音インデックスは
/// `index = (code>>2)*3 + (code&3)`（code&3==3 は欠番のため 2 にclamp）。
/// オクターブ（KC[6:4]）と KF（6bit, 1/64半音）を合わせて絶対ピッチを求める。
///
/// アンカー: A4=440Hz は KC=0x4A（octave4, code10→index8）→ MIDI 69。
/// `midi = octave*12 + index + 13 + kf_6bit/64`
pub fn kc_kf_to_ref_midi(kc: u8, kf: u8) -> f32 {
    let octave = ((kc >> 4) & 0x07) as f32;
    let code = kc & 0x0F;
    let lo = (code & 0x03).min(2);
    let index = ((code >> 2) * 3 + lo) as f32;
    let kf_frac = ((kf >> 2) & 0x3F) as f32 / 64.0;
    octave * 12.0 + index + 13.0 + kf_frac
}

/// SMFピッチベンド値（0-16383, center=8192）→ 半音数。
/// エンジン/VST側の解釈 `(value/16383 - 0.5) * 2 * PB_SENSITIVITY` に合わせる。
pub fn pb_to_semitones(pb: i16) -> f32 {
    (pb as f32 / 16383.0 - 0.5) * 2.0 * PB_SENSITIVITY as f32
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
    fn pb_no_delta_no_kf_is_center() {
        let (pb, oor) = compute_pitch_bend(0x4B, 0x4B, 0);
        assert_eq!(pb, 8192);
        assert!(!oor);
    }

    #[test]
    fn pb_kf_max_adds_fraction() {
        // KF=0xFC（最大63/64半音）、KC変化なし → センターより少し上
        let (pb, oor) = compute_pitch_bend(0x4B, 0x4B, 0xFC);
        assert!(pb > 8192);
        assert!(!oor);
    }

    #[test]
    fn pb_one_semitone_up() {
        // KC+1（1半音上）、感度12半音 → 8192 + 8192/12 ≈ 8875
        let (pb, oor) = compute_pitch_bend(0x4C, 0x4B, 0);
        assert!((pb as i32 - 8875).abs() <= 2);
        assert!(!oor);
    }

    #[test]
    fn ref_midi_anchors() {
        // canonical実機表のアンカー検証
        assert!((kc_kf_to_ref_midi(0x4A, 0) - 69.0).abs() < 1e-4); // A4 = 0x4A（code10）
        assert!((kc_kf_to_ref_midi(0x3E, 0) - 60.0).abs() < 1e-4); // C4 = 0x3E（code14）
        assert!((kc_kf_to_ref_midi(0x40, 0) - 61.0).abs() < 1e-4); // C#4 = 0x40（code0）
    }

    #[test]
    fn pipeline_matches_ref_on_all_valid_codes() {
        // ノートテーブル修正後、pipeline(kc_to_midi_note) と canonical実機表が
        // 全有効ノートコードで一致することを検証する（--dump-pitchのerror_cents=0に対応）。
        // 欠番コード(3,7,11,15)は除く。
        for code in [0u8, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14] {
            let kc = 0x40 | code; // octave 4
            let repro = kc_to_midi_note(kc) as f32;
            let refm = kc_kf_to_ref_midi(kc, 0);
            assert!(
                (repro - refm).abs() < 1e-4,
                "code {code}: pipeline={repro}, ref={refm}"
            );
        }
    }

    #[test]
    fn pb_out_of_range_over_sensitivity() {
        // 0x4B(A4=69) → 0x5C(A#5=82): 実際の半音差=13 > PB_SENSITIVITY=12 → out_of_range=true
        // ※ 旧実装の 0x4B+13=0x58 は KC差13でも実際の半音差が9にとどまるため使えない
        let (_pb, oor) = compute_pitch_bend(0x5C, 0x4B, 0);
        assert!(oor);
    }
}
