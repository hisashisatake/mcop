//! 1つのMIDIチャンネル分のCC/NRPNシャドウ状態（[`ChannelState`]）。
//!
//! `op505/tools/smf2op505`が使う形態（MIDIチャンネル別に16個保持し、オフライン/常駐
//! レンダリングで音色を組み立てる用途）を対象にする。`op505-vst`はDAWパラメーター・
//! `#[persist]`との共存が必要なため独自のシャドウ構造（プラグイングローバル＋差分検知）を
//! 持ち続けており、ここは参照しない（詳細はspec-fm.md 8章）。
//!
//! MasterEffects（sound-core型）は本クレートのAPIに出せない制約（Cargo.tomlコメント参照）
//! があるため、Reverb/Chorus系NRPN（NRPN(0,2)〜(0,8)）は[`DataEntryOutcome::Effect`]で
//! 呼び出し側へ通知し、呼び出し側が自分のMasterEffectsへ適用する。

use crate::control::{control_target, ControlTarget};
use crate::expression::{apply_expression_modulation, apply_soft_pedal, ExpressionDestination};
use crate::pedal::PedalState;
use crate::pitch_fg::apply_pitch_fg_expression;
use crate::rhythm::ChannelProgramState;
use crate::rpn::RpnTracker;
use crate::value::{cc_byte_to_u7, cc_byte_to_u8};
use op505_core::Op505Patch;

/// [`ChannelState::apply_data_entry`]の結果。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DataEntryOutcome {
    /// ChannelStateのみ変化した。`voice_update`がtrueなら発音中ボイスへの即時反映が必要。
    StateChanged { voice_update: bool },
    /// エフェクト系NRPN。呼び出し側が`target`に応じて自分のMasterEffectsへ`value`を適用する。
    Effect(EffectControlTarget, u8),
}

/// エフェクト系NRPN（NRPN(0,2)〜(0,8)）。MasterEffectsはsound-core型のため本クレートの
/// APIに出せず、[`DataEntryOutcome::Effect`]で呼び出し側へ通知する
/// （`ControlTarget`の対応バリアントの部分集合）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EffectControlTarget {
    ReverbType,
    ChorusType,
    ReverbTime,
    ChorusModRate,
    ChorusModDepth,
    ChorusFeedback,
    ChorusSendToReverb,
}

/// 1つのMIDIチャンネルの解釈状態（CC/NRPN シャドウ）。16個で全チャンネルを管理する
/// （`op505-vst`のプラグイングローバル・シャドウフィールドを**チャンネル別**に持ち直したもの）。
///
/// NRPN で上書きされる離散/焼き込みフィールドは `Option`（None=ベースパッチ値のまま＝
/// 「NRPN は現在のパッチの当該フィールドのみ書き換え」）、CC1/76/77/78 の Pitch FG 補正は
/// 中立既定の加算値として常時適用する。
pub struct ChannelState {
    /// Bank Select + Program Change の状態機械（VSTと同じ型・同じ解釈）。
    /// GM2リズムチャンネル判定（Bank Select MSB=120動的切替）を含む。
    pub program_state: ChannelProgramState,

    pub rpn: RpnTracker,
    pub data_entry_msb: u8,
    pub data_entry_lsb: u8,

    // --- ピッチベンド ---
    /// ピッチベンド感度（半音）。RPN(0,0)で変更、既定±2半音。
    pub pitch_bend_range: f32,
    /// 現在のベンド量（セント）。note_on 時に新ボイスへ再適用する。
    pub bend_cents: f32,

    // --- 音量（CC7/CC11、GM2）---
    pub cc7: u8,
    pub cc11: u8,

    // --- Pitch FG 演奏補正（中立既定、常時適用）---
    pub pitch_fg_cc1: u8,    // CC1 Modulation Wheel（0〜127）
    pub pitch_fg_cc76: u8,   // CC76 Vibrato Rate（0〜127、64=無補正）
    pub pitch_fg_cc77: u8,   // CC77 Vibrato Depth（0〜255、Depthへ0起点加算）
    pub pitch_fg_cc78: u8,   // CC78 Vibrato Delay（0〜127、64=無補正）
    pub pitch_fg_rpn0_5: u8, // RPN(0,5) Modulation Depth Range（GM2、既定64）

    // --- NRPN 離散/焼き込み上書き（None=ベースパッチ値）---
    pub algorithm: Option<u8>,
    pub operator_waveforms: [Option<u8>; 4],
    pub filter_type: Option<u8>,
    pub filter_self_oscillation: Option<bool>,
    /// OP単位F-Number上書き（NRPN(0,18)〜(0,21)、13bit、Some時のみ set_operator_f_number）。
    pub operator_f_number_override: [Option<u16>; 4],

    // --- アフタータッチ ---
    pub at_destination: ExpressionDestination,
    pub poly_at_destination: ExpressionDestination,
    pub channel_pressure: u8,
    /// Poly Key Pressure（ノート番号→圧力値）。ノート番号(0〜127)で直接引ける固定長配列。
    pub poly_pressure: [u8; 128],

    // --- CC2(ブレス)/CC4(フット) ---（NRPN(0,34)/(0,35)で行先選択、既定はCC2→TLキャリア一括／
    // CC4→Filter Cutoff＝手動ワウ）
    pub cc2: u8,
    pub cc4: u8,
    pub cc2_destination: ExpressionDestination,
    pub cc4_destination: ExpressionDestination,

    // --- ペダル（CC64 Sustain / CC66 Sostenuto / CC67 Soft）---
    pub pedal: PedalState,
}

impl ChannelState {
    /// `chi`はMIDIチャンネルindex(0〜15)、`rhythm_kits_available`は`--drum-bank`等でリズム
    /// キットが1つでもロードされているか（[`ChannelProgramState::new`]参照。falseなら
    /// ch10(chi==9)でも旋律で始まる、キット未ロード環境での回帰防止）。
    pub fn new(chi: usize, rhythm_kits_available: bool) -> Self {
        Self {
            program_state: ChannelProgramState::new(chi, rhythm_kits_available),
            rpn: RpnTracker::default(),
            data_entry_msb: 0,
            data_entry_lsb: 0,
            pitch_bend_range: 2.0,
            bend_cents: 0.0,
            cc7: 127,
            cc11: 127,
            pitch_fg_cc1: 0,
            pitch_fg_cc76: 64,
            pitch_fg_cc77: 0,
            pitch_fg_cc78: 64,
            pitch_fg_rpn0_5: 64,
            algorithm: None,
            operator_waveforms: [None; 4],
            filter_type: None,
            filter_self_oscillation: None,
            operator_f_number_override: [None; 4],
            at_destination: ExpressionDestination::default(),
            poly_at_destination: ExpressionDestination::default(),
            channel_pressure: 0,
            poly_pressure: [0; 128],
            cc2: 0,
            cc4: 0,
            cc2_destination: ExpressionDestination::TlCarriers,
            cc4_destination: ExpressionDestination::FilterCutoff,
            pedal: PedalState::default(),
        }
    }

    /// ベースパッチ（プログラムの音色）に、このチャンネルの NRPN 離散/焼き込み上書きを
    /// 重ねた実効パッチを組み立てる。
    ///
    /// Pitch FG 演奏補正（CC1/76/77/78）・AT（アフタータッチ）・Soft PedalはChannelParams外の
    /// 後処理のため、ここでは扱わず[`ChannelState::apply_note_post_processing`]で適用する。
    pub fn build_effective_patch(&self, base: &Op505Patch) -> Op505Patch {
        let mut patch = *base;

        if let Some(v) = self.algorithm {
            patch.channel.algorithm = v;
        }
        for (i, wf) in self.operator_waveforms.iter().enumerate() {
            if let Some(v) = wf {
                patch.operators[i].waveform = *v;
            }
        }
        if let Some(v) = self.filter_type {
            patch.channel.filter_type = v;
        }
        if let Some(v) = self.filter_self_oscillation {
            patch.channel.filter_self_oscillation = v;
        }

        patch
    }

    /// note_patchへ、CC2/CC4/AT/Pitch FG演奏補正/Soft Pedalを一括で後適用する
    /// （呼び出し側の発音中ボイス伝播ループ・ノートオンの両方から共通で呼ぶ）。
    pub fn apply_note_post_processing(&self, patch: &mut Op505Patch, note: u8) {
        apply_expression_modulation(
            note,
            &[
                (self.cc2, self.cc2_destination),
                (self.cc4, self.cc4_destination),
                (self.channel_pressure, self.at_destination),
            ],
            self.poly_at_destination,
            &self.poly_pressure,
            patch,
        );
        apply_pitch_fg_expression(patch, self.pitch_fg_cc1, self.pitch_fg_cc77, self.pitch_fg_cc78, self.pitch_fg_rpn0_5);
        if self.pedal.soft_notes & (1u128 << note) != 0 {
            apply_soft_pedal(patch, self.pedal.cc67);
        }
    }

    /// NRPN(0,18)〜(0,21)：CC6(MSB)+CC38(LSB)の14bit値を13bit(0〜8191)にclampして
    /// OP F-Number 上書きとして記録する。
    fn apply_operator_f_number_override(&mut self, op_index: usize) {
        let combined = (self.data_entry_msb as u16) * 128 + self.data_entry_lsb as u16;
        self.operator_f_number_override[op_index] = Some(combined.min(8191));
    }

    /// CC38(Data Entry LSB)受信時の処理。OP F-Number(NRPN(0,18)〜(0,21)選択中)のときだけ
    /// 14bit値を更新する。戻り値は発音中ボイスへの即時反映が必要か。
    pub fn apply_data_entry_lsb(&mut self, raw_value: u8) -> bool {
        self.data_entry_lsb = cc_byte_to_u7(raw_value);
        if let ControlTarget::OperatorFNumber(op_index) = control_target(self.rpn.selection) {
            self.apply_operator_f_number_override(op_index as usize);
            true
        } else {
            false
        }
    }

    /// CC6(Data Entry MSB)受信時、[`control_target`]で解決した制御対象に応じて値を適用する。
    ///
    /// `ControlTarget::ReservedFgLoopCurve`（op505のTimeEg 7本はpersist状態でNRPNからは
    /// 触らない欠番）・`ReservedTextureLfo`（旧質感LFO、退役済み欠番）は何もしない。
    pub fn apply_data_entry(&mut self, raw_value: u8) -> DataEntryOutcome {
        self.data_entry_msb = cc_byte_to_u7(raw_value);
        match control_target(self.rpn.selection) {
            ControlTarget::PitchBendRange => {
                self.pitch_bend_range = cc_byte_to_u7(raw_value) as f32;
                DataEntryOutcome::StateChanged { voice_update: false }
            }
            ControlTarget::ModulationDepthRange => {
                self.pitch_fg_rpn0_5 = cc_byte_to_u7(raw_value);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::ReservedTextureLfo => DataEntryOutcome::StateChanged { voice_update: false },
            ControlTarget::ReverbType => {
                DataEntryOutcome::Effect(EffectControlTarget::ReverbType, cc_byte_to_u7(raw_value))
            }
            ControlTarget::ChorusType => {
                DataEntryOutcome::Effect(EffectControlTarget::ChorusType, cc_byte_to_u7(raw_value))
            }
            ControlTarget::ReverbTime => {
                DataEntryOutcome::Effect(EffectControlTarget::ReverbTime, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusModRate => {
                DataEntryOutcome::Effect(EffectControlTarget::ChorusModRate, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusModDepth => {
                DataEntryOutcome::Effect(EffectControlTarget::ChorusModDepth, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusFeedback => {
                DataEntryOutcome::Effect(EffectControlTarget::ChorusFeedback, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusSendToReverb => {
                DataEntryOutcome::Effect(EffectControlTarget::ChorusSendToReverb, cc_byte_to_u8(raw_value))
            }
            ControlTarget::Algorithm => {
                self.algorithm = Some(cc_byte_to_u7(raw_value).min(7));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::OperatorWaveform(op_index) => {
                self.operator_waveforms[op_index as usize] = Some(cc_byte_to_u8(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::FilterType => {
                self.filter_type = Some(cc_byte_to_u7(raw_value).min(2));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::FilterSelfOscillation => {
                self.filter_self_oscillation = Some(cc_byte_to_u7(raw_value) != 0);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::AtDestination => {
                self.at_destination = ExpressionDestination::from_u8(cc_byte_to_u7(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::PolyAtDestination => {
                self.poly_at_destination = ExpressionDestination::from_u8(cc_byte_to_u7(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::OperatorFNumber(op_index) => {
                self.apply_operator_f_number_override(op_index as usize);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::ReservedFgLoopCurve => DataEntryOutcome::StateChanged { voice_update: false },
            ControlTarget::Cc2Destination => {
                self.cc2_destination = ExpressionDestination::from_u8(cc_byte_to_u7(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::Cc4Destination => {
                self.cc4_destination = ExpressionDestination::from_u8(cc_byte_to_u7(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::Unassigned => DataEntryOutcome::StateChanged { voice_update: false },
        }
    }

    /// CC121(Reset All Controllers)：③ジェスチャー層のみリセットする（②パート状態・
    /// ①音色は保持、program_stateへは意図的に触れない）。戻り値はペダル解放されたノートの
    /// ビットマスク（[`crate::pedal::released_notes`]で走査する）。
    pub fn reset_all_controllers(&mut self) -> u128 {
        let released = self.pedal.cc121();
        self.pitch_fg_cc1 = 0;
        self.bend_cents = 0.0;
        self.channel_pressure = 0;
        self.poly_pressure = [0; 128];
        released
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select_nrpn(st: &mut ChannelState, msb: u8, lsb: u8) {
        st.rpn.set_nrpn_msb(msb);
        st.rpn.set_nrpn_lsb(lsb);
    }

    /// 中立の ChannelState では実効パッチがベースと一致する。
    #[test]
    fn effective_patch_neutral_equals_base() {
        let base = Op505Patch::default();
        let st = ChannelState::new(0, false);
        let eff = st.build_effective_patch(&base);
        assert_eq!(eff, base);
    }

    /// NRPN(0,0)〜(0,1)・(0,22)〜(0,27)（旧質感LFO）は質感LFO退役後、欠番として何もしない。
    #[test]
    fn nrpn_reserved_texture_lfo_is_noop() {
        let mut st = ChannelState::new(0, false);
        for lsb in [0u8, 1, 22, 23, 24, 25, 26, 27] {
            select_nrpn(&mut st, 0, lsb);
            assert_eq!(
                st.apply_data_entry(100),
                DataEntryOutcome::StateChanged { voice_update: false },
                "NRPN(0,{lsb}) should be a no-op"
            );
        }
        let eff = st.build_effective_patch(&Op505Patch::default());
        assert_eq!(eff, Op505Patch::default());
    }

    /// NRPN(0,9) Algorithm 上書きは実効パッチの algorithm を置き換える。
    #[test]
    fn nrpn_algorithm_override() {
        let mut st = ChannelState::new(0, false);
        select_nrpn(&mut st, 0, 9);
        assert_eq!(st.apply_data_entry(5), DataEntryOutcome::StateChanged { voice_update: true });
        assert_eq!(st.algorithm, Some(5));
        let eff = st.build_effective_patch(&Op505Patch::default());
        assert_eq!(eff.channel.algorithm, 5);
    }

    /// エフェクト系 NRPN(0,2〜8) は`DataEntryOutcome::Effect`（ChannelState上のvoice_updateは無し）。
    #[test]
    fn nrpn_effects_return_effect_outcome() {
        let mut st = ChannelState::new(0, false);
        for lsb in [2u8, 3, 4, 5, 6, 7, 8] {
            select_nrpn(&mut st, 0, lsb);
            let outcome = st.apply_data_entry(64);
            assert!(matches!(outcome, DataEntryOutcome::Effect(_, _)), "NRPN(0,{lsb}) should be Effect");
        }
    }

    /// NRPN(0,28)〜(0,33)（op505ではReservedFgLoopCurve）は何も変えず voice_update=false を返す。
    #[test]
    fn nrpn_reserved_fg_loop_curve_is_noop() {
        let mut st = ChannelState::new(0, false);
        for lsb in 28u8..=33 {
            select_nrpn(&mut st, 0, lsb);
            assert_eq!(
                st.apply_data_entry(127),
                DataEntryOutcome::StateChanged { voice_update: false },
                "NRPN(0,{lsb}) should be a no-op"
            );
        }
    }

    /// NRPN(0,18)＝OP0 F-Number は CC6(MSB)+CC38(LSB)の14bit→13bit clamp。
    #[test]
    fn nrpn_operator_f_number_14bit() {
        let mut st = ChannelState::new(0, false);
        select_nrpn(&mut st, 0, 18);
        st.apply_data_entry_lsb(10);
        assert_eq!(st.apply_data_entry(60), DataEntryOutcome::StateChanged { voice_update: true }); // msb=60 → 60*128+10 = 7690
        assert_eq!(st.operator_f_number_override[0], Some(7690));
        // 8191 超は clamp
        st.apply_data_entry_lsb(127);
        assert_eq!(st.apply_data_entry(127), DataEntryOutcome::StateChanged { voice_update: true }); // 127*128+127 = 16383 → 8191
        assert_eq!(st.operator_f_number_override[0], Some(8191));
    }
}
