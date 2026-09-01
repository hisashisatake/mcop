use op505_core::{lfo_rate_to_hz, Op505Patch};

use crate::value::cc_byte_to_u8;

/// Cutoff FGの演奏用フォールバックを`patch`へ適用する（`apply_pitch_fg_expression`/
/// `apply_gain_fg_expression`と同じ「note_patchへの後処理」）。
///
/// Cutoff FGにはPitch FG(CC1/CC77)・Gain FG(CC92)のような専用CCが無く、深さは
/// NRPN(0,26)の絶対上書き（`PatchOverrides::cutoff_fg_depth`、`build_effective_patch`で
/// 本関数より先に適用済み）でのみ動く。プリセットが形を持たない（`stage_count==0`）まま
/// NRPN(0,26)でdepthが0より大きくなったときは標準オートワウ形状
/// （`op505_core::standard_bipolar_modulation_eg`）を書き込む。速さはGain FGと同じくCC76
/// （Pitch FGと共有するチャンネルのシャドウ値）から`lfo_rate_to_hz`でHzを求め
/// 段のtimeへ直接焼き込む（Cutoff FGにも`rate_scale`APIが無いため）。NRPN(0,26)未送信なら
/// 発火せず既存プリセットは出力ビット不変。
pub fn apply_cutoff_fg_expression(patch: &mut Op505Patch, pitch_fg_cc76: u8) {
    let depth = patch.channel.cutoff_fg.depth;
    if patch.channel.cutoff_fg.eg.stage_count == 0 && depth > 0 {
        let hz = lfo_rate_to_hz(cc_byte_to_u8(pitch_fg_cc76));
        let half_period = 0.5 / hz;
        patch.channel.cutoff_fg.eg = op505_core::standard_bipolar_modulation_eg(0.0, half_period, half_period);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// フォールバック：`stage_count==0`かつdepth==0（既定値）なら何も変わらない。
    #[test]
    fn fallback_noop_when_depth_zero() {
        let mut patch = Op505Patch::default();
        assert_eq!(patch.channel.cutoff_fg.eg.stage_count, 0);
        assert_eq!(patch.channel.cutoff_fg.depth, 0);
        let before = patch;
        apply_cutoff_fg_expression(&mut patch, 64);
        assert_eq!(patch, before);
    }

    /// フォールバック：`stage_count==0`かつdepth>0（NRPN(0,26)適用後を想定）で
    /// 標準オートワウ形状を書き込む。
    #[test]
    fn fallback_materializes_standard_shape_when_depth_positive() {
        let mut patch = Op505Patch::default();
        patch.channel.cutoff_fg.depth = 120;
        apply_cutoff_fg_expression(&mut patch, 64);
        assert_eq!(patch.channel.cutoff_fg.eg.stage_count, 4);
        assert_eq!(patch.channel.cutoff_fg.depth, 120, "depthは既にNRPN由来の値のまま変えない");
    }

    /// 既に形を持つプリセット（`stage_count>0`）はフォールバックが発火せず不変。
    #[test]
    fn no_fallback_when_preset_already_has_shape() {
        let mut patch = Op505Patch::default();
        patch.channel.cutoff_fg.depth = 120;
        patch.channel.cutoff_fg.eg.stage_count = 2;
        let before = patch;
        apply_cutoff_fg_expression(&mut patch, 64);
        assert_eq!(patch, before);
    }
}
