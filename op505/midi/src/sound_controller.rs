//! MIDI CC10（Pan）/ CC71（Resonance）/ CC72（Release Time）/ CC73（Attack Time）/
//! CC74（Brightness）/ CC75（Decay Time）の解釈（spec-sound.md「MIDI CC（GM2準拠）」節）。
//!
//! CC2/CC4/AT（[`crate::expression`]）は「行先を選べるユニポーラ加算」だが、こちらは
//! GM2で意味が固定された「64中心のバイポーラ補正」（CC71/74）と「保持区間をピーク検出で
//! Attack/Decayへ分割する時間スケール」（CC72/73/75）で意味論が異なるため、別モジュールにする。
//! 全て既定64（無補正）のときは`patch`を一切変更しない（既存パッチの出力をビット単位で
//! 不変に保つ）。

use op505_core::{cc_to_time_scale, Op505Patch};
use sound_fm::algorithm::ALGORITHMS;

/// CC71/CC74（64中心のバイポーラ補正）の加算量。64→0、0→-128、127→+126の線形写像。
/// `(cc-64)*2`という素朴な式だが、64ではi16計算を経ずとも常に0になる。
fn bipolar_delta(cc: u8) -> i16 {
    (cc as i16 - 64) * 2
}

/// `base`へ`bipolar_delta(cc)`を加算し0〜255へclampする。`cc==64`は`delta==0`のため
/// `base`をそのまま返す（ビット不変ガード）。
fn add_bipolar(base: u8, cc: u8) -> u8 {
    if cc == 64 {
        return base;
    }
    (base as i16 + bipolar_delta(cc)).clamp(0, 255) as u8
}

/// CC71(Resonance)/CC74(Brightness)/CC72/73/75(Release/Attack/Decay Time)を`patch`へ適用する
/// （[`crate::expression::apply_expression_modulation`]と同じ「note_patchへの後処理」）。
/// 全て既定64（無補正）ならこの関数は`patch`を一切変更しない。
/// CC10(Pan)はパッチではなくボイス単位のゲイン（`Vco::set_channel_pan`）で適用するため対象外
/// （[`crate::channel_state::ChannelState::pan_gains`]参照）。
pub fn apply_sound_controllers(
    patch: &mut Op505Patch,
    cc71_resonance: u8,
    cc72_release: u8,
    cc73_attack: u8,
    cc74_brightness: u8,
    cc75_decay: u8,
) {
    patch.channel.filter_resonance = add_bipolar(patch.channel.filter_resonance, cc71_resonance);
    patch.channel.filter_cutoff = add_bipolar(patch.channel.filter_cutoff, cc74_brightness);

    let attack_scale = cc_to_time_scale(cc73_attack);
    let decay_scale = cc_to_time_scale(cc75_decay);
    let release_scale = cc_to_time_scale(cc72_release);
    if attack_scale != 1.0 || decay_scale != 1.0 || release_scale != 1.0 {
        for &i in ALGORITHMS[patch.channel.algorithm as usize].carriers {
            patch.operators[i].eg.scale_section_times(attack_scale, decay_scale, release_scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_neutral_is_noop() {
        let mut patch = Op505Patch::default();
        let original = patch;
        apply_sound_controllers(&mut patch, 64, 64, 64, 64, 64);
        assert_eq!(patch, original, "全て64(無補正)ならpatchを一切変更しないはず");
    }

    #[test]
    fn cc71_raises_and_lowers_resonance() {
        let mut patch = Op505Patch::default();
        patch.channel.filter_resonance = 100;

        let mut raised = patch;
        apply_sound_controllers(&mut raised, 127, 64, 64, 64, 64);
        assert_eq!(raised.channel.filter_resonance, (100 + 126).min(255) as u8);

        let mut lowered = patch;
        apply_sound_controllers(&mut lowered, 0, 64, 64, 64, 64);
        assert_eq!(lowered.channel.filter_resonance, 100i16.saturating_sub(128).max(0) as u8);
    }

    #[test]
    fn cc74_raises_and_lowers_cutoff() {
        let mut patch = Op505Patch::default();
        patch.channel.filter_cutoff = 100;

        let mut raised = patch;
        apply_sound_controllers(&mut raised, 64, 64, 64, 127, 64);
        assert_eq!(raised.channel.filter_cutoff, (100 + 126).min(255) as u8);

        let mut lowered = patch;
        apply_sound_controllers(&mut lowered, 64, 64, 64, 0, 64);
        assert_eq!(lowered.channel.filter_cutoff, 100i16.saturating_sub(128).max(0) as u8);
    }

    #[test]
    fn cc71_74_clamp_at_bounds() {
        let mut near_max = Op505Patch::default();
        near_max.channel.filter_resonance = 250;
        near_max.channel.filter_cutoff = 250;
        apply_sound_controllers(&mut near_max, 127, 64, 64, 127, 64);
        assert_eq!(near_max.channel.filter_resonance, 255);
        assert_eq!(near_max.channel.filter_cutoff, 255);

        let mut near_min = Op505Patch::default();
        near_min.channel.filter_resonance = 5;
        near_min.channel.filter_cutoff = 5;
        apply_sound_controllers(&mut near_min, 0, 64, 64, 0, 64);
        assert_eq!(near_min.channel.filter_resonance, 0);
        assert_eq!(near_min.channel.filter_cutoff, 0);
    }

    #[test]
    fn cc73_scales_carrier_attack_time_only() {
        let mut patch = Op505Patch::default();
        patch.channel.algorithm = 7; // 全OP並列＝全4opがキャリア
        for op in patch.operators.iter_mut() {
            // `TimeStage`は`op505-core`が再エクスポートしていない（構造体リテラル構築不可）ため、
            // 既存値（`Default`）のフィールドを直接書き換える。
            op.eg.stages[0].time = 100;
            op.eg.stages[0].level = 255;
            op.eg.stage_count = 1;
            op.eg.release_point = 0;
        }
        let mut expected_eg = patch.operators[0].eg;
        expected_eg.scale_section_times(cc_to_time_scale(127), 1.0, 1.0);

        apply_sound_controllers(&mut patch, 64, 64, 127, 64, 64);

        for op in patch.operators.iter() {
            assert_eq!(op.eg, expected_eg);
        }
    }
}
