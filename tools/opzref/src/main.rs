//! opzref — ymfm OPZ(YM2414) を参照レンダラとして使う検証用ツール。
//!
//! TX81Z の .syx ボイスを ymfm OPZ エミュで直接鳴らし WAV 化する。
//! opz2x6＋38x6 の変換忠実度を、実機録音の交絡なしに突き合わせるための参照を作る。
//!
//! 使い方:
//!   opzref --selftest [out.wav]
//!   opzref render <bank.syx> <voice_index> [out.wav]
//!                 [--note <midi>] [--dur <sec>] [--gate <sec>]
//!                 [--kc <hex>] [--slots a,b,c,d]
//!
//! ops[] は [OP4,OP3,OP2,OP1]。--slots は ops[0..3] を割り当てる
//! レジスタ slot (0..3)。既定 [0,2,1,3] は VMEM のバイト配置（OP4@0,OP2@10,OP3@20,OP1@30、
//! parse.rs参照）がそのまま物理slot順を反映しているという前提に基づく
//! （TX81Z公式ドキュメント・NOZ氏の解説・本ツールでの実測波形で妥当性を確認済み）。

use std::os::raw::{c_uint, c_void};
use std::path::Path;

use opz2x6::parse::{parse_syx, OpzOpData, OpzVoice};

// TX81Z 実機のマスタークロック (OPM系の標準, NTSC colorburst)
const OPZ_CLOCK: u32 = 3_579_545;

extern "C" {
    fn opzref_create() -> *mut c_void;
    fn opzref_destroy(h: *mut c_void);
    fn opzref_sample_rate(h: *mut c_void, clock: c_uint) -> c_uint;
    fn opzref_write_reg(h: *mut c_void, reg: c_uint, data: u8);
    fn opzref_generate(h: *mut c_void, out: *mut f32, n: c_uint);
}

struct Opz {
    h: *mut c_void,
}

impl Opz {
    fn new() -> Self {
        Opz { h: unsafe { opzref_create() } }
    }
    fn sample_rate(&self) -> u32 {
        unsafe { opzref_sample_rate(self.h, OPZ_CLOCK) }
    }
    fn write(&self, reg: u32, data: u8) {
        unsafe { opzref_write_reg(self.h, reg, data) }
    }
    fn render(&self, n: usize) -> Vec<f32> {
        let mut buf = vec![0.0f32; n];
        unsafe { opzref_generate(self.h, buf.as_mut_ptr(), n as c_uint) }
        buf
    }
}

impl Drop for Opz {
    fn drop(&mut self) {
        unsafe { opzref_destroy(self.h) }
    }
}

// channel 0 のオペレーター register: base + slot*8。
fn op_reg(base: u32, slot: u32) -> u32 {
    base + slot * 8
}

// TX81Z DET (0-6, 3=中心) → OPM DT1 register (0-7)。
const DT1_FROM_DET: [u8; 7] = [7, 6, 5, 0, 1, 2, 3];

// midi semitone(0=C) → OPM KC note nibble（3,7,11,15は未使用、標準のOPM/OPZ表）。
const NOTECODE: [u8; 12] = [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14];

// TX81Z FREQ coarse(0-63) → OPZ register (MUL 0-15, fine 0-15)。
// ratio(opz2x6::conv::coarse_to_ratio、実測テーブルに基づく。opz2x6::conv::freq_to_mul_fine の
// コメント参照)を MUL+fine/16 に符号化する（floor→MUL、端数×16→fine。ymfmの
// cache.multiple=(MUL<<4)|FINEと同じx.4固定小数点表現）。
fn freq_to_reg_mul_fine(freq: u8) -> (u8, u8) {
    let ratio = opz2x6::conv::coarse_to_ratio(freq);
    if ratio <= 0.75 {
        return (0, 0); // MUL=0 は ymfm で 0.5 扱い
    }
    let mul = (ratio.floor() as u8).clamp(1, 15);
    let fine = (((ratio - mul as f32) * 16.0).round() as i32).clamp(0, 15) as u8;
    (mul, fine)
}

// TX81Z OUT (0-99, 99=最大) → OPZ TL register (0-127, 0=最大)。線形近似。
fn out_to_tl_reg(out: u8) -> u8 {
    let out = out.min(99) as f32;
    ((99.0 - out) * 127.0 / 99.0).round().clamp(0.0, 127.0) as u8
}

fn midi_to_kc(midi: u8) -> u8 {
    let oct = (midi / 12).saturating_sub(1) & 0x07;
    let note = NOTECODE[(midi % 12) as usize];
    (oct << 4) | note
}

/// 1ボイスを ymfm OPZ に書き込み、note を gate 秒鳴らして dur 秒レンダリングする。
fn render_voice(
    opz: &Opz,
    v: &OpzVoice,
    kc: u8,
    slots: [u32; 4],
    sr: u32,
    gate_secs: f32,
    dur_secs: f32,
) -> Vec<f32> {
    // チャンネル: alg/fb, panR(bit7), keyoff
    let ch20 = 0x80 | ((v.feedback & 0x07) << 3) | (v.algorithm & 0x07);
    opz.write(0x20, ch20);
    // 音程
    opz.write(0x28, kc);
    opz.write(0x30, 0x00);
    // PMS/AMS
    opz.write(0x38, ((v.pms & 0x07) << 4) | (v.ams & 0x03));

    // 各オペレーター
    for (j, op) in v.ops.iter().enumerate() {
        let slot = slots[j];
        write_operator(opz, op, slot);
    }

    // キーオン: 0x20 bit6=1 (ch0 は regdata[0x08]=0 既定なので発火)
    opz.write(0x20, ch20 | 0x40);

    let gate_n = (sr as f32 * gate_secs) as usize;
    let total_n = (sr as f32 * dur_secs).max(gate_secs as f32) as usize;
    let mut buf = opz.render(gate_n);
    // キーオフ
    opz.write(0x20, ch20);
    if total_n > gate_n {
        buf.extend(opz.render(total_n - gate_n));
    }
    buf
}

fn write_operator(opz: &Opz, op: &OpzOpData, slot: u32) {
    let (mul, fine) = freq_to_reg_mul_fine(op.freq);
    let dt1 = DT1_FROM_DET[op.det.min(6) as usize];
    let tl = out_to_tl_reg(op.out);

    // 0x40: DT1(6-4) | MUL(3-0), data bit7=0
    opz.write(op_reg(0x40, slot), ((dt1 & 0x07) << 4) | (mul & 0x0f));
    // 0x60: TL(6-0)
    opz.write(op_reg(0x60, slot), tl & 0x7f);
    // 0x80: KSR(7-6) | FIX(5)=0 | AR(4-0)
    opz.write(op_reg(0x80, slot), ((op.rs & 0x03) << 6) | (op.ar & 0x1f));
    // 0xa0: AM(7) | D1R(4-0)
    opz.write(op_reg(0xa0, slot), ((op.ame as u8) << 7) | (op.d1r & 0x1f));
    // 0xc0: DT2(7-6)=0 | D2R(4-0), data bit5=0
    opz.write(op_reg(0xc0, slot), op.d2r & 0x1f);
    // 0xe0: D1L(7-4) | RR(3-0)
    opz.write(op_reg(0xe0, slot), ((op.d1l & 0x0f) << 4) | (op.rr & 0x0f));
    // 0x40 alt (data bit7=1): waveform(6-4) | fine(3-0)
    opz.write(op_reg(0x40, slot), 0x80 | ((op.ow & 0x07) << 4) | (fine & 0x0f));
}

fn normalize(buf: &mut [f32]) -> f32 {
    let peak = buf.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    if peak > 1e-9 {
        let g = 0.5 / peak;
        for x in buf.iter_mut() {
            *x *= g;
        }
    }
    peak
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("render <bank.syx> <voice_index> [out.wav] ...".into());
    }
    let bank = &args[0];
    let vidx: usize = args[1].parse().map_err(|_| "voice_index が不正")?;
    let mut out = format!("opzref_voice{vidx}.wav");
    let mut note: u8 = 68; // G#4
    let mut dur = 2.5f32;
    let mut gate = 2.0f32;
    let mut kc_override: Option<u8> = None;
    let mut slots: [u32; 4] = [0, 2, 1, 3];
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--note" => { note = args[i + 1].parse().map_err(|_| "--note 不正")?; i += 2; }
            "--dur" => { dur = args[i + 1].parse().map_err(|_| "--dur 不正")?; i += 2; }
            "--gate" => { gate = args[i + 1].parse().map_err(|_| "--gate 不正")?; i += 2; }
            "--kc" => {
                kc_override = Some(u8::from_str_radix(args[i + 1].trim_start_matches("0x"), 16).map_err(|_| "--kc 不正")?);
                i += 2;
            }
            "--slots" => {
                let parts: Vec<u32> = args[i + 1].split(',').map(|s| s.parse().unwrap_or(0)).collect();
                if parts.len() != 4 { return Err("--slots は4値".into()); }
                slots = [parts[0], parts[1], parts[2], parts[3]];
                i += 2;
            }
            other => { out = other.to_string(); i += 1; }
        }
    }

    let data = std::fs::read(bank).map_err(|e| format!("{bank}: {e}"))?;
    let voices = parse_syx(&data)?;
    let v = voices.get(vidx).ok_or_else(|| format!("voice {vidx} が無い(全{}件)", voices.len()))?;
    eprintln!("voice {vidx}: \"{}\" alg={} fb={}", v.name, v.algorithm, v.feedback);
    for (j, op) in v.ops.iter().enumerate() {
        let (mul, fine) = freq_to_reg_mul_fine(op.freq);
        eprintln!(
            "  ops[{j}]->slot{}: out={} freq={}(mul{}.f{}) ow={} ar={} d1l={} rr={}",
            slots[j], op.out, op.freq, mul, fine, op.ow, op.ar, op.d1l, op.rr
        );
    }

    let opz = Opz::new();
    let sr = opz.sample_rate();
    let kc = kc_override.unwrap_or_else(|| midi_to_kc(note));
    eprintln!("note={note} kc=0x{kc:02X} sr={sr} slots={slots:?}");

    let mut buf = render_voice(&opz, v, kc, slots, sr, gate, dur);
    let peak = normalize(&mut buf);
    eprintln!("peak(before norm)={peak:.6} samples={}", buf.len());

    write_wav_mono16(Path::new(&out), &buf, sr).map_err(|e| format!("WAV書き込み失敗: {e}"))?;
    println!("レンダリング: {out} ({:.2}秒, {sr} Hz)", buf.len() as f32 / sr as f32);
    Ok(())
}

fn selftest(out_path: &Path) -> Result<(), String> {
    let opz = Opz::new();
    let sr = opz.sample_rate();
    eprintln!("ymfm OPZ native sample_rate = {sr} Hz");
    opz.write(0x20, 0x80 | 0x07);
    opz.write(op_reg(0x40, 0), 0x01);
    opz.write(op_reg(0x60, 0), 0x00);
    opz.write(op_reg(0x80, 0), 0x1f);
    opz.write(op_reg(0xa0, 0), 0x00);
    opz.write(op_reg(0xc0, 0), 0x00);
    opz.write(op_reg(0xe0, 0), 0x08);
    for slot in 1..4 {
        opz.write(op_reg(0x60, slot), 0x7f);
    }
    opz.write(0x28, 0x4a);
    opz.write(0x30, 0x00);
    opz.write(0x20, 0x80 | 0x40 | 0x07);
    let n = (sr as f32 * 1.5) as usize;
    let mut buf = opz.render(n);
    normalize(&mut buf);
    write_wav_mono16(out_path, &buf, sr).map_err(|e| format!("WAV書き込み失敗: {e}"))?;
    println!("selftest: {} ({sr} Hz)", out_path.display());
    Ok(())
}

fn write_wav_mono16(path: &Path, samples: &[f32], sr: u32) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data_len = (samples.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&sr.to_le_bytes())?;
    f.write_all(&(sr * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        f.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let res = match args.first().map(|s| s.as_str()) {
        Some("--selftest") => {
            let out = args.get(1).cloned().unwrap_or_else(|| "opzref_selftest.wav".into());
            selftest(Path::new(&out))
        }
        Some("render") => cmd_render(&args[1..]),
        _ => Err("usage: opzref render <bank.syx> <voice> [out.wav] | --selftest".into()),
    };
    match res {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("opzref: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
