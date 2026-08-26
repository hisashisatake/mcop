//! 1つのMIDIチャンネル分のCC/NRPNシャドウ状態（[`ChannelState`]）。
//!
//! `op505/tools/smf2op505`・`op505/standalone`・`op505-vst`が共通で使う形態（MIDIチャンネル
//! 別に16個保持し、レンダリング/演奏で音色を組み立てる用途）を対象にする。`op505-vst`は
//! DAWパラメーター・`#[persist]`との共存が必要なため、`channels: [ChannelState; 16]`とは
//! 別にDAWパラメーター由来のベースパッチ構築（`build_patch()`）とProgram Change選択結果の
//! キャッシュ（`program_patch`）を独自に持つ（詳細はspec-fm.md 8章）。
//!
//! MasterEffects（sound-core型）は本クレートのAPIに出せない制約（Cargo.tomlコメント参照）
//! があるため、Reverb/Chorus系NRPN（NRPN(0,2)〜(0,8)）は[`DataEntryOutcome::Effect`]で
//! 呼び出し側へ通知し、呼び出し側が自分のMasterEffectsへ適用する。
//!
//! [`ChannelState`]は`reset()`（`Plugin::reset`等のリアルタイムコンテキスト）から
//! 再構築されるため、**ヒープ確保を伴うフィールド（`Vec`/`Box`/`String`/`HashMap`等）を
//! 追加してはならない**（`poly_pressure`が`HashMap`ではなくノート番号で直接引ける
//! 固定長配列になっているのはこのため）。

use crate::control::{control_target, ControlTarget};
use crate::expression::{apply_expression_modulation, apply_soft_pedal, ExpressionDestination};
use crate::overrides::PatchOverrides;
use crate::pedal::PedalState;
use crate::pitch_fg::apply_pitch_fg_expression;
use crate::rhythm::{ChannelProgramState, ProgramSelection};
use crate::sound_controller::apply_sound_controllers;
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
#[derive(Clone, PartialEq, Debug)]
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

    // --- NRPN 離散/焼き込み上書き（Program Changeでclear()される）---
    pub overrides: PatchOverrides,
    /// OP単位F-Number上書き（NRPN(0,18)〜(0,21)、13bit、Some時のみ set_operator_f_number）。
    /// `PatchOverrides`には含めない（`build_effective_patch`を通らない別経路のため、
    /// `overrides.rs`のdocコメント参照）。
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

    // --- サウンドコントローラー（CC10/71/72/73/74/75、GM2）。全て64=無補正が中立既定 ---
    pub cc10_pan: u8,
    pub cc71_resonance: u8,
    pub cc72_release: u8,
    pub cc73_attack: u8,
    pub cc74_brightness: u8,
    pub cc75_decay: u8,
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
            overrides: PatchOverrides::default(),
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
            cc10_pan: 64,
            cc71_resonance: 64,
            cc72_release: 64,
            cc73_attack: 64,
            cc74_brightness: 64,
            cc75_decay: 64,
        }
    }

    /// GM2 System Reset相当（プラグインの`reset()`等、リアルタイムコンテキストから呼ぶ）。
    /// `ChannelProgramState::reset`と対になるAPIで、`[`Self::new`]と同じ状態に戻す
    /// （CC/NRPN/ペダルを含む全フィールドを初期化する。`ChannelProgramState::reset`の
    /// docコメントにある「CC121(Reset All Controllers)からは呼ばないこと」という注意点は
    /// このメソッドにも同様に当てはまる）。
    pub fn reset(&mut self, chi: usize, rhythm_kits_available: bool) {
        *self = Self::new(chi, rhythm_kits_available);
    }

    /// ベースパッチ（プログラムの音色）に、このチャンネルの NRPN 離散/焼き込み上書きを
    /// 重ねた実効パッチを組み立てる。
    ///
    /// Pitch FG 演奏補正（CC1/76/77/78）・AT（アフタータッチ）・Soft PedalはChannelParams外の
    /// 後処理のため、ここでは扱わず[`ChannelState::apply_note_post_processing`]で適用する。
    pub fn build_effective_patch(&self, base: &Op505Patch) -> Op505Patch {
        let mut patch = *base;
        self.overrides.apply(&mut patch);
        patch
    }

    /// note_patchへ、CC2/CC4/AT/サウンドコントローラー/Pitch FG演奏補正/Soft Pedalを
    /// 一括で後適用する（呼び出し側の発音中ボイス伝播ループ・ノートオンの両方から共通で呼ぶ）。
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
        apply_sound_controllers(
            patch,
            self.cc71_resonance,
            self.cc72_release,
            self.cc73_attack,
            self.cc74_brightness,
            self.cc75_decay,
        );
        apply_pitch_fg_expression(patch, self.pitch_fg_cc1, self.pitch_fg_cc77, self.pitch_fg_cc78, self.pitch_fg_rpn0_5);
        if self.pedal.soft_notes & (1u128 << note) != 0 {
            apply_soft_pedal(patch, self.pedal.cc67);
        }
    }

    /// CC10(Pan)の現在値から左右ゲインを返す（`Vco::set_channel_pan`／`_group`へそのまま渡す）。
    pub fn pan_gains(&self) -> (f32, f32) {
        op505_core::pan_gains(self.cc10_pan)
    }

    /// Program Change（Bank Select確定後の`ChannelProgramState::program_change`ラッパー）。
    /// NRPN離散上書きレイヤーを`clear()`してから旋律/リズムを確定する（「PC＝音色を選び直す」
    /// 「その後のNRPN＝その音色への微調整」という役割分担。呼び出し側が`program_state`を
    /// 直接触るとこのクリアが漏れるため、Program Changeは必ずこのメソッド経由で行うこと）。
    pub fn program_change(&mut self, program_u7: u8) -> ProgramSelection {
        self.overrides.clear();
        self.program_state.program_change(program_u7)
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
                self.overrides.algorithm = Some(cc_byte_to_u7(raw_value).min(7));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::OperatorWaveform(op_index) => {
                self.overrides.operator_waveforms[op_index as usize] = Some(cc_byte_to_u8(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::FilterType => {
                self.overrides.filter_type = Some(cc_byte_to_u7(raw_value).min(2));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::FilterSelfOscillation => {
                self.overrides.filter_self_oscillation = Some(cc_byte_to_u7(raw_value) != 0);
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
    use crate::rpn::RpnSelection;

    fn select_nrpn(st: &mut ChannelState, msb: u8, lsb: u8) {
        st.rpn.set_nrpn_msb(msb);
        st.rpn.set_nrpn_lsb(lsb);
    }

    fn select_rpn(st: &mut ChannelState, msb: u8, lsb: u8) {
        st.rpn.set_rpn_msb(msb);
        st.rpn.set_rpn_lsb(lsb);
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
        assert_eq!(st.overrides.algorithm, Some(5));
        let eff = st.build_effective_patch(&Op505Patch::default());
        assert_eq!(eff.channel.algorithm, 5);
    }

    /// 折衷案の挙動：Program Changeで上書きレイヤーがクリアされ、以降のNRPNは新たに効く。
    #[test]
    fn program_change_clears_overrides_but_subsequent_nrpn_still_applies() {
        let mut st = ChannelState::new(0, false);
        select_nrpn(&mut st, 0, 9);
        st.apply_data_entry(7);
        assert_eq!(st.overrides.algorithm, Some(7));

        st.program_change(0);
        assert_eq!(st.overrides, PatchOverrides::default(), "Program Changeで上書きはクリアされる");

        select_nrpn(&mut st, 0, 9);
        st.apply_data_entry(5);
        assert_eq!(st.overrides.algorithm, Some(5), "PC後のNRPNは新たに効く");
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

    /// `reset()`はCC/NRPN/ペダルを含む全フィールドを`new()`直後と同じ状態に戻す。
    #[test]
    fn reset_restores_new_state() {
        let mut st = ChannelState::new(3, false);
        select_nrpn(&mut st, 0, 9);
        st.apply_data_entry(5);
        st.cc7 = 10;
        st.cc11 = 20;
        st.pitch_fg_cc1 = 99;
        st.channel_pressure = 50;
        st.poly_pressure[60] = 40;
        let _ = st.pedal.cc64(127);
        st.program_change(10);

        st.reset(3, false);
        assert_eq!(st, ChannelState::new(3, false));
    }

    // --- チャンネル独立性テスト（op505-vstをChannelStateへ全面移行する根拠。
    // 「16個持てば独立」は型的にほぼ自明だが、これらは契約の凍結として存在する） ---

    /// NRPN選択状態（RpnTracker）は`ChannelState`ごとに独立しており、他chの選択が漏れない。
    /// 14bit F-Number（CC6 MSB + CC38 LSB）のような複数CCにまたがる値の組み立てで、
    /// もし選択状態がグローバル共有だと片方のchの選択が他方のCC6を横取りしてしまう。
    #[test]
    fn interleaved_14bit_nrpn_from_two_channels_do_not_corrupt() {
        let mut ch0 = ChannelState::new(0, false);
        let mut ch1 = ChannelState::new(1, false);

        select_nrpn(&mut ch0, 0, 18);
        ch0.apply_data_entry_lsb(10);
        select_nrpn(&mut ch1, 0, 18);
        ch1.apply_data_entry_lsb(100);

        ch0.apply_data_entry(60);
        ch1.apply_data_entry(60);

        assert_eq!(ch0.operator_f_number_override[0], Some(60 * 128 + 10));
        assert_eq!(ch1.operator_f_number_override[0], Some(60 * 128 + 100));
    }

    #[test]
    fn nrpn_selection_does_not_leak_across_channels() {
        let mut ch0 = ChannelState::new(0, false);
        let ch1 = ChannelState::new(1, false);

        select_nrpn(&mut ch0, 0, 9);
        assert_eq!(ch1.rpn.selection, RpnSelection::None);

        ch0.apply_data_entry(5);
        assert_eq!(ch0.overrides.algorithm, Some(5));
        assert_eq!(ch1.overrides, PatchOverrides::default());
    }

    #[test]
    fn program_change_clears_only_its_own_overrides() {
        let mut ch0 = ChannelState::new(0, false);
        let mut ch1 = ChannelState::new(1, false);
        select_nrpn(&mut ch0, 0, 9);
        ch0.apply_data_entry(5);
        select_nrpn(&mut ch1, 0, 9);
        ch1.apply_data_entry(6);

        ch0.program_change(0);

        assert_eq!(ch0.overrides, PatchOverrides::default());
        assert_eq!(ch1.overrides.algorithm, Some(6), "ch1の上書きはch0のPCの影響を受けない");
    }

    /// CC2(ブレス)のExpression Destination（NRPN(0,34)）はchごとに独立する。
    /// ch0だけ行先をFilterCutoffへ変更すると、同じcc2値でも実効パッチがch0/ch1で異なる。
    #[test]
    fn destinations_are_per_channel() {
        let mut ch0 = ChannelState::new(0, false);
        let mut ch1 = ChannelState::new(1, false);
        select_nrpn(&mut ch0, 0, 34);
        ch0.apply_data_entry(1); // FilterCutoffへ変更（既定はTlCarriers）
        assert_eq!(ch1.cc2_destination, ExpressionDestination::TlCarriers, "ch1は既定のまま");

        ch0.cc2 = 100;
        ch1.cc2 = 100;
        let base = Op505Patch::default();
        let mut p0 = ch0.build_effective_patch(&base);
        let mut p1 = ch1.build_effective_patch(&base);
        ch0.apply_note_post_processing(&mut p0, 60);
        ch1.apply_note_post_processing(&mut p1, 60);
        assert_ne!(p0, p1, "cc2の行先が違うので実効パッチも違う");
    }

    #[test]
    fn pitch_bend_range_is_per_channel() {
        let mut ch0 = ChannelState::new(0, false);
        let ch1 = ChannelState::new(1, false);
        select_rpn(&mut ch0, 0, 0);
        ch0.apply_data_entry(12);
        assert_eq!(ch0.pitch_bend_range, 12.0);
        assert_eq!(ch1.pitch_bend_range, 2.0);
    }

    #[test]
    fn reset_all_controllers_is_per_channel() {
        let mut ch0 = ChannelState::new(0, false);
        let mut ch1 = ChannelState::new(1, false);
        ch0.pitch_fg_cc1 = 100;
        ch0.channel_pressure = 80;
        ch1.pitch_fg_cc1 = 100;
        ch1.channel_pressure = 80;

        ch0.reset_all_controllers();

        assert_eq!(ch0.pitch_fg_cc1, 0);
        assert_eq!(ch0.channel_pressure, 0);
        assert_eq!(ch1.pitch_fg_cc1, 100, "ch1はリセットされない");
        assert_eq!(ch1.channel_pressure, 80, "ch1はリセットされない");
    }

    /// 方針「note_on経路もapply_note_post_processingへ一本化する」の前提：
    /// CC未受信（中立状態）ならapply_note_post_processingを通してもパッチはベースと不変。
    #[test]
    fn apply_note_post_processing_neutral_equals_base() {
        let st = ChannelState::new(0, false);
        let base = Op505Patch::default();
        let mut patch = base;
        st.apply_note_post_processing(&mut patch, 60);
        assert_eq!(patch, base);
    }

    /// エフェクト系NRPN(0,2)〜(0,8)は`ChannelState`自身を変化させない
    /// （MasterEffectsはグローバル1個のまま。呼び出し側がEffect outcomeを見て自分で適用する）。
    #[test]
    fn effect_nrpn_reports_but_does_not_mutate_state() {
        let mut st = ChannelState::new(0, false);
        let before = st.clone();
        select_nrpn(&mut st, 0, 4); // ReverbTime
        let outcome = st.apply_data_entry(64);
        assert!(matches!(outcome, DataEntryOutcome::Effect(EffectControlTarget::ReverbTime, _)));
        // data_entry_msb・rpn.selectionはCC6/NRPN選択の受信状態として当然変化するので、
        // それ以外（overrides等の音色状態）が変化していないことを確認する。
        assert_eq!(st.overrides, before.overrides);
        assert_eq!(st.operator_f_number_override, before.operator_f_number_override);
        assert_eq!(st.cc2_destination, before.cc2_destination);
        assert_eq!(st.cc4_destination, before.cc4_destination);
        assert_eq!(st.pitch_bend_range, before.pitch_bend_range);
    }
}
