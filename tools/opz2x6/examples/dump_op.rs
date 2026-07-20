//! 一時診断用: 指定バンク/ボイスの各OPの生パラメータと変換後TL/比率をダンプする。
//! 使い方: cargo run --example dump_op -p opz2x6 --release -- "<syx>" [voice_index]
//! voice_index 省略時は全ボイス名を一覧表示。

use opz2x6::conv::{self, coarse_fine_to_ratio, freq_to_mul_fine, CARRIERS};
use opz2x6::parse;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = &args[0];
    let data = std::fs::read(path).expect("read syx");
    let voices = parse::parse_syx(&data).expect("parse");

    if args.len() < 2 {
        for (i, v) in voices.iter().enumerate() {
            println!("{:3}  {}", i, v.name);
        }
        return;
    }

    let idx: usize = args[1].parse().expect("voice index");
    let v = &voices[idx];
    let alg = v.algorithm.min(7) as usize;
    let carriers = CARRIERS[alg];
    let patch = conv::voice_to_patch_opts(v, conv::ConvOptions::default());

    println!("Voice #{idx}  {}  (alg={alg}, fb={}, transpose={})", v.name, v.feedback, v.transpose);
    println!("operators[] = [OP4, OP3, OP2, OP1]");
    println!(
        "{:>4} {:>4} {:>5} {:>5} {:>4} {:>4} | {:>7} {:>4} {:>4} {:>5} {:>5}",
        "idx", "role", "freq", "fine", "out", "kvs", "ratio", "mul", "oft", "tl", "d1l"
    );
    for i in 0..4 {
        let op = &v.ops[i];
        let is_carrier = carriers.contains(&i);
        let ratio = coarse_fine_to_ratio(op.freq, op.fine);
        let (mul, oft) = freq_to_mul_fine(op.freq, op.fine);
        let p = &patch.operators[i];
        println!(
            "{:>4} {:>4} {:>5} {:>5} {:>4} {:>4} | {:>7.4} {:>4} {:>4} {:>5} {:>5}",
            i,
            if is_carrier { "CAR" } else { "mod" },
            op.freq,
            op.fine,
            op.out,
            op.kvs,
            ratio,
            mul,
            oft,
            p.tl,
            p.d1l,
        );
    }
}
