use op505_core::Op505Patch;

/// CC1(モジュレーションホイール)・CC77(Vibrato Depth)・CC78(Vibrato Delay)によるPitch FGの
/// ②③層補正を`patch`へ適用する（spec-sound.md「演奏層による補正」節）。`apply_expression_modulation`/
/// `apply_soft_pedal`と同じ「note_patchへの後処理」として、`base_patch`がDAWパラメーター由来か
/// Program Change由来かに関わらず一律に効かせる想定（呼び出し側の合成順序に依存しない）。
/// CC76(Rate)は`sound_core::cc76_to_rate_scale`という別経路（`set_pitch_fg_rate_scale`）のため
/// この関数の対象外。
pub fn apply_pitch_fg_expression(patch: &mut Op505Patch, cc1: u8, cc77: u8, cc78: u8, rpn0_5: u8) {
    // CC1のセント換算分をDepthと同じ0〜255単位空間へ逆変換して加算する
    // （Pitch FGの`(depth-128)/128*1200`セント変換式の逆算、cc1_cents = cc1/127 * rpn0_5*50/64）。
    let cc1_cents = (cc1 as f32 / 127.0) * (rpn0_5 as f32 * 50.0 / 64.0);
    let cc1_depth_units = (cc1_cents / 1200.0 * 128.0).round() as i32;
    let base_depth = patch.channel.pitch_fg.depth as i32;
    patch.channel.pitch_fg.depth = (base_depth + cc77 as i32 + cc1_depth_units).clamp(0, 255) as u8;

    // CC78(Vibrato Delay)：TimeEgにDelayフィールドが無いため、Pitch FGの第0段が
    // 「level=0の待ち段」であるときに限り、その段のtimeへ(CC78-64)を加算してDelay相当とする
    // （TimeEgではDelayを「level=0の段」で表現するのが自然なため）。第0段がlevel>0
    // （＝いきなり立ち上がる形）のときは対応する概念が無いので何もしない。
    let stage0 = &mut patch.channel.pitch_fg.eg.stages[0];
    if stage0.level == 0 {
        let delay_delta = cc78 as i32 - 64;
        let adjusted = stage0.time as i32 + delay_delta;
        stage0.time = adjusted.clamp(0, 255) as u8;
    }
}
