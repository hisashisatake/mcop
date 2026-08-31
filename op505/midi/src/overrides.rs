//! NRPNによる「パッチの離散/焼き込みフィールド」の上書きレイヤー（[`PatchOverrides`]）。
//!
//! `ChannelState`だけでなく`op505-vst`からも直接使える小さな値型として独立させてある
//! （VSTのNRPN系シャドウは現状チャンネル別化していないため、`ChannelState`全体への移行を
//! 待たずに挙動を共有するための切り出し）。
//!
//! Program Changeでこのレイヤーを`clear()`する（「PC＝音色を選び直す」「その後のNRPN＝
//! その音色への微調整」という役割分担。詳細はplan「op505-vstとop505-midiのNRPN上書き
//! レイヤー共有化」参照）。
//!
//! `operator_f_number_override`（`ChannelState`の別フィールド）はここに含めない。
//! `apply()`（実効パッチ組み立て）を通らず`Op505Engine::set_operator_f_number`専用APIへ
//! 流れる別経路で、VSTと`ChannelState`の間に元々挙動差が無いため対象外にしている。

use op505_core::Op505Patch;

/// None=ベースパッチ（Program Changeで選ばれた音色、または現在のDAWパラメーター等）の
/// 値のまま。Some=NRPNで上書き済み。
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct PatchOverrides {
    pub algorithm: Option<u8>,
    pub operator_waveforms: [Option<u8>; 4],
    pub filter_type: Option<u8>,
    pub filter_self_oscillation: Option<bool>,
    pub fixed_note_enable: Option<bool>,
    pub fixed_note: Option<u8>,
    pub fixed_note_fine: Option<u8>,
    /// NRPN(0,25)〜(0,27) FG Depth（0〜255）の絶対上書き。CC1/CC77/CC92の演奏補正は
    /// この上書き後の値へさらに加算される（`ChannelState::apply_note_post_processing`参照）。
    pub pitch_fg_depth: Option<u8>,
    pub cutoff_fg_depth: Option<u8>,
    pub gain_fg_depth: Option<u8>,
    /// NRPN(0,28)〜(0,33) FG Loop/Curve（0/1）。`op505-vst`ではTimeEg本体がpersist状態のため、
    /// この上書きは音には即座に反映されるがGUIエディタの表示・Save後のプリセットへは
    /// 反映されない（Algorithm等と同じ既知の制約、spec-sound.md参照）。
    pub pitch_fg_loop: Option<u8>,
    /// Curveは全段一括（`TimeStage::curve`は本来段ごとだが、NRPN 1本で全段へ同じ値を適用する）。
    pub pitch_fg_curve: Option<u8>,
    pub cutoff_fg_loop: Option<u8>,
    pub cutoff_fg_curve: Option<u8>,
    pub gain_fg_loop: Option<u8>,
    pub gain_fg_curve: Option<u8>,
}

impl PatchOverrides {
    /// `base`に上書きレイヤーを重ねる。Noneのフィールドは`base`の値のまま変化しない。
    pub fn apply(&self, patch: &mut Op505Patch) {
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
        if let Some(v) = self.fixed_note_enable {
            patch.channel.fixed_note_enable = v;
        }
        if let Some(v) = self.fixed_note {
            patch.channel.fixed_note = v;
        }
        if let Some(v) = self.fixed_note_fine {
            patch.channel.fixed_note_fine = v;
        }
        if let Some(v) = self.pitch_fg_depth {
            patch.channel.pitch_fg.depth = v;
        }
        if let Some(v) = self.cutoff_fg_depth {
            patch.channel.cutoff_fg.depth = v;
        }
        if let Some(v) = self.gain_fg_depth {
            patch.channel.gain_fg.depth = v;
        }
        if let Some(v) = self.pitch_fg_loop {
            patch.channel.pitch_fg.eg.loop_enabled = v;
        }
        if let Some(v) = self.pitch_fg_curve {
            for stage in patch.channel.pitch_fg.eg.stages.iter_mut() {
                stage.curve = v;
            }
        }
        if let Some(v) = self.cutoff_fg_loop {
            patch.channel.cutoff_fg.eg.loop_enabled = v;
        }
        if let Some(v) = self.cutoff_fg_curve {
            for stage in patch.channel.cutoff_fg.eg.stages.iter_mut() {
                stage.curve = v;
            }
        }
        if let Some(v) = self.gain_fg_loop {
            patch.channel.gain_fg.eg.loop_enabled = v;
        }
        if let Some(v) = self.gain_fg_curve {
            for stage in patch.channel.gain_fg.eg.stages.iter_mut() {
                stage.curve = v;
            }
        }
    }

    /// 上書きレイヤーを全て解除する（Program Change・System Reset時に呼ぶ）。
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_neutral_leaves_base_unchanged() {
        let base = Op505Patch::default();
        let mut patch = base;
        PatchOverrides::default().apply(&mut patch);
        assert_eq!(patch, base);
    }

    #[test]
    fn apply_overrides_replace_targeted_fields_only() {
        let base = Op505Patch::default();
        let mut patch = base;
        let overrides = PatchOverrides {
            algorithm: Some(5),
            operator_waveforms: [Some(9), None, Some(3), None],
            filter_type: Some(2),
            filter_self_oscillation: Some(false),
            fixed_note_enable: Some(true),
            fixed_note: Some(60),
            fixed_note_fine: Some(200),
            pitch_fg_depth: Some(210),
            cutoff_fg_depth: Some(220),
            gain_fg_depth: Some(230),
            pitch_fg_loop: Some(1),
            pitch_fg_curve: Some(1),
            cutoff_fg_loop: Some(1),
            cutoff_fg_curve: Some(1),
            gain_fg_loop: Some(1),
            gain_fg_curve: Some(1),
        };
        overrides.apply(&mut patch);
        assert_eq!(patch.channel.algorithm, 5);
        assert_eq!(patch.operators[0].waveform, 9);
        assert_eq!(patch.operators[1].waveform, base.operators[1].waveform);
        assert_eq!(patch.operators[2].waveform, 3);
        assert_eq!(patch.operators[3].waveform, base.operators[3].waveform);
        assert_eq!(patch.channel.filter_type, 2);
        assert!(!patch.channel.filter_self_oscillation);
        assert!(patch.channel.fixed_note_enable);
        assert_eq!(patch.channel.fixed_note, 60);
        assert_eq!(patch.channel.fixed_note_fine, 200);
        assert_eq!(patch.channel.pitch_fg.depth, 210);
        assert_eq!(patch.channel.cutoff_fg.depth, 220);
        assert_eq!(patch.channel.gain_fg.depth, 230);
        assert_eq!(patch.channel.pitch_fg.eg.loop_enabled, 1);
        assert!(patch.channel.pitch_fg.eg.stages.iter().all(|s| s.curve == 1));
        assert_eq!(patch.channel.cutoff_fg.eg.loop_enabled, 1);
        assert!(patch.channel.cutoff_fg.eg.stages.iter().all(|s| s.curve == 1));
        assert_eq!(patch.channel.gain_fg.eg.loop_enabled, 1);
        assert!(patch.channel.gain_fg.eg.stages.iter().all(|s| s.curve == 1));
    }

    #[test]
    fn clear_resets_all_fields_to_none() {
        let mut overrides = PatchOverrides {
            algorithm: Some(5),
            operator_waveforms: [Some(9); 4],
            filter_type: Some(2),
            filter_self_oscillation: Some(false),
            fixed_note_enable: Some(true),
            fixed_note: Some(60),
            fixed_note_fine: Some(200),
            pitch_fg_depth: Some(210),
            cutoff_fg_depth: Some(220),
            gain_fg_depth: Some(230),
            pitch_fg_loop: Some(1),
            pitch_fg_curve: Some(1),
            cutoff_fg_loop: Some(1),
            cutoff_fg_curve: Some(1),
            gain_fg_loop: Some(1),
            gain_fg_curve: Some(1),
        };
        overrides.clear();
        assert_eq!(overrides, PatchOverrides::default());
    }
}
