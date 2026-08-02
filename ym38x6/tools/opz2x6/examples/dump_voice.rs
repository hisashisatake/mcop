//! 調査用: TX81Z .syx の指定ボイスの周波数関連レジスタ生値と、
//! opz2x6変換後の実効周波数比をダンプする。
//!
//! 使い方: cargo run --example dump_voice -- <input.syx> <voice_num>

use opz2x6::conv;
use opz2x6::parse;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = &args[0];
    let voice_num: usize = args[1].parse().expect("voice番号");
    let data = std::fs::read(path).expect("syx読み込み");
    let voices = parse::parse_syx(&data).expect("parse");
    let v = &voices[voice_num];
    println!(
        "Voice {}: \"{}\"  alg={} fb={}",
        v.number, v.name, v.algorithm, v.feedback
    );
    let carriers = conv::CARRIERS[v.algorithm.min(7) as usize];
    // ops は [OP4,OP3,OP2,OP1]、38x6 operators[i] = ops[3-i]
    for i in 0..4 {
        let op = &v.ops[3 - i];
        let ratio = conv::coarse_fine_to_ratio(op.freq, op.fine);
        let (mul, oft) = conv::freq_to_mul_fine(op.freq, op.fine);
        let mul_ratio = if mul == 0 { 0.5 } else { mul as f32 };
        let eff = mul_ratio * 2f32.powf((oft as f32 - 128.0) / 127.0);
        let role = if carriers.contains(&i) { "CARRIER" } else { "mod    " };
        println!(
            "  operators[{}] OP{} [{}]  freq(coarse)={:2} fine={:2} det={} ow={}  |  parsed_ratio={:.4}  -> mul={:2} oft={:3} eff_ratio={:.4}",
            i, i + 1, role, op.freq, op.fine, op.det, op.ow, ratio, mul, oft, eff
        );
    }
}
