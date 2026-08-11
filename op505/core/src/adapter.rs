// ---------------------------------------------------------------------------
// Adapter: 既存.38x6（レート方式5段EG、`ym38x6_core::Ym38x6Patch`）を
// op505のN点Time/Level方式（`Op505Patch`）へ変換する。
//
// EG形状変換の本体（`convert_eg_shape`等、ym38x6非依存）は`crate::eg_convert`へ
// 移設済み（op505/デフォーク計画Phase 4）。このファイルは`Ym38x6Patch`の
// フィールドを読んで`eg_convert`の関数群へ渡す「配線」だけを担う。
// ---------------------------------------------------------------------------

use ym38x6_core::{OperatorParams, Ym38x6Patch};

use crate::eg_convert::{apply_transparent_gain_release, convert_eg_shape, prepend_delay_stage};
use crate::{Op505BipolarFg, Op505ChannelParams, Op505OperatorParams, Op505Patch};

/// オペレーターEG（`OperatorParams`のar/d1r/d1l/d2r/rr/floor/loop/curve）をTimeEgParamsへ変換する。
/// 警告が必要な場合は`convert_patch`経由で確認すること（この関数は静かにクランプする）。
pub fn convert_operator_eg(op: &OperatorParams) -> sound_core::TimeEgParams {
    let mut warnings = Vec::new();
    convert_eg_shape(op.ar, op.d1r, op.d1l, op.d2r, op.rr, op.floor, op.loop_enabled, op.curve, &mut warnings, "op")
}

/// `.38x6`（`Ym38x6Patch`）をop505の`Op505Patch`へ変換する。クランプ・特殊ケース変換が
/// 発生した箇所は警告としてラベル付きで返す（例: `"op2: decay2(d2r) 45.3秒は..."`）。
pub fn convert_patch(src: &Ym38x6Patch) -> (Op505Patch, Vec<String>) {
    let mut warnings = Vec::new();

    let operators = std::array::from_fn(|i| {
        let src_op = &src.operators[i];
        let eg = convert_eg_shape(
            src_op.ar,
            src_op.d1r,
            src_op.d1l,
            src_op.d2r,
            src_op.rr,
            src_op.floor,
            src_op.loop_enabled,
            src_op.curve,
            &mut warnings,
            &format!("op{i}"),
        );
        Op505OperatorParams {
            tl: src_op.tl,
            eg,
            mul: src_op.mul,
            dt1: src_op.dt1,
            ksr: src_op.ksr,
            am_enable: src_op.am_enable,
            velocity_sensitivity: src_op.velocity_sensitivity,
            waveform: src_op.waveform,
            op_fine_tune: src_op.op_fine_tune,
            eg_shift: src_op.eg_shift,
            level_scale: src_op.level_scale,
            velocity_gain: src_op.velocity_gain,
        }
    });

    let ch = &src.channel;

    let mut pitch_fg_eg = convert_eg_shape(
        ch.pitch_fg.eg.ar,
        ch.pitch_fg.eg.d1r,
        ch.pitch_fg.eg.d1l,
        ch.pitch_fg.eg.d2r,
        ch.pitch_fg.eg.rr,
        ch.pitch_fg.eg.floor,
        ch.pitch_fg.eg.loop_enabled,
        ch.pitch_fg.eg.curve,
        &mut warnings,
        "pitch_fg",
    );
    if ch.pitch_fg.eg.delay > 0 {
        prepend_delay_stage(&mut pitch_fg_eg, ch.pitch_fg.eg.delay, &mut warnings, "pitch_fg");
    }

    let mut cutoff_fg_eg = convert_eg_shape(
        ch.cutoff_fg.eg.ar,
        ch.cutoff_fg.eg.d1r,
        ch.cutoff_fg.eg.d1l,
        ch.cutoff_fg.eg.d2r,
        ch.cutoff_fg.eg.rr,
        ch.cutoff_fg.eg.floor,
        ch.cutoff_fg.eg.loop_enabled,
        ch.cutoff_fg.eg.curve,
        &mut warnings,
        "cutoff_fg",
    );
    if ch.cutoff_fg.eg.delay > 0 {
        prepend_delay_stage(&mut cutoff_fg_eg, ch.cutoff_fg.eg.delay, &mut warnings, "cutoff_fg");
    }

    let mut gain_fg_eg = convert_eg_shape(
        ch.gain_fg.ar,
        ch.gain_fg.d1r,
        ch.gain_fg.d1l,
        ch.gain_fg.d2r,
        ch.gain_fg.rr,
        ch.gain_fg.floor,
        ch.gain_fg.loop_enabled,
        ch.gain_fg.curve,
        &mut warnings,
        "gain_fg",
    );
    if ch.gain_fg.delay > 0 {
        prepend_delay_stage(&mut gain_fg_eg, ch.gain_fg.delay, &mut warnings, "gain_fg");
    }
    apply_transparent_gain_release(&mut gain_fg_eg, ch.gain_fg.rr, ch.gain_fg.loop_enabled, &mut warnings);

    let channel = Op505ChannelParams {
        algorithm: ch.algorithm,
        feedback: ch.feedback,
        chip_lfo_freq: ch.chip_lfo_freq,
        chip_lfo_pmd: ch.chip_lfo_pmd,
        chip_lfo_amd: ch.chip_lfo_amd,
        chip_lfo_delay: ch.chip_lfo_delay,
        pms: ch.pms,
        ams: ch.ams,
        filter_cutoff: ch.filter_cutoff,
        filter_resonance: ch.filter_resonance,
        filter_type: ch.filter_type,
        filter_self_oscillation: ch.filter_self_oscillation,
        pitch_fg: Op505BipolarFg { eg: pitch_fg_eg, depth: ch.pitch_fg.depth },
        cutoff_fg: Op505BipolarFg { eg: cutoff_fg_eg, depth: ch.cutoff_fg.depth },
        gain_fg: gain_fg_eg,
        texture_lfo: ch.texture_lfo,
    };

    (Op505Patch { operators, channel }, warnings)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sound_core::TimeEg;

    #[test]
    fn convert_patch_gain_fg_rr_zero_stays_transparent_after_release() {
        let src = Ym38x6Patch::default(); // channel.gain_fgは既定でrr=0（透過既定）
        let (patch, warnings) = convert_patch(&src);
        assert!(warnings.iter().any(|w| w.contains("gain_fg") && w.contains("透過")));

        let sr = 44100.0;
        let mut eg = TimeEg::new();
        eg.note_on();
        for _ in 0..1000 {
            eg.tick(sr, patch.channel.gain_fg, 1.0);
        }
        let before = eg.tick(sr, patch.channel.gain_fg, 1.0);
        eg.note_off();
        let mut after = before;
        for _ in 0..44100 {
            after = eg.tick(sr, patch.channel.gain_fg, 1.0);
        }
        assert!(
            (before - after).abs() < 1e-6,
            "gain_fg rr=0 should stay transparent (not close the gate) after note_off: before={before} after={after}"
        );
    }

    #[test]
    fn convert_patch_default_round_trips_through_serde() {
        let src = Ym38x6Patch::default();
        let (patch, _warnings) = convert_patch(&src);
        let json = serde_json::to_string(&patch).expect("serialize");
        let restored: Op505Patch = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(patch, restored);
    }
}
