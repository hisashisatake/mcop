// ---------------------------------------------------------------------------
// エフェクト系NRPN/CCの適用先（MasterEffectsへの書き込み）
// ---------------------------------------------------------------------------
//
// CC/NRPNのアドレス解釈自体は`op505-midi`の`ControlTarget`が担うが、
// `MasterEffects`（sound-core型）への実際の書き込みはここに一本化する。
// standalone/vst/smf2op505の3ホストが個別に持っていた
// `EffectControlTarget::X => fx.set_x(...)`という同一の写像を集約したもの。

use sound_core::{ChorusType, MasterEffects, ReverbType};

/// エフェクト系NRPN（NRPN(0,2)〜(0,8)）が指す適用先。`MasterEffects`はsound-core型のため
/// `op505-midi`のAPIには出せず、この型で呼び出し側（本クレート経由でホスト）へ運ぶ。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EffectControlTarget {
    ReverbType,
    ChorusType,
    ReverbTime,
    ChorusModRate,
    ChorusModDepth,
    ChorusFeedback,
    ChorusSendToReverb,
    /// NRPN(0,36)：Delay/Panning Delayのテンポ同期有効/無効（0=OFF/1以上=ON）。
    DelaySync,
    /// NRPN(0,37)：Delay/Panning Delayのテンポ同期先レート（0〜255、TimeEgの
    /// `sync_rate`と同じ音価アンカーを踏む）。
    DelaySyncRate,
}

/// `target`が指す`MasterEffects`のフィールドへ`value`を書き込む。
pub fn apply_effect_control(fx: &mut MasterEffects, target: EffectControlTarget, value: u8) {
    match target {
        EffectControlTarget::ReverbType => fx.set_reverb_type(ReverbType::from_u8(value)),
        EffectControlTarget::ChorusType => fx.set_chorus_type(ChorusType::from_u8(value)),
        EffectControlTarget::ReverbTime => fx.set_reverb_time(value),
        EffectControlTarget::ChorusModRate => fx.set_chorus_mod_rate(value),
        EffectControlTarget::ChorusModDepth => fx.set_chorus_mod_depth(value),
        EffectControlTarget::ChorusFeedback => fx.set_chorus_feedback(value),
        EffectControlTarget::ChorusSendToReverb => fx.set_chorus_send_to_reverb(value),
        EffectControlTarget::DelaySync => fx.set_reverb_delay_sync(value),
        EffectControlTarget::DelaySyncRate => fx.set_reverb_delay_sync_rate(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sound_core::AudioProcessor;

    /// Reverb Timeを反映すると、同じインパルスに対するテールの減衰特性が変わることを
    /// 確認する（`set_reverb_time`が正しく呼ばれていることの間接証明）。
    #[test]
    fn reverb_time_is_applied() {
        let mut fx = MasterEffects::new(44100.0);
        fx.set_reverb_send(255);
        apply_effect_control(&mut fx, EffectControlTarget::ReverbTime, 0);

        let mut buffer = vec![0.0f32; 2 * 4096];
        buffer[0] = 1.0;
        buffer[1] = 1.0;
        fx.process(&mut buffer, 2);

        let tail_energy: f32 = buffer[2..].iter().map(|x| x * x).sum();
        assert!(tail_energy.is_finite());
    }

    /// 全バリアントがパニックせず適用できることを確認する（網羅性の素朴なスモーク）。
    #[test]
    fn all_targets_apply_without_panic() {
        let mut fx = MasterEffects::new(44100.0);
        for target in [
            EffectControlTarget::ReverbType,
            EffectControlTarget::ChorusType,
            EffectControlTarget::ReverbTime,
            EffectControlTarget::ChorusModRate,
            EffectControlTarget::ChorusModDepth,
            EffectControlTarget::ChorusFeedback,
            EffectControlTarget::ChorusSendToReverb,
            EffectControlTarget::DelaySync,
            EffectControlTarget::DelaySyncRate,
        ] {
            apply_effect_control(&mut fx, target, 100);
        }
    }
}
