//! note_off後にどれくらいの時間、音が鳴り続けるか（リリーステール長）を実測する診断テスト。
//! `cargo test -p ym38x6-core --test release_tail -- --nocapture`
//!
//! presets_dir()配下の.38x6プリセットを読み込み、各プログラムについて
//! note_on→サスティンに達するまでrender→note_off→無音になるまでrender、
//! という流れでリリーステールの長さ（秒）を出力する。
//! ユーザープリセットが存在しない環境では何も出力せずスキップする。

use ym38x6_core::{presets_dir, PresetBank, Vco, Ym38x6Engine};

const SAMPLE_RATE: f32 = 44100.0;

#[test]
fn release_tail() {
    let dir = presets_dir();
    println!("presets_dir: {}", dir.display());
    let bank = PresetBank::load_from_dir(&dir);

    for program in 0u8..=15 {
        let Some(preset) = bank.get(0, program) else { continue };
        println!("\n=== program {program}: {} ===", preset.name);

        let mut engine = Ym38x6Engine::new(SAMPLE_RATE);
        let ch = 0;
        engine.set_patch(preset.patch);
        engine.note_on(ch, 440.0, 127);
        // サスティンに達するまで1秒分render
        let mut buf = vec![0.0f32; SAMPLE_RATE as usize];
        engine.render(&mut buf, 1);
        let sustain_peak = buf[buf.len() - 4410..].iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        println!("  sustain peak (last 0.1s before note_off): {sustain_peak:.5}");

        engine.note_off(ch);

        // note_off後、無音になるまで最大30秒分render
        let chunk = SAMPLE_RATE as usize / 10; // 0.1秒
        let mut elapsed = 0.0f32;
        let threshold = (sustain_peak * 0.01).max(1e-5); // sustain比-40dB or 絶対閾値
        for _ in 0..300 {
            let mut buf = vec![0.0f32; chunk];
            engine.render(&mut buf, 1);
            let peak = buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            elapsed += 0.1;
            if peak < threshold {
                println!("  release tail: {elapsed:.1}s (peak {peak:.6} < threshold {threshold:.6})");
                break;
            }
        }
    }
}
