use nice_plug::prelude::*;

pub(crate) const DEFAULT_REVERB_TIME: u8 = 128;
pub(crate) const DEFAULT_CHORUS_MOD_RATE: u8 = 128;
pub(crate) const DEFAULT_CHORUS_MOD_DEPTH: u8 = 128;
pub(crate) const DEFAULT_CHORUS_FEEDBACK: u8 = 0;
pub(crate) const DEFAULT_CHORUS_SEND_TO_REVERB: u8 = 0;
pub(crate) const DEFAULT_ALGORITHM: u8 = 0;
/// ReverbType::default()（Hall1）の宣言順インデックス。
pub(crate) const DEFAULT_REVERB_TYPE: u8 = 3;
/// ChorusType::default()（Chorus1）の宣言順インデックス。
pub(crate) const DEFAULT_CHORUS_TYPE: u8 = 0;

/// オペレーター単位パラメーター一式（13個）。`Ym38x6Params`側で`[OperatorVstParams; 4]`として
/// `#[nested(array, ...)]`展開し、各IDに`_1`〜`_4`が付与される（DAW上は「Operator 1」〜「Operator 4」）。
#[derive(Params)]
pub(crate) struct OperatorVstParams {
    #[id = "tl"]
    pub tl: IntParam,
    #[id = "ar"]
    pub ar: IntParam,
    #[id = "d1r"]
    pub d1r: IntParam,
    #[id = "d2r"]
    pub d2r: IntParam,
    #[id = "d1l"]
    pub d1l: IntParam,
    #[id = "rr"]
    pub rr: IntParam,
    #[id = "mul"]
    pub mul: IntParam,
    #[id = "dt1"]
    pub dt1: IntParam,
    #[id = "ksr"]
    pub ksr: IntParam,
    #[id = "ame"]
    pub ame: BoolParam,
    #[id = "vel_sens"]
    pub vel_sens: IntParam,
    #[id = "op_fine"]
    pub op_fine_tune: IntParam,
    #[id = "wf"]
    pub waveform: IntParam,
    /// ループ時の折り返しの底レベル(0〜255、既定0＝完全開閉)。`loop`がOFFの間は無効。
    #[id = "op_floor"]
    pub floor: IntParam,
    /// OP単位ループEG（VCF/VCAのFGと同じLoop/Floor/Curve機構をFMオペレーターEGに開放したもの）。
    /// OFF(既定)では従来の5段ADSRと完全に同一の挙動になる。
    #[id = "op_loop"]
    pub op_loop: BoolParam,
    /// 0=線形（角の立つ三角）/1=サイン風（レイズドコサインで角を丸める）。
    #[id = "op_curve"]
    pub curve: BoolParam,
    /// EGSFT（TX81Z EG Shift）。EGの減衰レンジ(dB)を圧縮する（0〜255、既定0＝96dBフルレンジ）。
    #[id = "op_eg_shift"]
    pub eg_shift: IntParam,
    /// Level Scaling（ノート依存の出力レベル減衰、OPL系KSL相当）。0〜255、既定0＝スケーリングなし。
    #[id = "op_level_scale"]
    pub level_scale: IntParam,
    /// キャリア出力へのベロシティ音量ゲイン深さ（0〜255、既定255＝フル）。
    /// モジュレーターでは無視される（`vel_sens`＝明るさとは独立・別軸、役割はALGで決まる）。
    #[id = "op_vel_gain"]
    pub velocity_gain: IntParam,
}

impl Default for OperatorVstParams {
    /// 「鳴る」状態を初期値とする（コアの`OperatorParams::default()`は全0で
    /// TL=0≒無音・AR=0≒極端に遅いアタックのため、VST起動直後に無音にならないよう
    /// 個別に明示値を設定する）。
    fn default() -> Self {
        Self {
            tl: IntParam::new("TL", 200, IntRange::Linear { min: 0, max: 255 }),
            ar: IntParam::new("AR", 255, IntRange::Linear { min: 0, max: 255 }),
            d1r: IntParam::new("D1R", 100, IntRange::Linear { min: 0, max: 255 }),
            d2r: IntParam::new("D2R", 80, IntRange::Linear { min: 0, max: 255 }),
            d1l: IntParam::new("D1L", 180, IntRange::Linear { min: 0, max: 255 }),
            rr: IntParam::new("RR", 150, IntRange::Linear { min: 0, max: 255 }),
            mul: IntParam::new("MUL", 1, IntRange::Linear { min: 0, max: 15 }),
            dt1: IntParam::new("DT1", 128, IntRange::Linear { min: 0, max: 255 }),
            ksr: IntParam::new("KSR", 64, IntRange::Linear { min: 0, max: 255 }),
            ame: BoolParam::new("AM Enable", false),
            // Velocity Sensitivityは「明るさ」専用のopt-inパラメーター（デフォルト0）。
            // ベロシティ→音量は常時ONなので、0でも素のパッチでベロシティの表情は出る。
            vel_sens: IntParam::new("Velocity Sensitivity", 0, IntRange::Linear { min: 0, max: 255 }),
            // 中心128＝オフセットなし（±1オクターブ）。DT1で足りない広いデチューン用
            op_fine_tune: IntParam::new("Op Fine Tune", 128, IntRange::Linear { min: 0, max: 255 }),
            waveform: IntParam::new("Waveform", 0, IntRange::Linear { min: 0, max: 255 }),
            // 既定OFF/0＝従来の5段ADSRと完全に同一の挙動（後方互換）。
            floor: IntParam::new("Op Loop Floor", 0, IntRange::Linear { min: 0, max: 255 }),
            op_loop: BoolParam::new("Op Loop", false),
            curve: BoolParam::new("Op Loop Curve", false),
            // 既定0＝EGSFTオフ（96dBフルレンジ、従来挙動そのまま）。
            eg_shift: IntParam::new("Op EG Shift", 0, IntRange::Linear { min: 0, max: 255 }),
            // 既定0＝Level Scalingオフ（ノート依存の出力減衰なし、従来挙動そのまま）。
            level_scale: IntParam::new("Op Level Scale", 0, IntRange::Linear { min: 0, max: 255 }),
            // 既定255＝フル（旧チャンネル一括velocity/127と数学的に同一、従来挙動そのまま）。
            velocity_gain: IntParam::new("Op Velocity Gain", 255, IntRange::Linear { min: 0, max: 255 }),
        }
    }
}

#[derive(Params)]
pub(crate) struct Ym38x6Params {
    // ---- チャンネル単位（48個、spec-sound.md MIDI実装方針参照。
    // Pitch/Cutoff/Gain FG各Delayの3個は仕様書記載の45個から今回追加） ----
    #[id = "algorithm"]
    pub algorithm: IntParam,
    #[id = "feedback"]
    pub feedback: IntParam,

    // 質感LFO（7個。旧パフォーマンスLFOのDAWパラメーター。CC補正は受けない焼き込み専用）
    #[id = "texture_lfo_rate"]
    pub texture_lfo_rate: IntParam,
    #[id = "texture_lfo_depth"]
    pub texture_lfo_depth: IntParam,
    #[id = "texture_lfo_delay"]
    pub texture_lfo_delay: IntParam,
    /// 質感LFOの5波形パレット(0=矩形/1=台形/2=S&H/3=Random/4=Chaos)へ直接対応する。
    #[id = "texture_lfo_waveform"]
    pub texture_lfo_waveform: IntParam,
    #[id = "texture_lfo_fade_mode"]
    pub texture_lfo_fade_mode: IntParam,
    #[id = "texture_lfo_fade_time"]
    pub texture_lfo_fade_time: IntParam,
    #[id = "texture_lfo_offset"]
    pub texture_lfo_offset: IntParam,
    /// 質感LFOの行き先(0=Pitch/1=Volume/2=TL/3=Cutoff、Ym38x6LfoDestitationと同じ並び)。
    /// NRPN(0,0)と共存（algorithm等と同じ1シャドウ差分検知方式、lib.rs参照）。
    #[id = "texture_lfo_destination"]
    pub texture_lfo_destination: IntParam,

    // チップ内LFO（4個）
    #[id = "chip_lfo_freq"]
    pub chip_lfo_freq: IntParam,
    #[id = "chip_lfo_pmd"]
    pub chip_lfo_pmd: IntParam,
    #[id = "chip_lfo_amd"]
    pub chip_lfo_amd: IntParam,
    #[id = "chip_lfo_delay"]
    pub chip_lfo_delay: IntParam,
    #[id = "pms"]
    pub pms: IntParam,
    #[id = "ams"]
    pub ams: IntParam,

    #[id = "cutoff"]
    pub cutoff: IntParam,
    #[id = "resonance"]
    pub resonance: IntParam,

    // Pitch FG（10個。CC1/76/77/78で②③層の補正を受ける唯一のFGスロット）
    #[id = "pitch_fg_ar"]
    pub pitch_fg_ar: IntParam,
    #[id = "pitch_fg_d1r"]
    pub pitch_fg_d1r: IntParam,
    #[id = "pitch_fg_d1l"]
    pub pitch_fg_d1l: IntParam,
    #[id = "pitch_fg_d2r"]
    pub pitch_fg_d2r: IntParam,
    #[id = "pitch_fg_rr"]
    pub pitch_fg_rr: IntParam,
    /// バイポーラDepth(0〜255、中心128＝変調なし)。
    #[id = "pitch_fg_depth"]
    pub pitch_fg_depth: IntParam,
    #[id = "pitch_fg_floor"]
    pub pitch_fg_floor: IntParam,
    /// キーオンからAR開始までの遅延(0〜255)。CC78の64中心相対補正対象。
    #[id = "pitch_fg_delay"]
    pub pitch_fg_delay: IntParam,
    #[id = "pitch_fg_loop"]
    pub pitch_fg_loop: BoolParam,
    #[id = "pitch_fg_curve"]
    pub pitch_fg_curve: BoolParam,

    // Cutoff FG（10個。旧Filter EG）
    #[id = "cutoff_fg_ar"]
    pub cutoff_fg_ar: IntParam,
    #[id = "cutoff_fg_d1r"]
    pub cutoff_fg_d1r: IntParam,
    #[id = "cutoff_fg_d1l"]
    pub cutoff_fg_d1l: IntParam,
    #[id = "cutoff_fg_d2r"]
    pub cutoff_fg_d2r: IntParam,
    #[id = "cutoff_fg_rr"]
    pub cutoff_fg_rr: IntParam,
    /// バイポーラDepth(0〜255、中心128＝変調なし)。コア`BipolarFg::depth`へ直接コピーする
    /// （旧unipolar Filter EG Depthの変換式は撤去済み）。
    #[id = "cutoff_fg_depth"]
    pub cutoff_fg_depth: IntParam,
    #[id = "cutoff_fg_floor"]
    pub cutoff_fg_floor: IntParam,
    #[id = "cutoff_fg_delay"]
    pub cutoff_fg_delay: IntParam,
    #[id = "cutoff_fg_loop"]
    pub cutoff_fg_loop: BoolParam,
    #[id = "cutoff_fg_curve"]
    pub cutoff_fg_curve: BoolParam,

    // Gain FG（9個。旧VCA EG、Depthなし＝Floorが深さ役）
    #[id = "gain_fg_ar"]
    pub gain_fg_ar: IntParam,
    #[id = "gain_fg_d1r"]
    pub gain_fg_d1r: IntParam,
    #[id = "gain_fg_d1l"]
    pub gain_fg_d1l: IntParam,
    #[id = "gain_fg_d2r"]
    pub gain_fg_d2r: IntParam,
    #[id = "gain_fg_rr"]
    pub gain_fg_rr: IntParam,
    #[id = "gain_fg_floor"]
    pub gain_fg_floor: IntParam,
    #[id = "gain_fg_delay"]
    pub gain_fg_delay: IntParam,
    #[id = "gain_fg_loop"]
    pub gain_fg_loop: BoolParam,
    #[id = "gain_fg_curve"]
    pub gain_fg_curve: BoolParam,

    #[id = "rev_send"]
    pub rev_send: IntParam,
    #[id = "cho_send"]
    pub cho_send: IntParam,

    // ---- オペレーター単位（12個 × 4op = 48個） ----
    #[nested(array, group = "Operator")]
    pub operators: [OperatorVstParams; 4],

    // ---- マスター単位（7個、MasterEffectsのパラメーター+Reverb/Chorus Typeに対応） ----
    #[id = "rev_type"]
    pub reverb_type: IntParam,
    #[id = "rev_time"]
    pub reverb_time: IntParam,
    #[id = "cho_type"]
    pub chorus_type: IntParam,
    #[id = "cho_rate"]
    pub chorus_mod_rate: IntParam,
    #[id = "cho_depth"]
    pub chorus_mod_depth: IntParam,
    #[id = "cho_fb"]
    pub chorus_feedback: IntParam,
    #[id = "cho_to_rev"]
    pub chorus_send_to_reverb: IntParam,
}

impl Default for Ym38x6Params {
    fn default() -> Self {
        Self {
            algorithm: IntParam::new("Algorithm", DEFAULT_ALGORITHM as i32, IntRange::Linear { min: 0, max: 7 }),
            feedback: IntParam::new("Feedback", 0, IntRange::Linear { min: 0, max: 255 }),

            texture_lfo_rate: IntParam::new("Texture LFO Rate", 0, IntRange::Linear { min: 0, max: 255 }),
            texture_lfo_depth: IntParam::new("Texture LFO Depth", 0, IntRange::Linear { min: 0, max: 255 }),
            texture_lfo_delay: IntParam::new("Texture LFO Delay", 0, IntRange::Linear { min: 0, max: 255 }),
            // 質感LFO5波形（0=矩形〜4=Chaos）。
            texture_lfo_waveform: IntParam::new("Texture LFO Waveform", 0, IntRange::Linear { min: 0, max: 4 }),
            // 宣言順インデックス(0=OnIn〜3=OffOut、lfo_fade_mode_from_index参照)。
            texture_lfo_fade_mode: IntParam::new("Texture LFO Fade Mode", 0, IntRange::Linear { min: 0, max: 3 }),
            // fade_time=0はフェード無効（旧来のハードエッジ挙動と等価）。
            texture_lfo_fade_time: IntParam::new("Texture LFO Fade Time", 0, IntRange::Linear { min: 0, max: 255 }),
            // 中心128＝オフセットなし（op_fine_tune等と同じ中心128の慣習）。
            texture_lfo_offset: IntParam::new("Texture LFO Offset", 128, IntRange::Linear { min: 0, max: 255 }),
            // 0=Pitch/1=Volume/2=TL/3=Cutoff。既定0＝Pitch（NRPN(0,0)の既定と一致）。
            texture_lfo_destination: IntParam::new("Texture LFO Destination", 0, IntRange::Linear { min: 0, max: 3 }),

            chip_lfo_freq: IntParam::new("Chip LFO Freq", 0, IntRange::Linear { min: 0, max: 255 }),
            chip_lfo_pmd: IntParam::new("Chip LFO PMD", 0, IntRange::Linear { min: 0, max: 255 }),
            chip_lfo_amd: IntParam::new("Chip LFO AMD", 0, IntRange::Linear { min: 0, max: 255 }),
            chip_lfo_delay: IntParam::new("Chip LFO Delay", 0, IntRange::Linear { min: 0, max: 255 }),
            pms: IntParam::new("PMS", 0, IntRange::Linear { min: 0, max: 255 }),
            ams: IntParam::new("AMS", 0, IntRange::Linear { min: 0, max: 255 }),

            cutoff: IntParam::new("Filter Cutoff", 255, IntRange::Linear { min: 0, max: 255 }),
            resonance: IntParam::new("Filter Resonance", 0, IntRange::Linear { min: 0, max: 255 }),

            // Pitch FG既定＝ym38x6-core::default_pitch_fg()と一致（「無効」状態）。
            pitch_fg_ar: IntParam::new("Pitch FG AR", 0, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_d1r: IntParam::new("Pitch FG D1R", 0, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_d1l: IntParam::new("Pitch FG D1L", 255, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_d2r: IntParam::new("Pitch FG D2R", 0, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_rr: IntParam::new("Pitch FG RR", 255, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_depth: IntParam::new("Pitch FG Depth", 128, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_floor: IntParam::new("Pitch FG Floor", 0, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_delay: IntParam::new("Pitch FG Delay", 0, IntRange::Linear { min: 0, max: 255 }),
            pitch_fg_loop: BoolParam::new("Pitch FG Loop", false),
            pitch_fg_curve: BoolParam::new("Pitch FG Curve", false),

            // Cutoff FG既定＝sound_core::BipolarFg::default()と一致。
            cutoff_fg_ar: IntParam::new("Cutoff FG AR", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_d1r: IntParam::new("Cutoff FG D1R", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_d1l: IntParam::new("Cutoff FG D1L", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_d2r: IntParam::new("Cutoff FG D2R", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_rr: IntParam::new("Cutoff FG RR", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_depth: IntParam::new("Cutoff FG Depth", 128, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_floor: IntParam::new("Cutoff FG Floor", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_delay: IntParam::new("Cutoff FG Delay", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff_fg_loop: BoolParam::new("Cutoff FG Loop", false),
            cutoff_fg_curve: BoolParam::new("Cutoff FG Curve", false),

            // Gain FG既定＝ym38x6-core::default_gain_fg()と一致（rr=0＝透過的既定、
            // オペレーター本来のリリース尾を打ち消さない。旧VST既定rr=255は不整合だった）。
            gain_fg_ar: IntParam::new("Gain FG AR", 255, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_d1r: IntParam::new("Gain FG D1R", 0, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_d1l: IntParam::new("Gain FG D1L", 255, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_d2r: IntParam::new("Gain FG D2R", 0, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_rr: IntParam::new("Gain FG RR", 0, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_floor: IntParam::new("Gain FG Floor", 0, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_delay: IntParam::new("Gain FG Delay", 0, IntRange::Linear { min: 0, max: 255 }),
            gain_fg_loop: BoolParam::new("Gain FG Loop", false),
            gain_fg_curve: BoolParam::new("Gain FG Curve", false),

            rev_send: IntParam::new("Reverb Send", 0, IntRange::Linear { min: 0, max: 255 }),
            cho_send: IntParam::new("Chorus Send", 0, IntRange::Linear { min: 0, max: 255 }),
            operators: Default::default(),
            reverb_type: IntParam::new("Reverb Type", DEFAULT_REVERB_TYPE as i32, IntRange::Linear { min: 0, max: 7 }),
            reverb_time: IntParam::new("Reverb Time", DEFAULT_REVERB_TIME as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_type: IntParam::new("Chorus Type", DEFAULT_CHORUS_TYPE as i32, IntRange::Linear { min: 0, max: 7 }),
            chorus_mod_rate: IntParam::new("Chorus Mod Rate", DEFAULT_CHORUS_MOD_RATE as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_mod_depth: IntParam::new("Chorus Mod Depth", DEFAULT_CHORUS_MOD_DEPTH as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_feedback: IntParam::new("Chorus Feedback", DEFAULT_CHORUS_FEEDBACK as i32, IntRange::Linear { min: 0, max: 255 }),
            chorus_send_to_reverb: IntParam::new("Chorus Send To Reverb", DEFAULT_CHORUS_SEND_TO_REVERB as i32, IntRange::Linear { min: 0, max: 255 }),
        }
    }
}
