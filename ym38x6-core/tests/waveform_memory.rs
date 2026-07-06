//! 波形メモリ音源モード（38x6のOP1のみ有効な1オペレーター音色）の統合テスト。
//!
//! 旧wms1-coreが持っていたパフォーマンスLFO・ポリフォニー・波形スロット選択のテストを、
//! 廃止に伴いym38x6エンジン向けに移植したもの。波形メモリ音色は`waveform_memory_patch`で
//! 生成する（Algorithm 7・OP1のみ可聴・OP2〜4はTL=0でミュート）。
//!
//! 注意: ym38x6は全ボイスにSVFフィルターが常時かかるため、旧WMS-1のような「実効音量0で
//! 完全無音」という厳密値の断定はできない（フィルターの内部状態でわずかに尾を引く）。
//! そのためLFO系は「振幅が周期的に大きく変動する」ことをピーク比較で検証する。

use ym38x6_core::{
    waveform_memory_patch, AdsrParams, LfoWaveform, Ym38x6Engine,
    Ym38x6LfoDestination,
};

/// 即アタック・無減衰・無限サスティンのADSR（出力レベルを1.0付近で保持し、
/// オシレーター波形とLFOの効果を観測しやすくする）。
fn sustained_adsr() -> AdsrParams {
    AdsrParams { attack: 255, decay: 0, sustain: 255, release: 0 }
}

/// バッファをウィンドウ分割し、各ウィンドウのピーク振幅（絶対値の最大）を返す。
fn window_peaks(buf: &[f32], window: usize) -> Vec<f32> {
    buf.chunks(window)
        .map(|c| c.iter().fold(0.0f32, |m, &s| m.max(s.abs())))
        .collect()
}

/// Destination=Volume・矩形波・Depth=1.0のパフォーマンスLFOをかけると、
/// 実効音量がLFOの半周期ごとに大きく上下し、振幅が周期的に変動する。
#[test]
fn performance_lfo_volume_destination_modulates_amplitude() {
    let sample_rate = 44100.0;
    let mut engine = Ym38x6Engine::new(sample_rate);
    let ch = 0;
    // 矩形波（waveform=3）は振幅が常に±1付近なので、振幅変動はLFOの効果として観測できる。
    engine.note_on(ch, 220.0, 127, waveform_memory_patch(3, sustained_adsr()));
    // rate=255（20Hz、最速）でバッファ内に複数周期が収まるようにする。
    engine.set_performance_lfo(ch, 255, 0, LfoWaveform::Square, Ym38x6LfoDestination::Volume, 1.0);

    // アタックを終えて出力が安定するまでウォームアップ
    let mut warmup = vec![0.0f32; 200];
    engine.render(&mut warmup, 1);

    let mut buf = vec![0.0f32; 4410]; // LFO約2周期分（20Hz@44.1kHz）
    engine.render(&mut buf, 1);

    let peaks = window_peaks(&buf, 64);
    let max_peak = peaks.iter().cloned().fold(0.0f32, f32::max);
    let min_peak = peaks.iter().cloned().fold(f32::MAX, f32::min);

    assert!(max_peak > 0.3, "変調中も大振幅の区間があるはず: max_peak={max_peak}");
    assert!(
        min_peak < max_peak * 0.5,
        "Volume LFOで振幅が周期的に大きく落ちるはず: min={min_peak} max={max_peak}"
    );
}

/// Destination=Pitch・Depth>0のパフォーマンスLFOは実効周波数を揺らすため、
/// Depth=0の場合と出力波形が乖離する。
#[test]
fn performance_lfo_pitch_destination_shifts_output() {
    let sample_rate = 44100.0;

    let mut engine_flat = Ym38x6Engine::new(sample_rate);
    engine_flat.note_on(0, 440.0, 127, waveform_memory_patch(0, sustained_adsr()));
    engine_flat.set_performance_lfo(0, 220, 0, LfoWaveform::Sine, Ym38x6LfoDestination::Pitch, 0.0);
    let mut warm_flat = vec![0.0f32; 200];
    engine_flat.render(&mut warm_flat, 1);
    let mut buf_flat = vec![0.0f32; 400];
    engine_flat.render(&mut buf_flat, 1);

    let mut engine_mod = Ym38x6Engine::new(sample_rate);
    engine_mod.note_on(0, 440.0, 127, waveform_memory_patch(0, sustained_adsr()));
    // ±1200セント（±1オクターブ）の大きめのビブラート
    engine_mod.set_performance_lfo(0, 220, 0, LfoWaveform::Sine, Ym38x6LfoDestination::Pitch, 1200.0);
    let mut warm_mod = vec![0.0f32; 200];
    engine_mod.render(&mut warm_mod, 1);
    let mut buf_mod = vec![0.0f32; 400];
    engine_mod.render(&mut buf_mod, 1);

    let differs = buf_flat.iter().zip(buf_mod.iter()).any(|(a, b)| (a - b).abs() > 1e-3);
    assert!(differs, "ピッチ変調により出力波形が変化するはず");
}

/// 同一波形・同一周波数・同一パッチの2音を別チャンネルIDで同時発音すると、
/// 出力は1音の場合のちょうど2倍になる（各ボイス独立・加算合成）。
#[test]
fn polyphony_two_identical_voices_sum_to_double() {
    let sample_rate = 44100.0;
    let patch = waveform_memory_patch(0, sustained_adsr());

    let mut engine_one = Ym38x6Engine::new(sample_rate);
    engine_one.note_on(0, 440.0, 127, patch);
    let mut buf_one = vec![0.0f32; 256];
    engine_one.render(&mut buf_one, 1);

    let mut engine_two = Ym38x6Engine::new(sample_rate);
    engine_two.note_on(0, 440.0, 127, patch);
    engine_two.note_on(1, 440.0, 127, patch);
    let mut buf_two = vec![0.0f32; 256];
    engine_two.render(&mut buf_two, 1);

    for i in 0..buf_one.len() {
        assert!(
            (buf_two[i] - 2.0 * buf_one[i]).abs() < 1e-5,
            "sample {i}: 2音の合成は1音の2倍になるはず: expected {}, got {}",
            2.0 * buf_one[i],
            buf_two[i]
        );
    }
}

/// リリースが完了して無音になったチャンネルは、以降のミックスに影響しなくなる。
/// 比較用に「持続音のみ」を同じtick数だけ鳴らしたエンジンと、後半ウィンドウで一致することを確認する。
#[test]
fn finished_channel_no_longer_affects_mix() {
    let sample_rate = 44100.0;
    let releasing = waveform_memory_patch(0, AdsrParams { attack: 255, decay: 0, sustain: 255, release: 255 });
    let sustained = waveform_memory_patch(0, sustained_adsr());

    const WARMUP: usize = 200;
    const SETTLE: usize = 2000; // リリース完了（rr=255）に十分な長さ
    const COMPARE: usize = 256;

    // ミックス: 持続音(B, ch=1, 660Hz) + 途中で離鍵する音(A, ch=0, 440Hz)
    let mut mix = Ym38x6Engine::new(sample_rate);
    mix.note_on(0, 440.0, 127, releasing);
    mix.note_on(1, 660.0, 127, sustained);
    let mut warm = vec![0.0f32; WARMUP];
    mix.render(&mut warm, 1);
    mix.note_off(0); // Aを離鍵
    let mut settle = vec![0.0f32; SETTLE];
    mix.render(&mut settle, 1);
    let mut buf_mix = vec![0.0f32; COMPARE];
    mix.render(&mut buf_mix, 1);

    // 参照: 持続音(B)のみを同じtick数だけ鳴らす（Bのボイス状態が一致する）
    let mut reference = Ym38x6Engine::new(sample_rate);
    reference.note_on(1, 660.0, 127, sustained);
    let mut warm_ref = vec![0.0f32; WARMUP];
    reference.render(&mut warm_ref, 1);
    let mut settle_ref = vec![0.0f32; SETTLE];
    reference.render(&mut settle_ref, 1);
    let mut buf_ref = vec![0.0f32; COMPARE];
    reference.render(&mut buf_ref, 1);

    for i in 0..COMPARE {
        assert!(
            (buf_mix[i] - buf_ref[i]).abs() < 1e-5,
            "sample {i}: 離鍵済みAはミックスからきれいに脱落し、持続音B単独と一致するはず: \
             ref={}, mix={}",
            buf_ref[i],
            buf_mix[i]
        );
    }
}

/// 波形スロットを変えると、実際に異なる波形テーブルが選択され出力が変わる
/// （エンジン経由での波形スロット選択のend-to-end確認）。
#[test]
fn different_waveform_slots_produce_different_output() {
    let sample_rate = 44100.0;
    let render_slot = |waveform: u8| {
        let mut engine = Ym38x6Engine::new(sample_rate);
        engine.note_on(0, 440.0, 127, waveform_memory_patch(waveform, sustained_adsr()));
        let mut warmup = vec![0.0f32; 200];
        engine.render(&mut warmup, 1);
        let mut buf = vec![0.0f32; 512];
        engine.render(&mut buf, 1);
        buf
    };

    let sine = render_slot(0); // サイン
    let square = render_slot(3); // 矩形
    let saw = render_slot(4); // ノコギリ

    let diff = |a: &[f32], b: &[f32]| a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-3);
    assert!(diff(&sine, &square), "サインと矩形は異なる出力になるはず");
    assert!(diff(&sine, &saw), "サインとノコギリは異なる出力になるはず");
    assert!(diff(&square, &saw), "矩形とノコギリは異なる出力になるはず");
}
