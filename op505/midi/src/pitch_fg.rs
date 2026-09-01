use op505_core::Op505Patch;

/// CC1(モジュレーションホイール)・CC77(Vibrato Depth)・CC78(Vibrato Delay)によるPitch FGの
/// ②③層補正を`patch`へ適用する（spec-sound.md「演奏層による補正」節）。`apply_expression_modulation`/
/// `apply_soft_pedal`と同じ「note_patchへの後処理」として、`base_patch`がDAWパラメーター由来か
/// Program Change由来かに関わらず一律に効かせる想定（呼び出し側の合成順序に依存しない）。
/// CC76(Rate)は`sound_core::cc76_to_rate_scale`という別経路（`set_pitch_fg_rate_scale`）のため
/// この関数の対象外。
///
/// **フォールバック**: プリセットが形を持たない（`stage_count==0`）まま、CC1/CC77でdepthが
/// 0より大きくなったときは標準ビブラート形状（`op505_core::standard_bipolar_modulation_eg`）を
/// `pitch_fg.eg`へ書き込む。モジュレーションホイールを回してもEGの形が無ければ変調が
/// 一切効かない、という製品の穴を塞ぐ（詳細はspec-sound.md「演奏用FGフォールバック」節）。
/// 既定プリセット（`depth=0`）はCCを送らない限りこの分岐へ入らないため出力ビット不変。
pub fn apply_pitch_fg_expression(patch: &mut Op505Patch, cc1: u8, cc77: u8, cc78: u8, rpn0_5: u8) {
    // CC1のセント換算分をDepthと同じ0〜255単位空間へ逆変換して加算する
    // （Pitch FGの`depth/255*1200`セント変換式の逆算、cc1_cents = cc1/127 * rpn0_5*50/64）。
    // Depthは符号を持たない強度（0＝変調なし）なので、CC1/CC77はそのまま強度への加算になる
    // （旧バイポーラDepth時代は中心128からの加算で、負方向Depthのパッチだと中心へ
    // 引き戻してしまう歪みがあった）。
    let cc1_cents = (cc1 as f32 / 127.0) * (rpn0_5 as f32 * 50.0 / 64.0);
    let cc1_depth_units = (cc1_cents / 1200.0 * 255.0).round() as i32;
    let base_depth = patch.channel.pitch_fg.depth as i32;
    let depth = (base_depth + cc77 as i32 + cc1_depth_units).clamp(0, 255) as u8;

    if patch.channel.pitch_fg.eg.stage_count == 0 && depth > 0 {
        patch.channel.pitch_fg.eg = op505_core::standard_bipolar_modulation_eg(
            0.0,
            op505_core::STANDARD_VIBRATO_HALF_PERIOD_SECONDS,
            op505_core::STANDARD_VIBRATO_HALF_PERIOD_SECONDS,
        );
    }
    patch.channel.pitch_fg.depth = depth;

    // CC78(Vibrato Delay)：TimeEgにDelayフィールドが無いため、Pitch FGの第0段が
    // 「無変調のまま待つ段」であるときに限り、その段のtimeへ(CC78-64)を加算してDelay相当とする
    // （TimeEgではDelayを「変調量0の段」で表現するのが自然なため）。バイポーラレベルでは
    // 無変調＝中央128なので、その値で待ち段を判定する。第0段が中央以外
    // （＝いきなり振れ始める形）のときは対応する概念が無いので何もしない。
    let stage0 = &mut patch.channel.pitch_fg.eg.stages[0];
    if stage0.level == op505_core::BIPOLAR_NEUTRAL_RAW {
        let delay_delta = cc78 as i32 - 64;
        let adjusted = stage0.time as i32 + delay_delta;
        stage0.time = adjusted.clamp(0, 255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// フォールバック：既定プリセット（`stage_count==0`・`depth=0`）でCC1/CC77未送信なら
    /// 何も変わらない（出力ビット不変の根拠）。
    #[test]
    fn fallback_noop_when_depth_zero() {
        let mut patch = Op505Patch::default();
        assert_eq!(patch.channel.pitch_fg.eg.stage_count, 0);
        let before = patch;
        apply_pitch_fg_expression(&mut patch, 0, 0, 64, 64);
        assert_eq!(patch, before);
    }

    /// フォールバック：`stage_count==0`のままCC77でdepthが正になったら標準ビブラート形状を書き込む。
    #[test]
    fn fallback_materializes_standard_shape_when_cc77_positive() {
        let mut patch = Op505Patch::default();
        apply_pitch_fg_expression(&mut patch, 0, 80, 64, 64);
        assert_eq!(patch.channel.pitch_fg.eg.stage_count, 4);
        assert_eq!(patch.channel.pitch_fg.depth, 80);
    }

    /// 既に形を持つプリセット（`stage_count>0`）はフォールバックが発火せず既存EGを保つ。
    #[test]
    fn no_fallback_when_preset_already_has_shape() {
        let mut patch = Op505Patch::default();
        patch.channel.pitch_fg.eg.stage_count = 2;
        let eg_before = patch.channel.pitch_fg.eg;
        apply_pitch_fg_expression(&mut patch, 0, 80, 64, 64);
        assert_eq!(patch.channel.pitch_fg.eg, eg_before, "既存の形はフォールバックで上書きされない");
        assert_eq!(patch.channel.pitch_fg.depth, 80);
    }

    /// CC78(Delay)はmaterialize後の第0段（中央レベル）へも従来通り効く。
    #[test]
    fn cc78_delay_applies_after_materialize() {
        let mut patch = Op505Patch::default();
        apply_pitch_fg_expression(&mut patch, 0, 80, 127, 64); // cc78=127 -> delay_delta=+63
        assert_eq!(patch.channel.pitch_fg.eg.stages[0].time, 63);
    }
}
