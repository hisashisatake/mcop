use sound_core::convert_wave_32;

/// convert_wave_32の出力テーブル(1024サンプル)のうち、入力32サンプルそのものに
/// 対応する位置（i = k*32）では、入力値とほぼ一致する値が出力される。
#[test]
fn convert_wave_32_samples_at_input_points() {
    let mut input = [0i8; 32];
    for k in 0..32 {
        input[k] = (k as i32 * 4 - 64) as i8;
    }
    let t = convert_wave_32(&input);

    for k in 0..32 {
        let expected = input[k] as f32 / 128.0;
        let actual = t.sample_at(k * 32);
        assert!(
            (actual - expected).abs() < 0.02,
            "input[{k}]={expected} but table[{}] = {actual}", k * 32
        );
    }
}

/// 入力点の中間（i = k*32 + 16）では、隣接する2つの入力サンプルの線形補間値になる。
#[test]
fn convert_wave_32_interpolates_between_input_points() {
    let mut input = [0i8; 32];
    for k in 0..32 {
        input[k] = (k as i32 * 8 - 128) as i8;
    }
    let t = convert_wave_32(&input);

    for k in 0..32 {
        let a = input[k] as f32 / 128.0;
        let b = input[(k + 1) % 32] as f32 / 128.0;
        let expected_mid = (a + b) / 2.0;
        let actual_mid = t.sample_at(k * 32 + 16);
        assert!(
            (actual_mid - expected_mid).abs() < 0.02,
            "midpoint after input[{k}]: expected {expected_mid}, got {actual_mid}"
        );
    }
}
