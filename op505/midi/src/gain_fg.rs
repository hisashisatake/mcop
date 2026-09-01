use op505_core::{lfo_rate_to_hz, Op505Patch};

use crate::value::cc_byte_to_u8;

/// CC92(Tremolo Depth)によるGain FGの演奏補正を`patch`へ適用する
/// （`apply_pitch_fg_expression`と同じ「note_patchへの後処理」）。
///
/// Gain FGのDepthはPitch/Cutoff FGと違いRPN(0,5)のような可変レンジを持たない
/// （標準MIDIのCC92にモジュレーションレンジRPNが存在しないため）。CC92は0起点の単純加算で、
/// NRPN(0,27)の絶対上書き（`PatchOverrides::gain_fg_depth`）の上にさらに加算される。
///
/// **フォールバック**: プリセットが形を持たない（`stage_count==0`）まま、Gainの既定`depth=255`は
/// 「無効化中の無意味な値」（spec-sound.md「TimeEgのFG無効化」節）なのでCC92加算前に捨て、
/// CC92そのものを新しいdepthにする。CC92>0のときだけ標準トレモロ形状
/// （`op505_core::standard_tremolo_gain_eg`）を書き込む。速さはGain FGに`rate_scale`APIが
/// 無いため、CC76(Vibrato Rate、Pitch FGと共有するチャンネルのシャドウ値)から
/// `lfo_rate_to_hz`でHzを求め段のtimeへ直接焼き込む。CC92未送信なら発火せず
/// 既存プリセットは出力ビット不変。
pub fn apply_gain_fg_expression(patch: &mut Op505Patch, cc92: u8, pitch_fg_cc76: u8) {
    if patch.channel.gain_fg.eg.stage_count == 0 {
        if cc92 > 0 {
            let hz = lfo_rate_to_hz(cc_byte_to_u8(pitch_fg_cc76));
            patch.channel.gain_fg.eg = op505_core::standard_tremolo_gain_eg(0.0, hz);
            patch.channel.gain_fg.depth = cc92;
        }
        return;
    }

    let base_depth = patch.channel.gain_fg.depth as i32;
    patch.channel.gain_fg.depth = (base_depth + cc92 as i32).clamp(0, 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc92_zero_leaves_base_depth_unchanged() {
        let mut patch = Op505Patch::default();
        patch.channel.gain_fg.eg.stage_count = 1;
        patch.channel.gain_fg.depth = 100;
        apply_gain_fg_expression(&mut patch, 0, 64);
        assert_eq!(patch.channel.gain_fg.depth, 100);
    }

    #[test]
    fn cc92_adds_onto_base_depth_and_clamps() {
        let mut patch = Op505Patch::default();
        patch.channel.gain_fg.eg.stage_count = 1;
        patch.channel.gain_fg.depth = 100;
        apply_gain_fg_expression(&mut patch, 50, 64);
        assert_eq!(patch.channel.gain_fg.depth, 150);

        patch.channel.gain_fg.depth = 240;
        apply_gain_fg_expression(&mut patch, 255, 64);
        assert_eq!(patch.channel.gain_fg.depth, 255, "0〜255へクランプされるはず");
    }

    /// フォールバック：既定プリセット（`stage_count==0`）でCC92未送信なら何も変わらない
    /// （既定`depth=255`という「無効化中の無意味な値」もそのまま、出力ビット不変の根拠）。
    #[test]
    fn fallback_noop_when_cc92_zero() {
        let mut patch = Op505Patch::default();
        assert_eq!(patch.channel.gain_fg.eg.stage_count, 0);
        let before = patch;
        apply_gain_fg_expression(&mut patch, 0, 64);
        assert_eq!(patch, before);
    }

    /// フォールバック：`stage_count==0`かつCC92>0で標準トレモロ形状を書き込み、
    /// depthはCC92そのもの（既定255のbaseは捨てる）。
    #[test]
    fn fallback_materializes_standard_shape_when_cc92_positive() {
        let mut patch = Op505Patch::default();
        apply_gain_fg_expression(&mut patch, 80, 64);
        assert_eq!(patch.channel.gain_fg.eg.stage_count, 5);
        assert_eq!(patch.channel.gain_fg.depth, 80);
    }
}
