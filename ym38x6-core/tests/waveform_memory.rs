//! 波形メモリ音源モード（38x6のOP1のみ有効な1オペレーター音色）の統合テスト。
//!
//! 旧wms1-coreが持っていたパフォーマンスLFO・ポリフォニー・波形スロット選択のテストを、
//! 廃止に伴いym38x6エンジン向けに移植したもの。波形メモリ音色は`waveform_memory_patch`で
//! 生成する（Algorithm 7・OP1のみ可聴・OP2〜4はTL=0でミュート）。
//!
//! 注意: ym38x6は全ボイスにSVFフィルターが常時かかるため、旧WMS-1のような「実効音量0で
//! 完全無音」という厳密値の断定はできない（フィルターの内部状態でわずかに尾を引く）。
//! そのためLFO系は「振幅が周期的に大きく変動する」ことをピーク比較で検証する。

use ym38x6_core::{waveform_memory_patch, AdsrParams, TextureLfo, Vco, Ym38x6Engine};

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

/// Destination=Volume・矩形波・Depth=255（最大）の質感LFOをかけると、
/// 実効音量がLFOの半周期ごとに大きく上下し、振幅が周期的に変動する。
#[test]
fn texture_lfo_volume_destination_modulates_amplitude() {
    let sample_rate = 44100.0;
    let mut engine = Ym38x6Engine::new(sample_rate);
    let ch = 0;
    // 矩形波（waveform=3）は振幅が常に±1付近なので、振幅変動はLFOの効果として観測できる。
    let mut patch = waveform_memory_patch(3, sustained_adsr());
    // 質感LFO波形も矩形(0)、destination=1(Volume)、rate=255（20Hz、最速）でバッファ内に
    // 複数周期が収まるようにし、depth=255（最大）で振幅を大きく揺らす。
    patch.channel.texture_lfo =
        TextureLfo { waveform: 0, destination: 1, rate: 255, depth: 255, ..TextureLfo::default() };
    engine.set_patch(patch);
    engine.note_on(ch, 220.0, 127);

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

/// パッチの`ChannelParams.texture_lfo`がnote_on時にPerformanceLfoへ反映されることを
/// end-to-endで確認する。fade_timeだけが異なる2エンジンを同条件で鳴らし、出力が違うことで
/// 「patchのfade_timeが実際に効いている」ことを示す（filter/VCA由来の他の変動と混同しないよう、
/// 絶対的な振幅閾値ではなく差分の有無で判定する）。
#[test]
fn texture_lfo_shape_from_patch_is_applied_at_note_on() {
    let sample_rate = 44100.0;
    let render_with_fade_time = |fade_time: u8| {
        let mut engine = Ym38x6Engine::new(sample_rate);
        let mut patch = waveform_memory_patch(3, sustained_adsr()); // 矩形波（振幅±1で変動が見やすい）
        patch.channel.texture_lfo = TextureLfo {
            waveform: 0,     // 矩形波
            destination: 1,  // Volume
            rate: 255,
            depth: 255,
            fade_mode: 0,    // ON-IN
            fade_time,
            ..TextureLfo::default()
        };
        engine.set_patch(patch);
        engine.note_on(0, 220.0, 127);

        let mut warmup = vec![0.0f32; 200];
        engine.render(&mut warmup, 1);
        let mut buf = vec![0.0f32; 2000];
        engine.render(&mut buf, 1);
        buf
    };

    let long_fade = render_with_fade_time(200); // delay_to_seconds(200)≈7.84秒、この窓ではほぼゲイン0
    let no_fade = render_with_fade_time(0); // フェード無効、常時フルゲイン

    let differs = long_fade.iter().zip(no_fade.iter()).any(|(a, b)| (a - b).abs() > 1e-3);
    assert!(
        differs,
        "patchのtexture_lfo.fade_timeが反映されていれば、fade_time違いで出力が変わるはず"
    );
}

/// `set_channel_params`で`texture_lfo`を変更すると、note_on済みの発音中ボイスにも
/// 次ブロックからリアルタイムに反映される（VSTのNRPN/DAWパラメーター変更が発音中ノートへ
/// 効くことを保証する）。
#[test]
fn texture_lfo_shape_updates_live_via_set_channel_params() {
    let sample_rate = 44100.0;
    let render = |switch_waveform: bool| {
        let mut engine = Ym38x6Engine::new(sample_rate);
        let mut patch = waveform_memory_patch(3, sustained_adsr());
        patch.channel.texture_lfo =
            TextureLfo { waveform: 0, destination: 1, rate: 255, depth: 255, ..TextureLfo::default() };
        engine.set_patch(patch);
        engine.note_on(0, 220.0, 127);

        let mut warmup = vec![0.0f32; 200];
        engine.render(&mut warmup, 1);

        if switch_waveform {
            let mut updated = patch.channel;
            updated.texture_lfo.waveform = 1; // 矩形波→台形波へ切替
            engine.set_channel_params(0, updated);
        }

        let mut buf = vec![0.0f32; 2000];
        engine.render(&mut buf, 1);
        buf
    };

    let switched = render(true);
    let unswitched = render(false);

    let differs = switched.iter().zip(unswitched.iter()).any(|(a, b)| (a - b).abs() > 1e-3);
    assert!(
        differs,
        "note_on後にset_channel_paramsでwaveformを変更すると、発音中でも出力が変わるはず"
    );
}

/// `set_patch_live`（gesture-appの音色エディタのノブ操作が呼ぶ）は、`set_patch`と異なり
/// 発音中の全チャンネルへも即座にパラメーターを伝播する。これに対し`set_patch`（Program Change
/// 相当）は次のnote-onまで発音中の音を変えないままであることも合わせて確認する。
#[test]
fn set_patch_live_updates_active_channel_but_set_patch_does_not() {
    let sample_rate = 44100.0;
    let base = waveform_memory_patch(3, sustained_adsr());
    let mut brighter = base;
    brighter.channel.filter_cutoff = 40;

    let render_with = |use_live: bool| {
        let mut engine = Ym38x6Engine::new(sample_rate);
        engine.set_patch(base);
        engine.note_on(0, 220.0, 127);
        let mut warmup = vec![0.0f32; 200];
        engine.render(&mut warmup, 1);

        if use_live {
            engine.set_patch_live(brighter);
        } else {
            engine.set_patch(brighter);
        }

        let mut buf = vec![0.0f32; 400];
        engine.render(&mut buf, 1);
        buf
    };

    let live = render_with(true);
    let not_live = render_with(false);

    let live_differs_from_base = {
        let mut engine = Ym38x6Engine::new(sample_rate);
        engine.set_patch(base);
        engine.note_on(0, 220.0, 127);
        let mut warmup = vec![0.0f32; 200];
        engine.render(&mut warmup, 1);
        let mut buf = vec![0.0f32; 400];
        engine.render(&mut buf, 1);
        buf
    };

    assert!(
        live.iter().zip(live_differs_from_base.iter()).any(|(a, b)| (a - b).abs() > 1e-3),
        "set_patch_liveは発音中チャンネルにも即座に反映されるはず"
    );
    assert_eq!(
        not_live, live_differs_from_base,
        "set_patch（Program Change相当）は発音中チャンネルを変えず、次のnote-onまで据え置かれるはず"
    );
}

/// Destination=Pitch・Depth>0の質感LFOは実効周波数を揺らすため、Depth=0の場合と出力波形が乖離する。
#[test]
fn texture_lfo_pitch_destination_shifts_output() {
    let sample_rate = 44100.0;

    let mut patch = waveform_memory_patch(0, sustained_adsr());
    patch.channel.texture_lfo =
        TextureLfo { waveform: 0, destination: 0, rate: 220, depth: 0, ..TextureLfo::default() };

    let mut engine_flat = Ym38x6Engine::new(sample_rate);
    engine_flat.set_patch(patch);
    engine_flat.note_on(0, 440.0, 127);
    let mut warm_flat = vec![0.0f32; 200];
    engine_flat.render(&mut warm_flat, 1);
    let mut buf_flat = vec![0.0f32; 400];
    engine_flat.render(&mut buf_flat, 1);

    // depth=255（質感LFOの最大値）の大きめのビブラート
    let mut patch_mod = patch;
    patch_mod.channel.texture_lfo.depth = 255;
    let mut engine_mod = Ym38x6Engine::new(sample_rate);
    engine_mod.set_patch(patch_mod);
    engine_mod.note_on(0, 440.0, 127);
    let mut warm_mod = vec![0.0f32; 200];
    engine_mod.render(&mut warm_mod, 1);
    let mut buf_mod = vec![0.0f32; 400];
    engine_mod.render(&mut buf_mod, 1);

    let differs = buf_flat.iter().zip(buf_mod.iter()).any(|(a, b)| (a - b).abs() > 1e-3);
    assert!(differs, "ピッチ変調により出力波形が変化するはず");
}

/// Destination=Cutoff（オートワウ）は、LFOでシフトした基準CutoffをVcfへ渡すことで
/// 音色（倍音構成）を持続的に変化させる。波形は矩形（倍音豊富でカットオフの効果が
/// 出やすい）、基準Cutoffを低め(80)にしておき、LFOなし/ありの出力波形が異なることを確認する。
#[test]
fn texture_lfo_cutoff_destination_shifts_output() {
    let sample_rate = 44100.0;

    let mut patch = waveform_memory_patch(3, sustained_adsr());
    patch.channel.filter_cutoff = 80;
    patch.channel.texture_lfo =
        TextureLfo { waveform: 0, destination: 3, rate: 220, depth: 0, ..TextureLfo::default() };

    let mut engine_flat = Ym38x6Engine::new(sample_rate);
    engine_flat.set_patch(patch);
    engine_flat.note_on(0, 220.0, 127);
    let mut warm_flat = vec![0.0f32; 200];
    engine_flat.render(&mut warm_flat, 1);
    let mut buf_flat = vec![0.0f32; 400];
    engine_flat.render(&mut buf_flat, 1);

    // 基準Cutoff(80)を中心に±150の大きめのオートワウ
    let mut patch_mod = patch;
    patch_mod.channel.texture_lfo.depth = 150;
    let mut engine_mod = Ym38x6Engine::new(sample_rate);
    engine_mod.set_patch(patch_mod);
    engine_mod.note_on(0, 220.0, 127);
    let mut warm_mod = vec![0.0f32; 200];
    engine_mod.render(&mut warm_mod, 1);
    let mut buf_mod = vec![0.0f32; 400];
    engine_mod.render(&mut buf_mod, 1);

    let differs = buf_flat.iter().zip(buf_mod.iter()).any(|(a, b)| (a - b).abs() > 1e-3);
    assert!(differs, "カットオフLFO変調（オートワウ）により出力波形が変化するはず");
}

/// 同一波形・同一周波数・同一パッチの2音を別チャンネルIDで同時発音すると、
/// 出力は1音の場合のちょうど2倍になる（各ボイス独立・加算合成）。
#[test]
fn polyphony_two_identical_voices_sum_to_double() {
    let sample_rate = 44100.0;
    let patch = waveform_memory_patch(0, sustained_adsr());

    let mut engine_one = Ym38x6Engine::new(sample_rate);
    engine_one.set_patch(patch);
    engine_one.note_on(0, 440.0, 127);
    let mut buf_one = vec![0.0f32; 256];
    engine_one.render(&mut buf_one, 1);

    let mut engine_two = Ym38x6Engine::new(sample_rate);
    engine_two.set_patch(patch);
    engine_two.note_on(0, 440.0, 127);
    engine_two.set_patch(patch);
    engine_two.note_on(1, 440.0, 127);
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
    mix.set_patch(releasing);
    mix.note_on(0, 440.0, 127);
    mix.set_patch(sustained);
    mix.note_on(1, 660.0, 127);
    let mut warm = vec![0.0f32; WARMUP];
    mix.render(&mut warm, 1);
    mix.note_off(0); // Aを離鍵
    let mut settle = vec![0.0f32; SETTLE];
    mix.render(&mut settle, 1);
    let mut buf_mix = vec![0.0f32; COMPARE];
    mix.render(&mut buf_mix, 1);

    // 参照: 持続音(B)のみを同じtick数だけ鳴らす（Bのボイス状態が一致する）
    let mut reference = Ym38x6Engine::new(sample_rate);
    reference.set_patch(sustained);
    reference.note_on(1, 660.0, 127);
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
        engine.set_patch(waveform_memory_patch(waveform, sustained_adsr()));
        engine.note_on(0, 440.0, 127);
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
