use crate::rpn::RpnSelection;

/// RPN/NRPN選択状態が指し示す制御対象。CC6(Data Entry MSB)が届いたときに実際どのパラメーターへ
/// 書き込むべきかを解決する（VST/smf2op505が独立に大きな`match`を持つと解釈がすれるため、
/// アドレス表をこの一箇所に集約する）。
///
/// 利用側は`_ =>`を書かず全バリアントを列挙すること。バリアント追加時に呼び出し側も
/// コンパイルエラーになり「片方だけ実装して解釈がすれる」が構造的に起きなくなる。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ControlTarget {
    /// 未割り当てのRPN/NRPN、または選択解除中。
    /// discriminant=0（先頭）に置く。この値自体はシリアライズされず`control_target()`が
    /// 毎回その場で計算する内部ディスパッチ用の値なので数値に意味は無いが、将来
    /// バリアントを追加する際に「無効値が末尾にある」形（2026-08-18に`FmLfoDestination`で
    /// 実際に問題になった並び）を再現しないよう、先頭へ寄せてある。
    Unassigned,
    /// RPN(0,0): Pitch Bend Sensitivity（半音）
    PitchBendRange,
    /// RPN(0,5): Modulation Depth Range
    ModulationDepthRange,
    /// NRPN(0,0)・(0,22)〜(0,27)。旧質感LFO(Destination/Waveform/FadeMode/Rate/Depth/
    /// Delay/FadeTime/Offset)のアドレス。質感LFO退役（`TimeEgParams::texture`へ統合、
    /// memory `project_texture_lfo_retirement.md`参照）に伴い**欠番として予約**し、再利用しない
    /// （`ReservedFgLoopCurve`と同じ理由：既存のSMF/DAWオートメーションが別の意味で
    /// 解釈されるのを防ぐため）。NRPN(0,1)は`ChannelEffectRoute`へ再割り当て済みのため
    /// このバリアントの対象外。
    ReservedTextureLfo,
    /// NRPN(0,1): Channel Effect Route。送信チャンネル自身の音声・エフェクト設定NRPN(0,2)〜
    /// (0,8)・CC91/93の適用先エフェクトスロット番号（0〜`EFFECT_SLOT_COUNT - 1`）を設定する。
    /// 質感LFO退役で空いた欠番の再利用（詳細はspec-fm.md 8章）。
    ChannelEffectRoute,
    /// NRPN(0,2): Reverb Type
    ReverbType,
    /// NRPN(0,3): Chorus Type
    ChorusType,
    /// NRPN(0,4): Reverb Time
    ReverbTime,
    /// NRPN(0,5): Chorus Mod Rate
    ChorusModRate,
    /// NRPN(0,6): Chorus Mod Depth
    ChorusModDepth,
    /// NRPN(0,7): Chorus Feedback
    ChorusFeedback,
    /// NRPN(0,8): Chorus Send To Reverb
    ChorusSendToReverb,
    /// NRPN(0,9): Algorithm
    Algorithm,
    /// NRPN(0,10)〜(0,13): Waveform Op0〜3（引数はOpインデックス0〜3）
    OperatorWaveform(u8),
    /// NRPN(0,14): Filter Type
    FilterType,
    /// NRPN(0,15): Filter Self-Oscillation
    FilterSelfOscillation,
    /// NRPN(0,16): AT Destination
    AtDestination,
    /// NRPN(0,17): Poly AT Destination
    PolyAtDestination,
    /// NRPN(0,18)〜(0,21): Operator F-Number Op0〜3（引数はOpインデックス0〜3）
    OperatorFNumber(u8),
    /// NRPN(0,28)〜(0,33)。op505のTimeEg 7本はpersist状態のためNRPNから触ると
    /// GUI表示と実音がズレる。**欠番として予約**（ym38x6版FG Loop/Curve相当）。
    ReservedFgLoopCurve,
    /// NRPN(0,34): CC2(ブレス)Destination
    Cc2Destination,
    /// NRPN(0,35): CC4(フット)Destination
    Cc4Destination,
}

/// RPN/NRPN選択状態から制御対象を解決する。
pub fn control_target(selection: RpnSelection) -> ControlTarget {
    match selection {
        RpnSelection::None => ControlTarget::Unassigned,
        RpnSelection::Rpn(0, 0) => ControlTarget::PitchBendRange,
        RpnSelection::Rpn(0, 5) => ControlTarget::ModulationDepthRange,
        RpnSelection::Rpn(_, _) => ControlTarget::Unassigned,
        RpnSelection::Nrpn(0, 0) => ControlTarget::ReservedTextureLfo,
        RpnSelection::Nrpn(0, 1) => ControlTarget::ChannelEffectRoute,
        RpnSelection::Nrpn(0, 2) => ControlTarget::ReverbType,
        RpnSelection::Nrpn(0, 3) => ControlTarget::ChorusType,
        RpnSelection::Nrpn(0, 4) => ControlTarget::ReverbTime,
        RpnSelection::Nrpn(0, 5) => ControlTarget::ChorusModRate,
        RpnSelection::Nrpn(0, 6) => ControlTarget::ChorusModDepth,
        RpnSelection::Nrpn(0, 7) => ControlTarget::ChorusFeedback,
        RpnSelection::Nrpn(0, 8) => ControlTarget::ChorusSendToReverb,
        RpnSelection::Nrpn(0, 9) => ControlTarget::Algorithm,
        RpnSelection::Nrpn(0, lsb @ 10..=13) => ControlTarget::OperatorWaveform(lsb - 10),
        RpnSelection::Nrpn(0, 14) => ControlTarget::FilterType,
        RpnSelection::Nrpn(0, 15) => ControlTarget::FilterSelfOscillation,
        RpnSelection::Nrpn(0, 16) => ControlTarget::AtDestination,
        RpnSelection::Nrpn(0, 17) => ControlTarget::PolyAtDestination,
        RpnSelection::Nrpn(0, lsb @ 18..=21) => ControlTarget::OperatorFNumber(lsb - 18),
        RpnSelection::Nrpn(0, 22..=27) => ControlTarget::ReservedTextureLfo,
        RpnSelection::Nrpn(0, 28..=33) => ControlTarget::ReservedFgLoopCurve,
        RpnSelection::Nrpn(0, 34) => ControlTarget::Cc2Destination,
        RpnSelection::Nrpn(0, 35) => ControlTarget::Cc4Destination,
        RpnSelection::Nrpn(_, _) => ControlTarget::Unassigned,
    }
}

/// この制御対象への書き込みが、発音中ボイスへの即時反映（次の定期同期を待たない伝播）を
/// 必要とするかどうか。Operator F-Numberは`build_patch`の毎ブロック同期経路に乗らない
/// 専用APIのため、この値がtrueの間は呼び出し側が発音中チャンネルを直接走査して書き込む。
pub fn needs_voice_update(target: ControlTarget) -> bool {
    match target {
        ControlTarget::OperatorFNumber(_) => true,
        ControlTarget::Unassigned
        | ControlTarget::PitchBendRange
        | ControlTarget::ModulationDepthRange
        | ControlTarget::ReservedTextureLfo
        | ControlTarget::ChannelEffectRoute
        | ControlTarget::ReverbType
        | ControlTarget::ChorusType
        | ControlTarget::ReverbTime
        | ControlTarget::ChorusModRate
        | ControlTarget::ChorusModDepth
        | ControlTarget::ChorusFeedback
        | ControlTarget::ChorusSendToReverb
        | ControlTarget::Algorithm
        | ControlTarget::OperatorWaveform(_)
        | ControlTarget::FilterType
        | ControlTarget::FilterSelfOscillation
        | ControlTarget::AtDestination
        | ControlTarget::PolyAtDestination
        | ControlTarget::ReservedFgLoopCurve
        | ControlTarget::Cc2Destination
        | ControlTarget::Cc4Destination => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_indices_are_zero_based() {
        assert_eq!(control_target(RpnSelection::Nrpn(0, 10)), ControlTarget::OperatorWaveform(0));
        assert_eq!(control_target(RpnSelection::Nrpn(0, 13)), ControlTarget::OperatorWaveform(3));
        assert_eq!(control_target(RpnSelection::Nrpn(0, 18)), ControlTarget::OperatorFNumber(0));
        assert_eq!(control_target(RpnSelection::Nrpn(0, 21)), ControlTarget::OperatorFNumber(3));
    }

    #[test]
    fn reserved_fg_loop_curve_range_is_never_touched() {
        for lsb in 28..=33 {
            assert_eq!(control_target(RpnSelection::Nrpn(0, lsb)), ControlTarget::ReservedFgLoopCurve);
            assert!(!needs_voice_update(ControlTarget::ReservedFgLoopCurve));
        }
    }

    /// 旧質感LFOのNRPNアドレス。質感LFO退役後は欠番として予約し、再利用しない。
    /// NRPN(0,1)はChannelEffectRouteへ再割り当て済みのためこのリストから除外
    /// （`channel_effect_route_address`参照）。
    #[test]
    fn reserved_texture_lfo_range_is_never_touched() {
        for lsb in [0, 22, 23, 24, 25, 26, 27] {
            assert_eq!(control_target(RpnSelection::Nrpn(0, lsb)), ControlTarget::ReservedTextureLfo);
            assert!(!needs_voice_update(ControlTarget::ReservedTextureLfo));
        }
    }

    /// NRPN(0,1)は質感LFO退役で空いた欠番から`ChannelEffectRoute`へ再割り当てされている。
    #[test]
    fn channel_effect_route_address() {
        assert_eq!(control_target(RpnSelection::Nrpn(0, 1)), ControlTarget::ChannelEffectRoute);
        assert!(!needs_voice_update(ControlTarget::ChannelEffectRoute));
    }

    #[test]
    fn operator_f_number_needs_immediate_voice_update() {
        assert!(needs_voice_update(ControlTarget::OperatorFNumber(0)));
        assert!(!needs_voice_update(ControlTarget::Algorithm));
    }

    #[test]
    fn unknown_nrpn_group_is_unassigned() {
        assert_eq!(control_target(RpnSelection::Nrpn(1, 0)), ControlTarget::Unassigned);
        assert_eq!(control_target(RpnSelection::None), ControlTarget::Unassigned);
    }
}
