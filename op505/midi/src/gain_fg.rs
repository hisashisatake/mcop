use op505_core::Op505Patch;

/// CC92(Tremolo Depth)によるGain FGの演奏補正を`patch`へ適用する
/// （`apply_pitch_fg_expression`と同じ「note_patchへの後処理」）。
///
/// Gain FGのDepthはPitch/Cutoff FGと違いRPN(0,5)のような可変レンジを持たない
/// （標準MIDIのCC92にモジュレーションレンジRPNが存在しないため）。CC92は0起点の単純加算で、
/// NRPN(0,27)の絶対上書き（`PatchOverrides::gain_fg_depth`）の上にさらに加算される。
pub fn apply_gain_fg_expression(patch: &mut Op505Patch, cc92: u8) {
    let base_depth = patch.channel.gain_fg.depth as i32;
    patch.channel.gain_fg.depth = (base_depth + cc92 as i32).clamp(0, 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc92_zero_leaves_base_depth_unchanged() {
        let mut patch = Op505Patch::default();
        patch.channel.gain_fg.depth = 100;
        apply_gain_fg_expression(&mut patch, 0);
        assert_eq!(patch.channel.gain_fg.depth, 100);
    }

    #[test]
    fn cc92_adds_onto_base_depth_and_clamps() {
        let mut patch = Op505Patch::default();
        patch.channel.gain_fg.depth = 100;
        apply_gain_fg_expression(&mut patch, 50);
        assert_eq!(patch.channel.gain_fg.depth, 150);

        patch.channel.gain_fg.depth = 240;
        apply_gain_fg_expression(&mut patch, 255);
        assert_eq!(patch.channel.gain_fg.depth, 255, "0〜255へクランプされるはず");
    }
}
