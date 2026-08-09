use sound_core::{LfoFadeMode, LfoWaveform, PerformanceLfo, PerformanceLfoShape};

/// rate=255は約20Hzにマッピングされる。sample_rate=160Hzにすると
/// 1周期がちょうど8サンプルになり、各位相での出力値を直接比較できる。
const SAMPLE_RATE: f32 = 160.0;

fn ticks(lfo: &mut PerformanceLfo, n: usize) -> Vec<f32> {
    (0..n).map(|_| lfo.tick(SAMPLE_RATE)).collect()
}

#[test]
fn triangle_lfo_shape() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_waveform(LfoWaveform::Triangle);
    lfo.note_on();

    let v = ticks(&mut lfo, 8);
    assert!((v[1] - 0.0).abs() < 0.02, "phase 1/4: {}", v[1]);
    assert!((v[3] - 1.0).abs() < 0.02, "phase 1/2: {}", v[3]);
    assert!((v[5] - 0.0).abs() < 0.02, "phase 3/4: {}", v[5]);
    assert!((v[7] + 1.0).abs() < 0.02, "phase ~1: {}", v[7]);
}

#[test]
fn sine_lfo_shape() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_waveform(LfoWaveform::Sine);
    lfo.note_on();

    let v = ticks(&mut lfo, 8);
    assert!((v[1] - 1.0).abs() < 0.02, "phase 1/4: {}", v[1]);
    assert!((v[3] - 0.0).abs() < 0.02, "phase 1/2: {}", v[3]);
    assert!((v[5] + 1.0).abs() < 0.02, "phase 3/4: {}", v[5]);
    assert!((v[7] - 0.0).abs() < 0.02, "phase ~1: {}", v[7]);
}

#[test]
fn square_lfo_shape() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_waveform(LfoWaveform::Square);
    lfo.note_on();

    let v = ticks(&mut lfo, 8);
    assert_eq!(v[0], 1.0, "phase 1/8 should be +1");
    assert_eq!(v[4], -1.0, "phase 5/8 should be -1");
}

/// S&H波形は値域内に収まり、周期ごとに更新されるため毎サンプルは変化しない
/// （= 複数サンプルにわたって同じ値を保持する区間がある）。
#[test]
fn sample_hold_lfo_updates_and_stays_in_range() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_waveform(LfoWaveform::SampleHold);
    lfo.note_on();

    let v = ticks(&mut lfo, 32); // 約4周期分

    for &x in &v {
        assert!((-1.0..=1.0).contains(&x), "S&H value out of range: {x}");
    }

    let distinct: std::collections::HashSet<_> = v.iter().map(|x| x.to_bits()).collect();
    assert!(distinct.len() >= 2, "S&H should produce more than one value over several cycles");
    assert!(distinct.len() < v.len(), "S&H should hold each value across multiple samples");
}

/// ディレイ経過前は常に0.0、経過後は変調値が出力される
#[test]
fn delay_gates_output_until_elapsed() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(128); // delay_to_seconds(128) ≈ 5.02秒
    lfo.set_waveform(LfoWaveform::Triangle);
    lfo.note_on();

    let sample_rate = 1000.0;

    let before: Vec<f32> = (0..5000).map(|_| lfo.tick(sample_rate)).collect();
    assert!(before.iter().all(|&v| v == 0.0), "expected silence during delay");

    let after: Vec<f32> = (0..100).map(|_| lfo.tick(sample_rate)).collect();
    assert!(after.iter().any(|&v| v != 0.0), "expected non-zero modulation after delay");
}

/// note_onで位相とディレイ経過時間がリセットされる
#[test]
fn note_on_resets_delay() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(128);
    lfo.set_waveform(LfoWaveform::Triangle);
    lfo.note_on();

    let sample_rate = 1000.0;
    for _ in 0..3000 {
        lfo.tick(sample_rate);
    }

    lfo.note_on();
    assert_eq!(lfo.tick(sample_rate), 0.0, "note_on直後はディレイが再びかかるはず");
}

#[test]
fn saw_lfo_shape() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_waveform(LfoWaveform::Saw);
    lfo.note_on();

    let v = ticks(&mut lfo, 8);
    for &x in &v {
        assert!((-1.0..=1.0).contains(&x), "Saw value out of range: {x}");
    }
    assert!((v[0] + 0.75).abs() < 0.02, "phase 1/8: {}", v[0]);
    assert!((v[3] - 0.0).abs() < 0.02, "phase 1/2: {}", v[3]);
    assert!((v[7] + 1.0).abs() < 0.02, "phase ~1 (wrap to 0): {}", v[7]);
}

/// 三角波なら頂点は1サンプルのみだが、Trapezoidは上下でクリップされ
/// 複数サンプルにわたって±1近辺でフラットになる（台形の平らな部分）。
#[test]
fn trapezoid_lfo_shape_has_flat_plateau() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_waveform(LfoWaveform::Trapezoid);
    lfo.note_on();

    let v = ticks(&mut lfo, 8);
    for &x in &v {
        assert!((-1.0..=1.0).contains(&x), "Trapezoid value out of range: {x}");
    }

    let near_plus_one = v.iter().filter(|&&x| (x - 1.0).abs() < 0.02).count();
    assert!(
        near_plus_one >= 2,
        "expected a flat plateau near +1, got {near_plus_one} samples: {v:?}"
    );
}

/// RandomはSampleHoldと異なり、周期境界で抽選した目標値へ位相で線形補間するため
/// 1周期内のサンプルがほぼ全て異なる値を取る（SampleHoldのような複数サンプル保持がない）。
#[test]
fn random_lfo_interpolates_smoothly_and_stays_in_range() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_waveform(LfoWaveform::Random);
    lfo.note_on();

    let v = ticks(&mut lfo, 32); // 4周期分、最初の1周期はウォームアップ

    for &x in &v {
        assert!((-1.0..=1.0).contains(&x), "Random value out of range: {x}");
    }

    let period = &v[16..24]; // 3周期目（補間が効いている区間）
    let distinct: std::collections::HashSet<_> = period.iter().map(|x| x.to_bits()).collect();
    assert!(
        distinct.len() >= 6,
        "Random should vary within a period, got {distinct:?} from {period:?}"
    );
}

/// Chaosはロジスティック写像による決定論的カオス。note_onで同じ初期状態から
/// 始まるため再現性があり、複数周期にわたって非周期的に値が変化する。
#[test]
fn chaos_lfo_is_deterministic_and_in_range() {
    let mut lfo1 = PerformanceLfo::new();
    lfo1.set_rate(255);
    lfo1.set_delay(0);
    lfo1.set_waveform(LfoWaveform::Chaos);
    lfo1.note_on();
    let v1 = ticks(&mut lfo1, 32);

    let mut lfo2 = PerformanceLfo::new();
    lfo2.set_rate(255);
    lfo2.set_delay(0);
    lfo2.set_waveform(LfoWaveform::Chaos);
    lfo2.note_on();
    let v2 = ticks(&mut lfo2, 32);

    assert_eq!(v1, v2, "Chaos should be deterministic from the same note_on state");

    for &x in &v1 {
        assert!((-1.0..=1.0).contains(&x), "Chaos value out of range: {x}");
    }

    let distinct: std::collections::HashSet<_> = v1.iter().map(|x| x.to_bits()).collect();
    assert!(distinct.len() >= 3, "Chaos should vary across cycles, got {distinct:?}");
}

/// Offsetは生値(-1.0〜1.0)に加算後クランプされる。offset=+100で下側(-1.0)が
/// 0.0まで持ち上がり、上側(+1.0)は+1.0のままクランプされる。
#[test]
fn offset_shifts_center_and_clamps() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_shape(PerformanceLfoShape {
        waveform: LfoWaveform::Square,
        fade_mode: LfoFadeMode::OnIn,
        fade_time: 0,
        offset: 100,
    });
    lfo.note_on();

    let v = ticks(&mut lfo, 8);
    assert_eq!(v[0], 1.0, "raw +1 shifted by offset(+100) should clamp to +1");
    assert_eq!(v[4], 0.0, "raw -1 shifted by offset(+100) should land at 0.0");
}

/// fade_time=0はフェード無効（常時フルゲイン）。旧来のハードエッジ挙動と等価になる
/// 後方互換の既定値であることを確認する。
#[test]
fn fade_time_zero_disables_fade() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_shape(PerformanceLfoShape {
        waveform: LfoWaveform::Square,
        fade_mode: LfoFadeMode::OnIn,
        fade_time: 0,
        offset: 0,
    });
    lfo.note_on();

    assert_eq!(lfo.tick(1000.0).abs(), 1.0, "fade_time=0 should give full amplitude immediately");
}

#[test]
fn fade_on_in_ramps_amplitude_from_zero() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_shape(PerformanceLfoShape {
        waveform: LfoWaveform::Square,
        fade_mode: LfoFadeMode::OnIn,
        fade_time: 128, // delay_to_seconds(128) ≈ 5.02秒
        offset: 0,
    });
    lfo.note_on();

    let sample_rate = 1000.0;
    let just_after_on = lfo.tick(sample_rate).abs();
    for _ in 0..2000 {
        lfo.tick(sample_rate);
    }
    let mid = lfo.tick(sample_rate).abs();
    for _ in 0..10000 {
        lfo.tick(sample_rate);
    }
    let after = lfo.tick(sample_rate).abs();

    assert!(just_after_on < 0.05, "immediately after note_on amplitude should be ~0, got {just_after_on}");
    assert!(mid > just_after_on && mid < after, "amplitude should ramp up: {just_after_on} < {mid} < {after}");
    assert!((after - 1.0).abs() < 0.02, "after fade_time amplitude should reach full, got {after}");
}

#[test]
fn fade_on_out_ramps_amplitude_to_zero() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_shape(PerformanceLfoShape {
        waveform: LfoWaveform::Square,
        fade_mode: LfoFadeMode::OnOut,
        fade_time: 128,
        offset: 0,
    });
    lfo.note_on();

    let sample_rate = 1000.0;
    let just_after_on = lfo.tick(sample_rate).abs();
    for _ in 0..12000 {
        lfo.tick(sample_rate);
    }
    let after = lfo.tick(sample_rate).abs();

    assert!((just_after_on - 1.0).abs() < 0.02, "immediately after note_on amplitude should be full, got {just_after_on}");
    assert!(after < 0.05, "after fade_time amplitude should reach ~0, got {after}");
}

#[test]
fn fade_off_in_ramps_amplitude_from_zero_after_note_off() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_shape(PerformanceLfoShape {
        waveform: LfoWaveform::Square,
        fade_mode: LfoFadeMode::OffIn,
        fade_time: 128,
        offset: 0,
    });
    lfo.note_on();

    let sample_rate = 1000.0;
    let held = lfo.tick(sample_rate).abs();
    for _ in 0..500 {
        lfo.tick(sample_rate);
    }
    assert!(held < 0.02, "while held (before note_off), OffIn amplitude should be ~0, got {held}");

    lfo.note_off();
    let just_after_off = lfo.tick(sample_rate).abs();
    for _ in 0..10000 {
        lfo.tick(sample_rate);
    }
    let after = lfo.tick(sample_rate).abs();

    assert!(just_after_off < 0.05, "immediately after note_off amplitude should still be ~0, got {just_after_off}");
    assert!((after - 1.0).abs() < 0.02, "after fade_time past note_off amplitude should reach full, got {after}");
}

#[test]
fn fade_off_out_ramps_amplitude_to_zero_after_note_off() {
    let mut lfo = PerformanceLfo::new();
    lfo.set_rate(255);
    lfo.set_delay(0);
    lfo.set_shape(PerformanceLfoShape {
        waveform: LfoWaveform::Square,
        fade_mode: LfoFadeMode::OffOut,
        fade_time: 128,
        offset: 0,
    });
    lfo.note_on();

    let sample_rate = 1000.0;
    let held = lfo.tick(sample_rate).abs();
    for _ in 0..500 {
        lfo.tick(sample_rate);
    }
    assert!((held - 1.0).abs() < 0.02, "while held (before note_off), OffOut amplitude should be full, got {held}");

    lfo.note_off();
    let just_after_off = lfo.tick(sample_rate).abs();
    for _ in 0..10000 {
        lfo.tick(sample_rate);
    }
    let after = lfo.tick(sample_rate).abs();

    assert!((just_after_off - 1.0).abs() < 0.05, "immediately after note_off amplitude should still be near full, got {just_after_off}");
    assert!(after < 0.05, "after fade_time past note_off amplitude should reach ~0, got {after}");
}
