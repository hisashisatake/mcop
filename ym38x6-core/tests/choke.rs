use ym38x6_core::{ChannelParams, OperatorParams, Vco, Ym38x6Engine, Ym38x6Patch};

/// AR最速・サスティン無限・RR=200（中速リリース）の4op並列(algorithm 7)パッチ。
/// frequency=440.0(note=69)でKSRの影響を受けず、レート計算が単純になる。
fn sustained_release_patch() -> Ym38x6Patch {
    let op = OperatorParams {
        tl: 255,
        ar: 255,
        d1r: 0,
        d2r: 0,
        d1l: 255,
        rr: 200,
        mul: 1,
        dt1: 128,
        ksr: 0,
        am_enable: false,
        velocity_sensitivity: 0,
        waveform: 0, // サイン波（サンプル間の段差が小さく、連続性を観測しやすい）
        op_fine_tune: 128,
        floor: 0,
        loop_enabled: 0,
        curve: 0,
    };
    Ym38x6Patch {
        operators: [op; 4],
        channel: ChannelParams { algorithm: 7, ..ChannelParams::default() },
    }
}

/// リリース中の同じチャンネルIDへ再度`note_on`すると、env_levelを0に
/// リセットせず残響レベルから再アタックする（実機OPMのKey-On挙動）。
///
/// 残響保持を単独で観測するため、再アタックのARを最遅(ar=0)にする。
/// env_levelを0へ落とす旧「同音チョーク」実装なら無音から始まるが、残響再アタックなら
/// 直前のリリースレベル付近を維持する。
#[test]
fn note_on_retriggers_from_residual_not_silence() {
    let mut engine = Ym38x6Engine::new(44100.0);
    let patch = sustained_release_patch();
    let ch = 0;
    engine.set_patch(patch);
    engine.note_on(ch, 440.0, 127);
    let mut warmup = vec![0.0f32; 100];
    engine.render(&mut warmup, 1);

    engine.note_off(ch);

    // RR=200のリリースで1000サンプル分減衰させる（env_level: 1.0 → 約0.72）
    let mut release_buf = vec![0.0f32; 1000];
    engine.render(&mut release_buf, 1);
    let release_peak = release_buf[900..].iter().fold(0.0f32, |m, &s| m.max(s.abs()));

    // 残響再アタック。ARを最遅(ar=0)にして、env_levelがほぼ動かない状態で観測する。
    let mut slow_attack = sustained_release_patch();
    for op in slow_attack.operators.iter_mut() {
        op.ar = 0;
    }
    engine.set_patch(slow_attack);
    engine.note_on(ch, 440.0, 127);
    let mut after = vec![0.0f32; 500];
    engine.render(&mut after, 1);
    let after_peak = after.iter().fold(0.0f32, |m, &s| m.max(s.abs()));

    // 残響レベルを保持しているので、ar=0でも無音に落ちず直前のリリースレベル付近を維持する。
    assert!(
        after_peak > release_peak * 0.8,
        "note_on should re-attack from residual (not reset to silence): after_peak={after_peak}, release_peak={release_peak}"
    );
}

/// note_onは同じチャンネルIDへ再発音すると残響レベルから再アタックする。
/// 新しいAttackがフルレベルへ向かうため、リリース残響レベルを上回る。
#[test]
fn note_on_reattacks_toward_full_level() {
    let mut engine = Ym38x6Engine::new(44100.0);
    let ch = 0;
    engine.set_patch(sustained_release_patch());
    engine.note_on(ch, 440.0, 100);
    let mut warmup = vec![0.0f32; 100];
    engine.render(&mut warmup, 1);

    engine.note_off(ch);

    let mut release_buf = vec![0.0f32; 1000];
    engine.render(&mut release_buf, 1);
    let release_peak = release_buf[900..].iter().fold(0.0f32, |m, &s| m.max(s.abs()));

    engine.set_patch(sustained_release_patch());

    engine.note_on(ch, 440.0, 100);
    let mut after = vec![0.0f32; 500];
    engine.render(&mut after, 1);
    // 残響から再アタックし、フルレベルへ立ち上がるためリリースレベルを超える
    let after_peak = after[300..].iter().fold(0.0f32, |m, &s| m.max(s.abs()));

    assert!(
        after_peak > release_peak,
        "note_on should re-attack toward full level: after_peak={after_peak}, release_peak={release_peak}"
    );
}
