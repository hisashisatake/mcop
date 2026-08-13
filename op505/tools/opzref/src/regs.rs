//! ymfm OPZ(YM2414)へのレジスタ書き込みロジック。実機（C++ FFI経由の`Opz`）とテスト用
//! コレクタ（`Vec<(u32,u8)>`）の両方へ`RegSink`トレイトで二重化し、ゴールデンテストから
//! 音を鳴らさずにレジスタ計算だけを検証できるようにする。
//!
//! 由来: ym38x6/tools/opzref4x6/src/main.rs（コミット b61ba7a 時点の複製、2026-08-13）。
//! デフォーク後のop505ツール群向け複製（fork-on-write）。

use opz2op505::parse::{OpzOpData, OpzVoice};

/// レジスタ書き込み先の抽象化。実機（`Opz`、main.rs側）とテスト用コレクタで共有する。
pub trait RegSink {
    fn write(&mut self, reg: u32, data: u8);
}

impl RegSink for Vec<(u32, u8)> {
    fn write(&mut self, reg: u32, data: u8) {
        self.push((reg, data));
    }
}

/// channel 0 のオペレーター register: base + slot*8。
pub fn op_reg(base: u32, slot: u32) -> u32 {
    base + slot * 8
}

// TX81Z DET (0-6, 3=中心) → OPM DT1 register (0-7)。
pub const DT1_FROM_DET: [u8; 7] = [7, 6, 5, 0, 1, 2, 3];

// midi semitone(0=C) → OPM KC note nibble（3,7,11,15は未使用、標準のOPM/OPZ表）。
pub const NOTECODE: [u8; 12] = [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14];

/// TX81Z FREQ (coarse 0-63, fine 0-15) → OPZ register (MUL 0-15, reg_fine 0-15)。
/// ratio(opz2op505::map::coarse_fine_to_ratio、DXConvert実測テーブル)を MUL+reg_fine/16 に
/// 符号化する（floor→MUL、端数×16→reg_fine。ymfmのcache.multiple=(MUL<<4)|FINEと同じ
/// x.4固定小数点表現）。
pub fn freq_to_reg_mul_fine(freq: u8, fine: u8) -> (u8, u8) {
    let ratio = opz2op505::map::coarse_fine_to_ratio(freq, fine);
    if ratio <= 0.75 {
        return (0, 0); // MUL=0 は ymfm で 0.5 扱い
    }
    let mul = (ratio.floor() as u8).clamp(1, 15);
    let reg_fine = (((ratio - mul as f32) * 16.0).round() as i32).clamp(0, 15) as u8;
    (mul, reg_fine)
}

pub fn midi_to_kc(midi: u8) -> u8 {
    // ymfm実測で判明: KCのオクターブ境界はC/C#間ではなくC#/D間よりさらにずれており、
    // 「オクターブブロックはC#(N)〜C(N+1)の12音」という区切りになっている。
    // 引数を1つずらすことでこの境界のズレを吸収する（複製元main.rsのコメント参照）。
    let midi = midi.saturating_sub(1);
    let oct = (midi / 12).saturating_sub(1) & 0x07;
    let note = NOTECODE[(midi % 12) as usize];
    (oct << 4) | note
}

fn write_operator<S: RegSink>(sink: &mut S, op: &OpzOpData, slot: u32, alg_atten: u8, note: u8) {
    let (mul, fine) = freq_to_reg_mul_fine(op.freq, op.fine);
    let dt1 = DT1_FROM_DET[op.det.min(6) as usize];
    // TX81Z OUT (0-99, 99=最大) → OPZ TL register (0-127, 0=最大)。
    // 実機の非線形テーブル(opz2op505::map::ol_to_atten)にAalg（アルゴリズムによる追加減衰、
    // キャリアのみ）とAls（Level Scaling、crate::attls::attls_reg）を加算する。
    let ls_atten = crate::attls::attls_reg(op.ls, note);
    let tl = opz2op505::map::ol_to_atten(op.out)
        .saturating_add(alg_atten)
        .saturating_add(ls_atten)
        .min(127);

    // 0x40: DT1(6-4) | MUL(3-0), data bit7=0
    sink.write(op_reg(0x40, slot), ((dt1 & 0x07) << 4) | (mul & 0x0f));
    // 0x60: TL(6-0)
    sink.write(op_reg(0x60, slot), tl & 0x7f);
    // 0x80: KSR(7-6) | FIX(5)=0 | AR(4-0)
    sink.write(op_reg(0x80, slot), ((op.rs & 0x03) << 6) | (op.ar & 0x1f));
    // 0xa0: AM(7) | D1R(4-0)
    sink.write(op_reg(0xa0, slot), ((op.ame as u8) << 7) | (op.d1r & 0x1f));
    // 0xc0: DT2(7-6)=0 | D2R(4-0), data bit5=0
    sink.write(op_reg(0xc0, slot), op.d2r & 0x1f);
    // 0xc0 alt (data bit5=1): EG shift(7-6) | Reverb rate(2-0)=0。
    sink.write(op_reg(0xc0, slot), 0x20 | ((op.egsft & 0x03) << 6));
    // 0xe0: D1L(7-4) | RR(3-0)
    // sysexのD1Lはパネル極性(15=フルサステイン)、チップレジスタは減衰量極性(15=-93dB)なので
    // 15-panel で反転してから書く。
    sink.write(op_reg(0xe0, slot), ((15 - (op.d1l & 0x0f)) << 4) | (op.rr & 0x0f));
    // 0x40 alt (data bit7=1): waveform(6-4) | fine(3-0)
    sink.write(op_reg(0x40, slot), 0x80 | ((op.ow & 0x07) << 4) | (fine & 0x0f));
}

/// 1ボイス分のチャンネル設定＋4オペレーターをレジスタへ書き込む（キーオンは含まない）。
/// 戻り値は0x20レジスタの値（bit6=keyonを含まない）で、呼び出し側がkeyon/keyoffの
/// 切り替えに使う。
pub fn write_voice_setup<S: RegSink>(
    sink: &mut S,
    v: &OpzVoice,
    note: u8,
    kc: u8,
    slots: [u32; 4],
    force_sine: bool,
) -> u8 {
    // チャンネル: alg/fb, panR(bit7)
    let ch20 = 0x80 | ((v.feedback & 0x07) << 3) | (v.algorithm & 0x07);
    sink.write(0x20, ch20);
    // 音程
    sink.write(0x28, kc);
    sink.write(0x30, 0x00);
    // PMS/AMS
    sink.write(0x38, ((v.pms & 0x07) << 4) | (v.ams & 0x03));

    // Aalg（アルゴリズムによる追加減衰、opz2op505::map::alg_atten）はキャリアのみに乗る。
    let atten = opz2op505::map::alg_atten(v.algorithm);
    let carriers = opz2op505::map::CARRIERS[v.algorithm.min(7) as usize];
    for (j, op) in v.ops.iter().enumerate() {
        let slot = slots[j];
        let alg_atten = if carriers.contains(&j) { atten } else { 0 };
        if force_sine {
            let mut op = op.clone();
            op.ow = 0;
            write_operator(sink, &op, slot, alg_atten, note);
        } else {
            write_operator(sink, op, slot, alg_atten, note);
        }
    }
    ch20
}

#[cfg(test)]
mod tests {
    use super::*;
    use op505_tools::golden::{assert_golden, Fingerprint};
    use opz2op505::parse::OpzVoice;
    use std::path::PathBuf;

    const GOLDEN_VERSION: u32 = 1;

    fn golden_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
    }

    fn base_voice() -> OpzVoice {
        let mut v = OpzVoice::default();
        v.algorithm = 4;
        v.feedback = 5;
        v.pms = 3;
        v.ams = 1;
        for op in v.ops.iter_mut() {
            op.ar = 31;
            op.d1r = 10;
            op.d2r = 5;
            op.rr = 8;
            op.d1l = 12;
            op.out = 80;
            op.rs = 2;
            op.kvs = 4;
            op.freq = 1;
            op.det = 3;
        }
        v
    }

    #[test]
    fn op_argument_order_not_swapped() {
        // d1l=15(フルサステイン)・rr=0 の音色で、0xe0レジスタ上位nibbleが0（反転後15-15=0）に
        // なることを確認する（d1l/rrの引数取り違え検知）。
        let mut v = base_voice();
        v.ops[0].d1l = 15;
        v.ops[0].rr = 0;
        let mut regs: Vec<(u32, u8)> = Vec::new();
        write_voice_setup(&mut regs, &v, 60, 0x4a, [0, 2, 1, 3], false);
        let e0 = regs.iter().find(|&&(r, _)| r == op_reg(0xe0, 0)).unwrap().1;
        assert_eq!(e0 & 0xf0, 0x00, "d1l=15(フルサステイン)は上位nibble=0になるはず");
        assert_eq!(e0 & 0x0f, 0x00, "rr=0はそのまま下位nibbleに入るはず");
    }

    #[test]
    fn midi_to_kc_known_values() {
        // selftest実測で確定した境界値（main.rsのコメント参照）。
        assert_eq!(midi_to_kc(69), 0x4a); // A4
    }

    #[test]
    fn freq_to_reg_mul_fine_low_ratio_clamps_to_zero() {
        assert_eq!(freq_to_reg_mul_fine(0, 0), (0, 0));
    }

    #[test]
    fn reg_sweep_golden() {
        let mut fp = Fingerprint::new();
        for &alg in &[0u8, 3, 4, 7] {
            for &fb in &[0u8, 7] {
                for &note in &[24u8, 60, 108, 127] {
                    let mut v = base_voice();
                    v.algorithm = alg;
                    v.feedback = fb;
                    for (i, op) in v.ops.iter_mut().enumerate() {
                        op.ar = (i as u8 * 7) % 32;
                        op.d1r = (i as u8 * 5) % 32;
                        op.d2r = (i as u8 * 3) % 32;
                        op.rr = (i as u8 * 2) % 16;
                        op.d1l = (i as u8 * 4) % 16;
                        op.out = 20 + (i as u8 * 15);
                        op.ls = 50;
                        op.ow = i as u8 % 8;
                        op.fine = i as u8 % 16;
                        op.egsft = i as u8 % 4;
                    }
                    let kc = midi_to_kc(note);
                    let mut regs: Vec<(u32, u8)> = Vec::new();
                    write_voice_setup(&mut regs, &v, note, kc, [0, 2, 1, 3], false);
                    fp.push(&regs);
                }
            }
        }
        assert_golden(&golden_path("reg_sweep.fnv"), &fp.finish(GOLDEN_VERSION));
    }
}
