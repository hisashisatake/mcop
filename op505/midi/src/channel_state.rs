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
//! 呼び出し側へ通知し、呼び出し側が自分のMasterEffectsへ適用する。エフェクトはチャンネル別に
//! `effect_route_slot`（NRPN(0,1) Channel Effect Routeで設定、既定0）へルーティングされる
//! マルチスロット構成を前提とし、エフェクト設定NRPN・CC91/93はいずれも送信チャンネルの
//! `effect_route_slot`が指すスロットへ適用される（詳細はspec-fm.md 8章）。
//!
//! [`ChannelState`]は`reset()`（`Plugin::reset`等のリアルタイムコンテキスト）から
//! 再構築されるため、**ヒープ確保を伴うフィールド（`Vec`/`Box`/`String`/`HashMap`等）を
//! 追加してはならない**（`poly_pressure`が`HashMap`ではなくノート番号で直接引ける
//! 固定長配列になっているのはこのため）。

use crate::control::{control_target, ControlTarget};
use crate::expression::{apply_expression_modulation, apply_soft_pedal, ExpressionDestination};
use crate::mono::MonoState;
use crate::overrides::PatchOverrides;
use crate::pedal::PedalState;
use crate::gain_fg::apply_gain_fg_expression;
use crate::pitch_fg::apply_pitch_fg_expression;
use crate::rhythm::{ChannelProgramState, ProgramSelection};
use crate::sound_controller::apply_sound_controllers;
use crate::rpn::RpnTracker;
use crate::value::{cc_byte_to_u7, cc_byte_to_u8};
use op505_core::Op505Patch;

/// エフェクトスロット数（MIDIチャンネル数と同数）。NRPN(0,1) Channel Effect Routeで
/// 指定するスロット番号（0〜`EFFECT_SLOT_COUNT - 1`）と、ホスト側の`MasterEffects`配列長を
/// この値で揃える（`op505-midi`側でクランプ境界を一元管理し、3ホストが個別に上限値を
/// ハードコードするのを避ける）。
pub const EFFECT_SLOT_COUNT: u8 = 16;

/// [`ChannelState::apply_data_entry`]の結果。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DataEntryOutcome {
    /// ChannelStateのみ変化した。`voice_update`がtrueなら発音中ボイスへの即時反映が必要。
    StateChanged { voice_update: bool },
    /// エフェクト系NRPN。呼び出し側が`effect_route_slot`（0〜`EFFECT_SLOT_COUNT - 1`）が指す
    /// 自分のMasterEffectsへ`value`を適用する。
    Effect(u8, EffectControlTarget, u8),
}

/// エフェクト系NRPN（NRPN(0,2)〜(0,8)）。MasterEffectsはsound-core型のため本クレートの
/// APIに出せず、[`DataEntryOutcome::Effect`]で呼び出し側へ通知する
/// （`ControlTarget`の対応バリアントの部分集合）。適用先スロット番号は
/// `DataEntryOutcome::Effect`の先頭要素（`u8`）が別途運ぶ。
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

    // --- チューニング（RPN(0,1)/(0,2)、GM2必須セット）---
    /// RPN(0,2) Channel Coarse Tuning（MSBのみ、64＝無補正、値-64が半音オフセット）。
    pub tune_coarse: u8,
    /// RPN(0,1) Channel Fine Tuning（CC6(MSB)+CC38(LSB)の14bit、8192＝無補正、±100セント）。
    pub tune_fine: u16,

    // --- 音量（CC7/CC11、GM2）---
    pub cc7: u8,
    pub cc11: u8,

    // --- Pitch FG 演奏補正（中立既定、常時適用）---
    pub pitch_fg_cc1: u8,    // CC1 Modulation Wheel（0〜127）
    pub pitch_fg_cc76: u8,   // CC76 Vibrato Rate（0〜127、64=無補正）
    pub pitch_fg_cc77: u8,   // CC77 Vibrato Depth（0〜255、Depthへ0起点加算）
    pub pitch_fg_cc78: u8,   // CC78 Vibrato Delay（0〜127、64=無補正）
    pub pitch_fg_rpn0_5: u8, // RPN(0,5) Modulation Depth Range（GM2、既定64）

    // --- Gain FG 演奏補正（中立既定、常時適用）---
    pub gain_fg_cc92: u8, // CC92 Tremolo Depth（0〜255、Depthへ0起点加算。RPN連動レンジは無い）

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

    // --- ポルタメント（CC5/CC65、GM2）---
    /// CC65 Portamento On/Off（>=64でON）。
    pub portamento_on: bool,
    /// CC5 Portamento Time（0〜127、0=グライドなし）。
    pub portamento_time: u8,
    /// 直前に`note_on`したノート番号（グライド起点）。CC120/CC121/CC123の全てでクリアする
    /// （このチャンネルの発音状態が丸ごとリセットされる操作なので、直前ノートという文脈も
    /// 一緒に消える方が自然なため）。
    pub last_note: Option<u8>,

    // --- Mono/Poly Mode（CC126/CC127、GM2）---
    pub mono: MonoState,

    /// NRPN(0,1) Channel Effect Route：このチャンネルの音声・エフェクト設定NRPN(0,2)〜(0,8)・
    /// CC91/93の適用先エフェクトスロット番号（0〜`EFFECT_SLOT_COUNT - 1`）。既定0
    /// （誰も送らなければ全チャンネルがスロット0に集まり、既存の単一MasterEffectsと同じ挙動）。
    pub effect_route_slot: u8,
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
            tune_coarse: 64,
            tune_fine: 8192,
            cc7: 127,
            cc11: 127,
            pitch_fg_cc1: 0,
            pitch_fg_cc76: 64,
            pitch_fg_cc77: 0,
            pitch_fg_cc78: 64,
            pitch_fg_rpn0_5: 64,
            gain_fg_cc92: 0,
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
            portamento_on: false,
            portamento_time: 0,
            last_note: None,
            mono: MonoState::default(),
            effect_route_slot: 0,
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
        apply_gain_fg_expression(patch, self.gain_fg_cc92);
        if self.pedal.soft_notes & (1u128 << note) != 0 {
            apply_soft_pedal(patch, self.pedal.cc67);
        }
    }

    /// CC10(Pan)の現在値から左右ゲインを返す（`Vco::set_channel_pan`／`_group`へそのまま渡す）。
    pub fn pan_gains(&self) -> (f32, f32) {
        op505_core::pan_gains(self.cc10_pan)
    }

    /// RPN(0,1)/(0,2) Channel Fine/Coarse Tuningの合計セントオフセット。
    /// ピッチベンド（`bend_cents`）とは独立の「チャンネル設定」のため、CC121(Reset All
    /// Controllers)では初期化しない（`portamento_time`と同じ扱い、`reset_all_controllers`参照）。
    pub fn tune_cents(&self) -> f32 {
        let coarse = (self.tune_coarse as f32 - 64.0) * 100.0;
        let fine = (self.tune_fine as f32 - 8192.0) / 8192.0 * 100.0;
        coarse + fine
    }

    /// ピッチベンド（`bend_cents`）とチューニング（`tune_cents`）を合算した、
    /// `Vco::set_pitch_bend`／`_group`へ渡すべき実際のセント値。
    pub fn total_pitch_bend_cents(&self) -> f32 {
        self.bend_cents + self.tune_cents()
    }

    /// CC5(Portamento Time)からグライド秒数を導く。0=グライドなし、127で約5秒
    /// （5ms×1000^(cc5/127)の対数スケール。実機シンセのポルタメントタイムノブに倣い、
    /// 低い値側を細かく、高い値側を大きく動かせるようにする）。
    pub fn portamento_seconds(&self) -> f32 {
        if self.portamento_time == 0 {
            0.0
        } else {
            0.005 * 1000f32.powf(self.portamento_time as f32 / 127.0)
        }
    }

    /// このnote_onでグライドすべきなら`(起点ノート, 秒)`を返す。`portamento_on`かつ
    /// `last_note`が別ノートで、かつ秒>0のときだけSome。周波数への変換は呼び出し側が
    /// 各自の音名/周波数変換関数で行う（本クレートはMIDIノート番号までしか扱わず、
    /// 周波数計算はop505-core/呼び出し側の責務のため）。
    pub fn glide_source(&self, note: u8) -> Option<(u8, f32)> {
        if !self.portamento_on {
            return None;
        }
        let from = self.last_note?;
        if from == note {
            return None;
        }
        let seconds = self.portamento_seconds();
        if seconds <= 0.0 {
            return None;
        }
        Some((from, seconds))
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

    /// RPN(0,1)：CC6(MSB)+CC38(LSB)の14bit値(0〜16383、8192＝無補正)をそのまま`tune_fine`へ。
    fn apply_channel_fine_tuning(&mut self) {
        self.tune_fine = (self.data_entry_msb as u16) * 128 + self.data_entry_lsb as u16;
    }

    /// CC38(Data Entry LSB)受信時の処理。OP F-Number(NRPN(0,18)〜(0,21))・Channel Fine
    /// Tuning(RPN(0,1))選択中のときだけ14bit値を更新する。戻り値は発音中ボイスへの
    /// 即時反映が必要か。
    pub fn apply_data_entry_lsb(&mut self, raw_value: u8) -> bool {
        self.data_entry_lsb = cc_byte_to_u7(raw_value);
        match control_target(self.rpn.selection) {
            ControlTarget::OperatorFNumber(op_index) => {
                self.apply_operator_f_number_override(op_index as usize);
                true
            }
            ControlTarget::ChannelFineTuning => {
                self.apply_channel_fine_tuning();
                true
            }
            _ => false,
        }
    }

    /// CC6(Data Entry MSB)受信時、[`control_target`]で解決した制御対象に応じて値を適用する。
    ///
    /// `ControlTarget::ReservedTextureLfo`（旧質感LFO、退役済み欠番）は何もしない。
    /// FG Loop/Curve（NRPN(0,28)〜(0,33)）は`PatchOverrides`経由（Algorithm/FilterTypeと同じ
    /// 「NRPN離散上書きレイヤー」）。`op505-vst`ではTimeEg本体がpersist状態のため、音には
    /// 即座に反映されるがGUIエディタの表示・Save後のプリセットへは反映されない
    /// （Algorithm等と同じ既知の制約）。
    ///
    /// `ControlTarget::ChannelEffectRoute`（NRPN(0,1)）はこのチャンネル自身の`effect_route_slot`
    /// を書き換えるだけで`DataEntryOutcome::StateChanged`を返す。NRPN(0,2)〜(0,8)（エフェクト
    /// 設定7項目）は常にその時点の`effect_route_slot`を`DataEntryOutcome::Effect`の先頭要素として
    /// 返す（「エフェクト関連の設定は送信したチャンネルのルーティング先スロットへ適用する」という
    /// 1レジスタ統合方式、詳細はspec-fm.md 8章）。
    pub fn apply_data_entry(&mut self, raw_value: u8) -> DataEntryOutcome {
        self.data_entry_msb = cc_byte_to_u7(raw_value);
        match control_target(self.rpn.selection) {
            ControlTarget::PitchBendRange => {
                self.pitch_bend_range = cc_byte_to_u7(raw_value) as f32;
                DataEntryOutcome::StateChanged { voice_update: false }
            }
            ControlTarget::ChannelFineTuning => {
                self.apply_channel_fine_tuning();
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::ChannelCoarseTuning => {
                self.tune_coarse = cc_byte_to_u7(raw_value);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::ModulationDepthRange => {
                self.pitch_fg_rpn0_5 = cc_byte_to_u7(raw_value);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::ReservedTextureLfo => DataEntryOutcome::StateChanged { voice_update: false },
            ControlTarget::ChannelEffectRoute => {
                self.effect_route_slot = cc_byte_to_u7(raw_value).min(EFFECT_SLOT_COUNT - 1);
                DataEntryOutcome::StateChanged { voice_update: false }
            }
            ControlTarget::ReverbType => {
                DataEntryOutcome::Effect(self.effect_route_slot, EffectControlTarget::ReverbType, cc_byte_to_u7(raw_value))
            }
            ControlTarget::ChorusType => {
                DataEntryOutcome::Effect(self.effect_route_slot, EffectControlTarget::ChorusType, cc_byte_to_u7(raw_value))
            }
            ControlTarget::ReverbTime => {
                DataEntryOutcome::Effect(self.effect_route_slot, EffectControlTarget::ReverbTime, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusModRate => {
                DataEntryOutcome::Effect(self.effect_route_slot, EffectControlTarget::ChorusModRate, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusModDepth => {
                DataEntryOutcome::Effect(self.effect_route_slot, EffectControlTarget::ChorusModDepth, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusFeedback => {
                DataEntryOutcome::Effect(self.effect_route_slot, EffectControlTarget::ChorusFeedback, cc_byte_to_u8(raw_value))
            }
            ControlTarget::ChorusSendToReverb => {
                DataEntryOutcome::Effect(
                    self.effect_route_slot,
                    EffectControlTarget::ChorusSendToReverb,
                    cc_byte_to_u8(raw_value),
                )
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
            ControlTarget::FixedNoteEnable => {
                self.overrides.fixed_note_enable = Some(cc_byte_to_u7(raw_value) != 0);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::FixedNote => {
                self.overrides.fixed_note = Some(cc_byte_to_u7(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::FixedNoteFine => {
                self.overrides.fixed_note_fine = Some(cc_byte_to_u8(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::PitchFgDepth => {
                self.overrides.pitch_fg_depth = Some(cc_byte_to_u8(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::CutoffFgDepth => {
                self.overrides.cutoff_fg_depth = Some(cc_byte_to_u8(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::GainFgDepth => {
                self.overrides.gain_fg_depth = Some(cc_byte_to_u8(raw_value));
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::PitchFgLoop => {
                self.overrides.pitch_fg_loop = Some((cc_byte_to_u7(raw_value) != 0) as u8);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::PitchFgCurve => {
                self.overrides.pitch_fg_curve = Some((cc_byte_to_u7(raw_value) != 0) as u8);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::CutoffFgLoop => {
                self.overrides.cutoff_fg_loop = Some((cc_byte_to_u7(raw_value) != 0) as u8);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::CutoffFgCurve => {
                self.overrides.cutoff_fg_curve = Some((cc_byte_to_u7(raw_value) != 0) as u8);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::GainFgLoop => {
                self.overrides.gain_fg_loop = Some((cc_byte_to_u7(raw_value) != 0) as u8);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
            ControlTarget::GainFgCurve => {
                self.overrides.gain_fg_curve = Some((cc_byte_to_u7(raw_value) != 0) as u8);
                DataEntryOutcome::StateChanged { voice_update: true }
            }
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
        // CC65(Portamento On/Off)はRAC対象。CC5(Time)はRAC対象外（GM2のRACはOn/Offスイッチ系の
        // コントローラーのみを対象にする一般的な扱いに合わせる）。
        self.portamento_on = false;
        self.last_note = None;
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

    /// NRPN(0,0)（旧質感LFO）は質感LFO退役後、欠番として何もしない。NRPN(0,1)は
    /// ChannelEffectRoute、(0,22)〜(0,24)はFixed Note 3項目、(0,25)〜(0,27)はFG Depth
    /// 3項目へ再割り当て済みのためこのリストから除外（別テスト参照）。
    #[test]
    fn nrpn_reserved_texture_lfo_is_noop() {
        let mut st = ChannelState::new(0, false);
        select_nrpn(&mut st, 0, 0);
        assert_eq!(
            st.apply_data_entry(100),
            DataEntryOutcome::StateChanged { voice_update: false },
            "NRPN(0,0) should be a no-op"
        );
        let eff = st.build_effective_patch(&Op505Patch::default());
        assert_eq!(eff, Op505Patch::default());
    }

    /// NRPN(0,25)〜(0,27) FG Depth：`PatchOverrides`経由で絶対上書きし、発音中ボイスへの
    /// 即時反映が必要（voice_update=true）。
    #[test]
    fn nrpn_fg_depth_overrides_apply_absolute_value() {
        // raw_valueはMIDIの生CCバイト(0〜127の7bit)。100 -> cc_byte_to_u8 -> round(100/127*255)=201。
        let mut st = ChannelState::new(0, false);
        for lsb in [25u8, 26, 27] {
            select_nrpn(&mut st, 0, lsb);
            assert_eq!(
                st.apply_data_entry(100),
                DataEntryOutcome::StateChanged { voice_update: true },
                "NRPN(0,{lsb}) should require voice update"
            );
        }
        assert_eq!(st.overrides.pitch_fg_depth, Some(201));
        assert_eq!(st.overrides.cutoff_fg_depth, Some(201));
        assert_eq!(st.overrides.gain_fg_depth, Some(201));

        let eff = st.build_effective_patch(&Op505Patch::default());
        assert_eq!(eff.channel.pitch_fg.depth, 201);
        assert_eq!(eff.channel.cutoff_fg.depth, 201);
        assert_eq!(eff.channel.gain_fg.depth, 201);
    }

    /// NRPN(0,1) Channel Effect Route は`effect_route_slot`を書き換え、
    /// `EFFECT_SLOT_COUNT - 1`を超える値はクランプされる。
    #[test]
    fn nrpn_channel_effect_route_updates_effect_route_slot() {
        let mut st = ChannelState::new(0, false);
        assert_eq!(st.effect_route_slot, 0);

        select_nrpn(&mut st, 0, 1);
        assert_eq!(st.apply_data_entry(3), DataEntryOutcome::StateChanged { voice_update: false });
        assert_eq!(st.effect_route_slot, 3);

        select_nrpn(&mut st, 0, 1);
        st.apply_data_entry(127);
        assert_eq!(st.effect_route_slot, EFFECT_SLOT_COUNT - 1, "127はEFFECT_SLOT_COUNT-1へクランプされる");
    }

    /// エフェクト設定NRPN(0,2)〜(0,8)は、その時点の`effect_route_slot`を
    /// `DataEntryOutcome::Effect`の先頭要素として運ぶ。
    #[test]
    fn effect_nrpn_outcome_carries_effect_route_slot() {
        let mut st = ChannelState::new(0, false);
        select_nrpn(&mut st, 0, 1);
        st.apply_data_entry(3); // effect_route_slot = 3

        select_nrpn(&mut st, 0, 4); // Reverb Time
        let outcome = st.apply_data_entry(64);
        // ReverbTimeはcc_byte_to_u8（7bit→8bit拡大）を使うため、生値64は129になる。
        assert_eq!(outcome, DataEntryOutcome::Effect(3, EffectControlTarget::ReverbTime, cc_byte_to_u8(64)));
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

    /// NRPN(0,22)〜(0,24) Fixed Note 3項目は`overrides`へ書き込まれ、実効パッチの
    /// `channel.fixed_note_enable`/`fixed_note`/`fixed_note_fine`を置き換える。
    #[test]
    fn nrpn_fixed_note_overrides() {
        let mut st = ChannelState::new(0, false);

        select_nrpn(&mut st, 0, 22); // Fixed Note Enable
        assert_eq!(st.apply_data_entry(127), DataEntryOutcome::StateChanged { voice_update: true });
        assert_eq!(st.overrides.fixed_note_enable, Some(true));

        select_nrpn(&mut st, 0, 23); // Fixed Note
        assert_eq!(st.apply_data_entry(60), DataEntryOutcome::StateChanged { voice_update: true });
        assert_eq!(st.overrides.fixed_note, Some(60));

        select_nrpn(&mut st, 0, 24); // Fixed Note Fine
        // ReverbTimeと同じcc_byte_to_u8（7bit→8bit拡大）を使うため、生値64は129になる。
        assert_eq!(st.apply_data_entry(64), DataEntryOutcome::StateChanged { voice_update: true });
        assert_eq!(st.overrides.fixed_note_fine, Some(cc_byte_to_u8(64)));

        let eff = st.build_effective_patch(&Op505Patch::default());
        assert!(eff.channel.fixed_note_enable);
        assert_eq!(eff.channel.fixed_note, 60);
        assert_eq!(eff.channel.fixed_note_fine, cc_byte_to_u8(64));
    }

    /// NRPN(0,22) Fixed Note Enableは生値0で無効(false)、非0(=127)で有効(true)になる
    /// （`filter_self_oscillation`と同じ「非0=true」規約）。
    #[test]
    fn nrpn_fixed_note_enable_zero_is_false() {
        let mut st = ChannelState::new(0, false);
        select_nrpn(&mut st, 0, 22);
        st.apply_data_entry(0);
        assert_eq!(st.overrides.fixed_note_enable, Some(false));
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
            assert!(matches!(outcome, DataEntryOutcome::Effect(_, _, _)), "NRPN(0,{lsb}) should be Effect");
        }
    }

    /// NRPN(0,28)〜(0,33) FG Loop/Curveは`overrides`へ書き込まれ、実効パッチの
    /// pitch/cutoff/gain各FGのloop_enabled・全段curveを置き換える。
    #[test]
    fn nrpn_fg_loop_curve_overrides() {
        let mut st = ChannelState::new(0, false);
        for lsb in 28u8..=33 {
            select_nrpn(&mut st, 0, lsb);
            assert_eq!(
                st.apply_data_entry(127),
                DataEntryOutcome::StateChanged { voice_update: true },
                "NRPN(0,{lsb}) should update overrides"
            );
        }
        assert_eq!(st.overrides.pitch_fg_loop, Some(1));
        assert_eq!(st.overrides.pitch_fg_curve, Some(1));
        assert_eq!(st.overrides.cutoff_fg_loop, Some(1));
        assert_eq!(st.overrides.cutoff_fg_curve, Some(1));
        assert_eq!(st.overrides.gain_fg_loop, Some(1));
        assert_eq!(st.overrides.gain_fg_curve, Some(1));

        let eff = st.build_effective_patch(&Op505Patch::default());
        assert_eq!(eff.channel.pitch_fg.eg.loop_enabled, 1);
        assert!(eff.channel.pitch_fg.eg.stages.iter().all(|s| s.curve == 1));
        assert_eq!(eff.channel.cutoff_fg.eg.loop_enabled, 1);
        assert!(eff.channel.cutoff_fg.eg.stages.iter().all(|s| s.curve == 1));
        assert_eq!(eff.channel.gain_fg.eg.loop_enabled, 1);
        assert!(eff.channel.gain_fg.eg.stages.iter().all(|s| s.curve == 1));
    }

    /// NRPN(0,28) Pitch FG Loopは生値0で無効(0)、非0(=127)で有効(1)になる
    /// （`filter_self_oscillation`と同じ「非0=true」規約）。
    #[test]
    fn nrpn_fg_loop_zero_is_off() {
        let mut st = ChannelState::new(0, false);
        select_nrpn(&mut st, 0, 28);
        st.apply_data_entry(0);
        assert_eq!(st.overrides.pitch_fg_loop, Some(0));
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

    /// RPN(0,2) Channel Coarse Tuningはmsbのみで半音単位、既定64(=無補正)から-64〜+63半音。
    #[test]
    fn rpn_channel_coarse_tuning() {
        let mut st = ChannelState::new(0, false);
        assert_eq!(st.tune_cents(), 0.0);

        select_rpn(&mut st, 0, 2);
        assert_eq!(st.apply_data_entry(70), DataEntryOutcome::StateChanged { voice_update: true }); // +6半音
        assert_eq!(st.tune_coarse, 70);
        assert_eq!(st.tune_cents(), 600.0);
        assert_eq!(st.total_pitch_bend_cents(), 600.0);
    }

    /// RPN(0,1) Channel Fine TuningはCC6(MSB)+CC38(LSB)の14bit、既定8192(=無補正)から±100セント。
    #[test]
    fn rpn_channel_fine_tuning_14bit() {
        let mut st = ChannelState::new(0, false);
        select_rpn(&mut st, 0, 1);
        st.apply_data_entry_lsb(0);
        st.apply_data_entry(64); // msb=64 → 64*128+0 = 8192（中心、無補正）
        assert_eq!(st.tune_fine, 8192);
        assert_eq!(st.tune_cents(), 0.0);

        select_rpn(&mut st, 0, 1);
        st.apply_data_entry(127);
        st.apply_data_entry_lsb(127); // 127*128+127 = 16383（最大）
        assert_eq!(st.tune_fine, 16383);
        // 中心8192・最小0・最大16383は非対称（MIDI 14bit値の一般的な非対称性、ピッチベンドと同じ）
        // なので最大値では+100セントちょうどにはならない（8191/8192*100 ≈ 99.988）。
        assert!((st.tune_cents() - 100.0).abs() < 0.02, "最大値で+100セント付近のはず: {}", st.tune_cents());
    }

    /// Fine/Coarse Tuningは加算され、`total_pitch_bend_cents`はピッチベンドとも独立に合算する。
    #[test]
    fn tune_cents_combines_with_pitch_bend() {
        let mut st = ChannelState::new(0, false);
        select_rpn(&mut st, 0, 2);
        st.apply_data_entry(65); // +1半音=100セント
        st.bend_cents = 50.0;
        assert_eq!(st.total_pitch_bend_cents(), 150.0);
    }

    /// CC121(Reset All Controllers)はチューニング(RPN(0,1)/(0,2))を初期化しない
    /// （`portamento_time`と同じ「チャンネル設定」扱い、演奏コントローラーではないため）。
    #[test]
    fn reset_all_controllers_does_not_clear_tuning() {
        let mut st = ChannelState::new(0, false);
        select_rpn(&mut st, 0, 2);
        st.apply_data_entry(70);
        st.reset_all_controllers();
        assert_eq!(st.tune_coarse, 70, "チューニングはRAC対象外");
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

    /// CC5(Portamento Time)→秒数：0はグライドなし、それ以外は単調増加し127で約5秒。
    #[test]
    fn portamento_seconds_is_zero_at_zero_and_monotonic() {
        let mut st = ChannelState::new(0, false);
        st.portamento_time = 0;
        assert_eq!(st.portamento_seconds(), 0.0);

        st.portamento_time = 1;
        let low = st.portamento_seconds();
        assert!(low > 0.0);

        st.portamento_time = 64;
        let mid = st.portamento_seconds();
        assert!(mid > low);

        st.portamento_time = 127;
        let high = st.portamento_seconds();
        assert!(high > mid);
        assert!((high - 5.0).abs() < 0.1, "127で約5秒のはず: {high}");
    }

    /// `glide_source`はportamento_on・別ノート・秒>0の全条件が揃ったときだけSomeを返す。
    #[test]
    fn glide_source_requires_portamento_on_and_a_different_previous_note() {
        let mut st = ChannelState::new(0, false);
        st.portamento_time = 64;

        // portamento_on=falseならNone
        assert_eq!(st.glide_source(64), None);

        st.portamento_on = true;
        // last_noteが無ければNone（最初のノート）
        assert_eq!(st.glide_source(64), None);

        st.last_note = Some(60);
        let (from, seconds) = st.glide_source(64).expect("グライド対象になるはず");
        assert_eq!(from, 60);
        assert!(seconds > 0.0);

        // 同じノートへの弾き直しはNone
        assert_eq!(st.glide_source(60), None);

        // portamento_time=0（秒<=0）ならNone
        st.portamento_time = 0;
        assert_eq!(st.glide_source(64), None);
    }

    /// CC121(Reset All Controllers)はportamento_onとlast_noteをクリアするが
    /// portamento_timeとmono.enabledは維持する。
    #[test]
    fn reset_all_controllers_clears_portamento_switch_but_not_time_or_mono_mode() {
        let mut st = ChannelState::new(0, false);
        st.portamento_on = true;
        st.portamento_time = 100;
        st.last_note = Some(60);
        st.mono.enabled = true;

        st.reset_all_controllers();

        assert!(!st.portamento_on, "CC65はRAC対象");
        assert_eq!(st.last_note, None);
        assert_eq!(st.portamento_time, 100, "CC5はRAC対象外");
        assert!(st.mono.enabled, "チャンネルモードはRAC対象外");
    }

    /// エフェクト系NRPN(0,2)〜(0,8)は`ChannelState`自身を変化させない
    /// （MasterEffectsは呼び出し側が持つ配列で、`ChannelState`はEffect outcomeを返すのみ）。
    #[test]
    fn effect_nrpn_reports_but_does_not_mutate_state() {
        let mut st = ChannelState::new(0, false);
        let before = st.clone();
        select_nrpn(&mut st, 0, 4); // ReverbTime
        let outcome = st.apply_data_entry(64);
        assert!(matches!(outcome, DataEntryOutcome::Effect(_, EffectControlTarget::ReverbTime, _)));
        // data_entry_msb・rpn.selectionはCC6/NRPN選択の受信状態として当然変化するので、
        // それ以外（overrides等の音色状態、effect_route_slotを含む）が変化していないことを確認する。
        assert_eq!(st.overrides, before.overrides);
        assert_eq!(st.operator_f_number_override, before.operator_f_number_override);
        assert_eq!(st.cc2_destination, before.cc2_destination);
        assert_eq!(st.cc4_destination, before.cc4_destination);
        assert_eq!(st.pitch_bend_range, before.pitch_bend_range);
        assert_eq!(st.effect_route_slot, before.effect_route_slot);
    }

    /// `effect_route_slot`は`ChannelState`ごとに独立する
    /// （チャンネル独立性テスト群、上記の他フィールドと同じ契約凍結の目的）。
    #[test]
    fn effect_route_slot_is_per_channel() {
        let mut ch0 = ChannelState::new(0, false);
        let ch1 = ChannelState::new(1, false);
        select_nrpn(&mut ch0, 0, 1); // Channel Effect Route
        ch0.apply_data_entry(5);
        assert_eq!(ch0.effect_route_slot, 5);
        assert_eq!(ch1.effect_route_slot, 0, "ch1は既定のまま");
    }
}
