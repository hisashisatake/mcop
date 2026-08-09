use sound_core::{gen_sawtooth, gen_sine, gen_square, gen_triangle};

#[test]
fn sine_wave_shape() {
    let t = gen_sine();
    let n = t.len();
    assert!(t.sample_at(0).abs() < 0.01, "sine(0) should be ~0, got {}", t.sample_at(0));
    assert!((t.sample_at(n / 4) - 1.0).abs() < 0.01, "sine(1/4) should be ~1");
    assert!(t.sample_at(n / 2).abs() < 0.01, "sine(1/2) should be ~0");
    assert!((t.sample_at(n * 3 / 4) + 1.0).abs() < 0.01, "sine(3/4) should be ~-1");
}

#[test]
fn square_wave_shape() {
    let t = gen_square();
    let n = t.len();
    assert!((t.sample_at(0) - 1.0).abs() < 0.01, "square(0) should be ~1");
    assert!((t.sample_at(n / 2 - 1) - 1.0).abs() < 0.01, "square(1/2-) should be ~1");
    assert!((t.sample_at(n / 2) + 1.0).abs() < 0.01, "square(1/2) should be ~-1");
    assert!((t.sample_at(n - 1) + 1.0).abs() < 0.01, "square(end) should be ~-1");
}

#[test]
fn sawtooth_wave_shape() {
    let t = gen_sawtooth();
    let n = t.len();
    assert!((t.sample_at(0) + 1.0).abs() < 0.01, "sawtooth(0) should be ~-1");
    assert!(t.sample_at(n / 2).abs() < 0.02, "sawtooth(1/2) should be ~0");
    assert!(t.sample_at(n - 1) > 0.9, "sawtooth(end) should be near +1");
}

#[test]
fn triangle_wave_shape() {
    let t = gen_triangle();
    let n = t.len();
    assert!((t.sample_at(0) + 1.0).abs() < 0.01, "triangle(0) should be ~-1");
    assert!(t.sample_at(n / 4).abs() < 0.02, "triangle(1/4) should be ~0");
    assert!((t.sample_at(n / 2) - 1.0).abs() < 0.01, "triangle(1/2) should be ~1");
    assert!(t.sample_at(n * 3 / 4).abs() < 0.02, "triangle(3/4) should be ~0");
}
