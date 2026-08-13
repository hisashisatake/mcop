//! opzref — ymfm OPZ(YM2414) を参照レンダラとして使う検証用ツール（op505向け）。
//!
//! TX81Z の .syx ボイスを ymfm OPZ エミュで直接鳴らし WAV 化する。
//! opz2op505 の変換忠実度を、実機録音の交絡なしに突き合わせるための参照を作る。
//!
//! 使い方:
//!   opzref --selftest [out.wav] [kc(hex)]
//!   opzref render <bank.syx> <voice_index> [out.wav]
//!                 [--note <midi>] [--dur <sec>] [--gate <sec>]
//!                 [--kc <hex>] [--slots a,b,c,d] [--force-sine]
//!
//! --force-sine は全オペレーターの波形(ow)を強制的に正弦波(0)にする診断用オプション。
//! ymfm OPZの非sine波形テーブルの再現精度を疑うときの切り分けに使う。
//!
//! ops[] は [OP4,OP3,OP2,OP1]。--slots は ops[0..3] を割り当てる
//! レジスタ slot (0..3)。既定 [0,2,1,3] は VMEM のバイト配置（OP4@0,OP2@10,OP3@20,OP1@30、
//! opz2op505::parse参照）がそのまま物理slot順を反映しているという前提に基づく
//! （TX81Z公式ドキュメント・NOZ氏の解説・本ツールでの実測波形で妥当性を確認済み）。
//!
//! 由来: ym38x6/tools/opzref4x6/src/main.rs（コミット b61ba7a 時点の複製、2026-08-13）。
//! デフォーク後のop505ツール群向け複製（fork-on-write）。
//! ym38x6/tools/opzref4x6側の修正は自動では反映されない
//! （`git diff b61ba7a -- ym38x6/tools/opzref4x6/src/main.rs` で追従漏れを確認できる）。

use std::os::raw::{c_uint, c_void};
use std::path::Path;

use opz2op505::parse::{parse_syx, OpzVoice};
use opzref::regs::{midi_to_kc, write_voice_setup, RegSink};

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
    fn render(&self, n: usize) -> Vec<f32> {
        let mut buf = vec![0.0f32; n];
        unsafe { opzref_generate(self.h, buf.as_mut_ptr(), n as c_uint) }
        buf
    }
}

impl RegSink for Opz {
    fn write(&mut self, reg: u32, data: u8) {
        unsafe { opzref_write_reg(self.h, reg, data) }
    }
}

impl Drop for Opz {
    fn drop(&mut self) {
        unsafe { opzref_destroy(self.h) }
    }
}

/// 1ボイスを ymfm OPZ に書き込み、note を gate 秒鳴らして dur 秒レンダリングする。
fn render_voice(
    opz: &mut Opz,
    v: &OpzVoice,
    note: u8,
    kc: u8,
    slots: [u32; 4],
    sr: u32,
    gate_secs: f32,
    dur_secs: f32,
    force_sine: bool,
) -> Vec<f32> {
    let ch20 = write_voice_setup(opz, v, note, kc, slots, force_sine);

    // キーオン: 0x20 bit6=1
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
    let mut force_sine = false;
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
            "--force-sine" => { force_sine = true; i += 1; }
            other => { out = other.to_string(); i += 1; }
        }
    }

    let data = std::fs::read(bank).map_err(|e| format!("{bank}: {e}"))?;
    let voices = parse_syx(&data)?;
    let v = voices.get(vidx).ok_or_else(|| format!("voice {vidx} が無い(全{}件)", voices.len()))?;
    eprintln!("voice {vidx}: \"{}\" alg={} fb={}", v.name, v.algorithm, v.feedback);
    for (j, op) in v.ops.iter().enumerate() {
        eprintln!(
            "  ops[{j}]->slot{}: out={} freq={}.f{} ow={} ar={} d1l={} rr={} egsft={}",
            slots[j], op.out, op.freq, op.fine, op.ow, op.ar, op.d1l, op.rr, op.egsft
        );
    }

    let mut opz = Opz::new();
    let sr = opz.sample_rate();
    let kc = kc_override.unwrap_or_else(|| midi_to_kc(note));
    eprintln!("note={note} kc=0x{kc:02X} sr={sr} slots={slots:?}");

    let mut buf = render_voice(&mut opz, v, note, kc, slots, sr, gate, dur, force_sine);
    let peak = normalize(&mut buf);
    eprintln!("peak(before norm)={peak:.6} samples={}", buf.len());

    op505_tools::wav::write_wav_mono16(Path::new(&out), &buf, sr).map_err(|e| format!("WAV書き込み失敗: {e}"))?;
    println!("レンダリング: {out} ({:.2}秒, {sr} Hz)", buf.len() as f32 / sr as f32);
    Ok(())
}

fn selftest(out_path: &Path, kc: u8) -> Result<(), String> {
    use opzref::regs::op_reg;

    let mut opz = Opz::new();
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
    opz.write(0x28, kc);
    opz.write(0x30, 0x00);
    opz.write(0x20, 0x80 | 0x40 | 0x07);
    let n = (sr as f32 * 1.5) as usize;
    let mut buf = opz.render(n);
    normalize(&mut buf);
    op505_tools::wav::write_wav_mono16(out_path, &buf, sr).map_err(|e| format!("WAV書き込み失敗: {e}"))?;
    println!("selftest: {} ({sr} Hz)", out_path.display());
    Ok(())
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let res = match args.first().map(|s| s.as_str()) {
        Some("--selftest") => {
            let out = args.get(1).cloned().unwrap_or_else(|| "opzref_selftest.wav".into());
            let kc = args.get(2)
                .and_then(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0x4a);
            selftest(Path::new(&out), kc)
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
