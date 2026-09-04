//! パネルパラメーター（DAWパラメーター相当）の識別enumと、min/max/default/表示名の正本。
//!
//! 正本をここに置く理由: エディタのdefaultは「鳴る値」（TL=200等）で、
//! `Op505OperatorParams::default()`（TL=0＝無音）とは意図的に別物のため、op505-coreに
//! 置くと2種類のdefaultが同居して混乱する。
//!
//! パラメーター識別は`_ =>`を使わない全列挙enumにする（`op505_midi::ControlTarget`と
//! 同じ方針）。表示名は`short_name`（ノブのツールチップ用、standalone側で変更可）と
//! `daw_name`（DAWオートメーション一覧用、**絶対に変更しない**）の2列で持つ——実際に
//! 食い違うのはオペレーターの5個（VelSens/OpFineTune/EgShift/LevelScale/VelocityGain）
//! だけだが、統一はしない（`daw_name`を変えると既存DAWプロジェクトのオートメーション
//! 対応が壊れるため）。

/// 4オペレーターのインデックス。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpIndex {
    Op1,
    Op2,
    Op3,
    Op4,
}

impl OpIndex {
    pub const ALL: [OpIndex; 4] = [OpIndex::Op1, OpIndex::Op2, OpIndex::Op3, OpIndex::Op4];

    /// 配列インデックス（0〜3）。`Op505Patch::operators[i.index()]`で使う。
    pub const fn index(self) -> usize {
        match self {
            OpIndex::Op1 => 0,
            OpIndex::Op2 => 1,
            OpIndex::Op3 => 2,
            OpIndex::Op4 => 3,
        }
    }
}

/// 3本のFG（Pitch/Cutoff/Gain各種変調）系統。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FgSlot {
    Pitch,
    Cutoff,
    Gain,
}

impl FgSlot {
    pub const ALL: [FgSlot; 3] = [FgSlot::Pitch, FgSlot::Cutoff, FgSlot::Gain];
}

/// オペレーター単位のintパラメーター10種。DAW名がOP1〜4で共通のため、`spec()`は
/// オペレーター番号に依存しない（`PatchInt::Op(_, f).spec() == f.spec()`が成り立つ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpInt {
    Tl,
    Mul,
    Dt1,
    Ksr,
    VelSens,
    OpFineTune,
    Waveform,
    EgShift,
    LevelScale,
    VelocityGain,
}

impl OpInt {
    pub const ALL: [OpInt; 10] = [
        OpInt::Tl,
        OpInt::Mul,
        OpInt::Dt1,
        OpInt::Ksr,
        OpInt::VelSens,
        OpInt::OpFineTune,
        OpInt::Waveform,
        OpInt::EgShift,
        OpInt::LevelScale,
        OpInt::VelocityGain,
    ];

    pub const fn spec(self) -> IntSpec {
        match self {
            OpInt::Tl => IntSpec { min: 0, max: 255, default: 200, short_name: "TL", daw_name: "TL", daw_bipolar: false },
            OpInt::Mul => IntSpec { min: 0, max: 15, default: 1, short_name: "MUL", daw_name: "MUL", daw_bipolar: false },
            OpInt::Dt1 => IntSpec { min: 0, max: 255, default: 128, short_name: "DT1", daw_name: "DT1", daw_bipolar: true },
            OpInt::Ksr => IntSpec { min: 0, max: 255, default: 64, short_name: "KSR", daw_name: "KSR", daw_bipolar: false },
            OpInt::VelSens => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "VEL", daw_name: "Velocity Sensitivity", daw_bipolar: false }
            }
            OpInt::OpFineTune => {
                IntSpec { min: 0, max: 255, default: 128, short_name: "FINE", daw_name: "Op Fine Tune", daw_bipolar: true }
            }
            OpInt::Waveform => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "Waveform", daw_name: "Waveform", daw_bipolar: false }
            }
            OpInt::EgShift => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "EGSFT", daw_name: "Op EG Shift", daw_bipolar: false }
            }
            OpInt::LevelScale => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "LEVEL SCALE", daw_name: "Op Level Scale", daw_bipolar: false }
            }
            OpInt::VelocityGain => {
                IntSpec { min: 0, max: 255, default: 255, short_name: "V.GAIN", daw_name: "Op Velocity Gain", daw_bipolar: false }
            }
        }
    }
}

/// チャンネル単位＋オペレーター単位のintパラメーター（MASTER EFFECTSを除く50個）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatchInt {
    Algorithm,
    Feedback,
    FixedNote,
    FixedNoteFine,
    Cutoff,
    Resonance,
    FilterType,
    FgDepth(FgSlot),
    Op(OpIndex, OpInt),
}

impl PatchInt {
    /// 50個（チャンネル単一7 + FgDepth3 + Op(4×10)40）を全列挙する。
    pub fn all() -> Vec<PatchInt> {
        let mut out = vec![
            PatchInt::Algorithm,
            PatchInt::Feedback,
            PatchInt::FixedNote,
            PatchInt::FixedNoteFine,
            PatchInt::Cutoff,
            PatchInt::Resonance,
            PatchInt::FilterType,
        ];
        out.extend(FgSlot::ALL.into_iter().map(PatchInt::FgDepth));
        for op in OpIndex::ALL {
            out.extend(OpInt::ALL.into_iter().map(|f| PatchInt::Op(op, f)));
        }
        out
    }

    pub const fn spec(self) -> IntSpec {
        match self {
            PatchInt::Algorithm => {
                IntSpec { min: 0, max: 7, default: 0, short_name: "Algorithm", daw_name: "Algorithm", daw_bipolar: false }
            }
            PatchInt::Feedback => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "Feedback", daw_name: "Feedback", daw_bipolar: false }
            }
            PatchInt::FixedNote => {
                IntSpec { min: 0, max: 127, default: 60, short_name: "Fixed Note", daw_name: "Fixed Note", daw_bipolar: false }
            }
            PatchInt::FixedNoteFine => IntSpec {
                min: 0,
                max: 255,
                default: 128,
                short_name: "Fixed Note Fine",
                daw_name: "Fixed Note Fine",
                daw_bipolar: true,
            },
            PatchInt::Cutoff => {
                IntSpec { min: 0, max: 255, default: 255, short_name: "Filter Cutoff", daw_name: "Filter Cutoff", daw_bipolar: false }
            }
            PatchInt::Resonance => IntSpec {
                min: 0,
                max: 255,
                default: 0,
                short_name: "Filter Resonance",
                daw_name: "Filter Resonance",
                daw_bipolar: false,
            },
            PatchInt::FilterType => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "Filter Type", daw_name: "Filter Type", daw_bipolar: false }
            }
            PatchInt::FgDepth(FgSlot::Pitch) => IntSpec {
                min: 0,
                max: 255,
                default: 0,
                short_name: "Pitch FG Depth",
                daw_name: "Pitch FG Depth",
                daw_bipolar: false,
            },
            PatchInt::FgDepth(FgSlot::Cutoff) => IntSpec {
                min: 0,
                max: 255,
                default: 0,
                short_name: "Cutoff FG Depth",
                daw_name: "Cutoff FG Depth",
                daw_bipolar: false,
            },
            // Gain FGだけ既定255（旧仕様と完全一致するための「EGの形をそのまま使う」既定値）。
            // Pitch/Cutoff FGの既定0（無変調）とは意味が異なる（op505-vst params.rsと同じ配慮）。
            PatchInt::FgDepth(FgSlot::Gain) => IntSpec {
                min: 0,
                max: 255,
                default: 255,
                short_name: "Gain FG Depth",
                daw_name: "Gain FG Depth",
                daw_bipolar: false,
            },
            PatchInt::Op(_, f) => f.spec(),
        }
    }
}

/// MASTER EFFECTS（Reverb/Chorus）+ マスターボリュームのintパラメーター10個。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FxInt {
    RevSend,
    ReverbType,
    ReverbTime,
    ChoSend,
    ChorusType,
    ChorusModRate,
    ChorusModDepth,
    ChorusFeedback,
    ChorusSendToReverb,
    MasterVolume,
    DelaySync,
    DelaySyncRate,
}

impl FxInt {
    /// `shared::FX_*`定数（standalone側）と同じ順序で保つこと——ずれるとreverb_typeと
    /// reverb_timeが入れ替わって無音デバッグ地獄になる（Step 7で相互検証テストを追加する）。
    /// `MasterVolume`・`DelaySync`・`DelaySyncRate`は既存9個の後に追加した欄のため、
    /// 既存の並びを保つよう**末尾に追加**する（途中に挿入すると`shared::FX_*`の数値との
    /// 対応が全てずれる）。
    pub const ALL: [FxInt; 12] = [
        FxInt::RevSend,
        FxInt::ReverbType,
        FxInt::ReverbTime,
        FxInt::ChoSend,
        FxInt::ChorusType,
        FxInt::ChorusModRate,
        FxInt::ChorusModDepth,
        FxInt::ChorusFeedback,
        FxInt::ChorusSendToReverb,
        FxInt::MasterVolume,
        FxInt::DelaySync,
        FxInt::DelaySyncRate,
    ];

    pub const fn spec(self) -> IntSpec {
        match self {
            FxInt::RevSend => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "Reverb Send", daw_name: "Reverb Send", daw_bipolar: false }
            }
            FxInt::ReverbType => {
                IntSpec { min: 0, max: 7, default: 3, short_name: "Reverb Type", daw_name: "Reverb Type", daw_bipolar: false }
            }
            FxInt::ReverbTime => {
                IntSpec { min: 0, max: 255, default: 128, short_name: "Reverb Time", daw_name: "Reverb Time", daw_bipolar: false }
            }
            FxInt::ChoSend => {
                IntSpec { min: 0, max: 255, default: 0, short_name: "Chorus Send", daw_name: "Chorus Send", daw_bipolar: false }
            }
            FxInt::ChorusType => {
                IntSpec { min: 0, max: 7, default: 0, short_name: "Chorus Type", daw_name: "Chorus Type", daw_bipolar: false }
            }
            FxInt::ChorusModRate => IntSpec {
                min: 0,
                max: 255,
                default: 128,
                short_name: "Chorus Mod Rate",
                daw_name: "Chorus Mod Rate",
                daw_bipolar: false,
            },
            FxInt::ChorusModDepth => IntSpec {
                min: 0,
                max: 255,
                default: 128,
                short_name: "Chorus Mod Depth",
                daw_name: "Chorus Mod Depth",
                daw_bipolar: false,
            },
            FxInt::ChorusFeedback => IntSpec {
                min: 0,
                max: 255,
                default: 0,
                short_name: "Chorus Feedback",
                daw_name: "Chorus Feedback",
                daw_bipolar: false,
            },
            FxInt::ChorusSendToReverb => IntSpec {
                min: 0,
                max: 255,
                default: 0,
                short_name: "Chorus Send To Reverb",
                daw_name: "Chorus Send To Reverb",
                daw_bipolar: false,
            },
            // 既定255＝無補正（MasterOutputの既定と一致させ、初回起動時に音量が変わって
            // 聞こえる回帰を防ぐ）。
            FxInt::MasterVolume => IntSpec {
                min: 0,
                max: 255,
                default: 255,
                short_name: "Master Volume",
                daw_name: "Master Volume",
                daw_bipolar: false,
            },
            // Delay/Panning Delayのテンポ同期（NRPN(0,36)/(0,37)、詳細はspec-sound.md
            // 「マスターエフェクト」節）。Room1〜Plateタイプには効果がない。
            FxInt::DelaySync => {
                IntSpec { min: 0, max: 1, default: 0, short_name: "Delay Sync", daw_name: "Delay Sync", daw_bipolar: false }
            }
            FxInt::DelaySyncRate => IntSpec {
                min: 0,
                max: 255,
                // TimeEgのsync_rate既定と同じ1/4アンカー。
                default: sound_core::sync_note_anchor(10) as i32,
                short_name: "Delay Sync Rate",
                daw_name: "Delay Sync Rate",
                daw_bipolar: false,
            },
        }
    }
}

/// intパラメーターの全識別子（`Op505Patch`側の50個 + MASTER EFFECTS+マスターボリューム側の
/// 10個＝60個）。これらが`Op505Patch`外である事実をバリアント分割で型に載せる
/// （`Op505Patch`への読み書き関数は`PatchInt`だけを網羅すればよく、取り違えるとコンパイルエラーになる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntField {
    Patch(PatchInt),
    Fx(FxInt),
}

impl IntField {
    pub fn all() -> Vec<IntField> {
        let mut out: Vec<IntField> = PatchInt::all().into_iter().map(IntField::Patch).collect();
        out.extend(FxInt::ALL.into_iter().map(IntField::Fx));
        out
    }

    pub const fn spec(self) -> IntSpec {
        match self {
            IntField::Patch(p) => p.spec(),
            IntField::Fx(f) => f.spec(),
        }
    }
}

/// boolパラメーター8個（`FixedNoteEnable`/`FilterSelfOscillation`/`GainFgToMaster`/
/// `GainFgToOperators`/オペレーター単位の`Ame`×4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoolField {
    FixedNoteEnable,
    FilterSelfOscillation,
    GainFgToMaster,
    GainFgToOperators,
    Ame(OpIndex),
}

impl BoolField {
    pub const ALL: [BoolField; 8] = [
        BoolField::FixedNoteEnable,
        BoolField::FilterSelfOscillation,
        BoolField::GainFgToMaster,
        BoolField::GainFgToOperators,
        BoolField::Ame(OpIndex::Op1),
        BoolField::Ame(OpIndex::Op2),
        BoolField::Ame(OpIndex::Op3),
        BoolField::Ame(OpIndex::Op4),
    ];

    pub const fn spec(self) -> BoolSpec {
        match self {
            BoolField::FixedNoteEnable => BoolSpec { default: false, daw_name: "Fixed Note Enable" },
            BoolField::FilterSelfOscillation => BoolSpec { default: true, daw_name: "Filter Self-Oscillation" },
            BoolField::GainFgToMaster => BoolSpec { default: true, daw_name: "Gain FG to Master" },
            BoolField::GainFgToOperators => BoolSpec { default: false, daw_name: "Gain FG to Operators" },
            BoolField::Ame(_) => BoolSpec { default: false, daw_name: "AM Enable" },
        }
    }
}

/// TimeEg（7本＝OP1〜4 EG + Pitch/Cutoff/Gain FG）の識別子。`EgSlot::name()`が
/// `op505/vst/src/editor.rs`と`op505/standalone/src/editor/panel_params.rs`に重複していた
/// `match I { 0 => "OP1 EG", ... }`の一元化先。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EgSlot {
    Op(OpIndex),
    Fg(FgSlot),
}

impl EgSlot {
    pub const ALL: [EgSlot; 7] = [
        EgSlot::Op(OpIndex::Op1),
        EgSlot::Op(OpIndex::Op2),
        EgSlot::Op(OpIndex::Op3),
        EgSlot::Op(OpIndex::Op4),
        EgSlot::Fg(FgSlot::Pitch),
        EgSlot::Fg(FgSlot::Cutoff),
        EgSlot::Fg(FgSlot::Gain),
    ];

    pub const fn name(self) -> &'static str {
        match self {
            EgSlot::Op(OpIndex::Op1) => "OP1 EG",
            EgSlot::Op(OpIndex::Op2) => "OP2 EG",
            EgSlot::Op(OpIndex::Op3) => "OP3 EG",
            EgSlot::Op(OpIndex::Op4) => "OP4 EG",
            EgSlot::Fg(FgSlot::Pitch) => "PITCH FG",
            EgSlot::Fg(FgSlot::Cutoff) => "CUTOFF FG",
            EgSlot::Fg(FgSlot::Gain) => "GAIN FG",
        }
    }
}

/// intパラメーター1個ぶんの定義値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntSpec {
    pub min: i32,
    pub max: i32,
    pub default: i32,
    /// ノブのツールチップ用（standalone側、変更可）。
    pub short_name: &'static str,
    /// DAWオートメーション一覧用。**絶対に変更しない**（既存DAWプロジェクトのオートメーション対応が壊れる）。
    pub daw_name: &'static str,
    /// `bipolar_int()`（DAWの値表示を-128〜+127へ写像する処理）の適用対象か。
    /// 対象はDt1/OpFineTune/FixedNoteFineの3つ。
    pub daw_bipolar: bool,
}

/// boolパラメーター1個ぶんの定義値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoolSpec {
    pub default: bool,
    pub daw_name: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_int_fields_have_valid_range() {
        for field in IntField::all() {
            let spec = field.spec();
            assert!(
                spec.min <= spec.default && spec.default <= spec.max,
                "{field:?} の spec が min<=default<=max を満たさない: {spec:?}"
            );
        }
    }

    #[test]
    fn enum_counts_match_plan() {
        assert_eq!(IntField::all().len(), 62, "IntField（Patch 50 + Fx 12）");
        assert_eq!(BoolField::ALL.len(), 8, "BoolField（単一4 + Ame×4）");
        assert_eq!(EgSlot::ALL.len(), 7, "EgSlot（Op×4 + Fg×3）");
    }

    #[test]
    fn eg_slot_names_are_seven_distinct_values() {
        let names: HashSet<&'static str> = EgSlot::ALL.iter().map(|slot| slot.name()).collect();
        assert_eq!(
            names,
            HashSet::from(["OP1 EG", "OP2 EG", "OP3 EG", "OP4 EG", "PITCH FG", "CUTOFF FG", "GAIN FG"])
        );
    }

    #[test]
    fn op_field_spec_is_operator_independent() {
        for op in OpIndex::ALL {
            for f in OpInt::ALL {
                assert_eq!(
                    PatchInt::Op(op, f).spec(),
                    f.spec(),
                    "{op:?}/{f:?} の spec がオペレーター非依存であるべき"
                );
            }
        }
    }

    #[test]
    fn op_index_to_array_index_is_bijective() {
        let indices: HashSet<usize> = OpIndex::ALL.iter().map(|op| op.index()).collect();
        assert_eq!(indices, HashSet::from([0, 1, 2, 3]));
    }
}
