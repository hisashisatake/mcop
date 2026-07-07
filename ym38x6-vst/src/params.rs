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
        }
    }
}

#[derive(Params)]
pub(crate) struct Ym38x6Params {
    // ---- チャンネル単位（20個、spec.md MIDI実装方針参照） ----
    #[id = "algorithm"]
    pub algorithm: IntParam,
    #[id = "feedback"]
    pub feedback: IntParam,
    #[id = "lfo_rate"]
    pub lfo_rate: IntParam,
    #[id = "lfo_depth"]
    pub lfo_depth: IntParam,
    #[id = "lfo_delay"]
    pub lfo_delay: IntParam,
    #[id = "tone_freq"]
    pub tone_freq: IntParam,
    #[id = "tone_pmd"]
    pub tone_pmd: IntParam,
    #[id = "tone_amd"]
    pub tone_amd: IntParam,
    #[id = "tone_delay"]
    pub tone_delay: IntParam,
    #[id = "pms"]
    pub pms: IntParam,
    #[id = "ams"]
    pub ams: IntParam,
    #[id = "cutoff"]
    pub cutoff: IntParam,
    #[id = "resonance"]
    pub resonance: IntParam,
    #[id = "feg_ar"]
    pub feg_ar: IntParam,
    #[id = "feg_d1r"]
    pub feg_d1r: IntParam,
    #[id = "feg_d1l"]
    pub feg_d1l: IntParam,
    #[id = "feg_d2r"]
    pub feg_d2r: IntParam,
    #[id = "feg_rr"]
    pub feg_rr: IntParam,
    #[id = "feg_depth"]
    pub feg_depth: IntParam,
    #[id = "vca_ar"]
    pub vca_ar: IntParam,
    #[id = "vca_d1r"]
    pub vca_d1r: IntParam,
    #[id = "vca_d1l"]
    pub vca_d1l: IntParam,
    #[id = "vca_d2r"]
    pub vca_d2r: IntParam,
    #[id = "vca_rr"]
    pub vca_rr: IntParam,
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
            lfo_rate: IntParam::new("Perf LFO Rate", 0, IntRange::Linear { min: 0, max: 255 }),
            lfo_depth: IntParam::new("Perf LFO Depth", 0, IntRange::Linear { min: 0, max: 255 }),
            lfo_delay: IntParam::new("Perf LFO Delay", 0, IntRange::Linear { min: 0, max: 255 }),
            tone_freq: IntParam::new("Tone LFO Freq", 0, IntRange::Linear { min: 0, max: 255 }),
            tone_pmd: IntParam::new("Tone LFO PMD", 0, IntRange::Linear { min: 0, max: 255 }),
            tone_amd: IntParam::new("Tone LFO AMD", 0, IntRange::Linear { min: 0, max: 255 }),
            tone_delay: IntParam::new("Tone LFO Delay", 0, IntRange::Linear { min: 0, max: 255 }),
            pms: IntParam::new("PMS", 0, IntRange::Linear { min: 0, max: 255 }),
            ams: IntParam::new("AMS", 0, IntRange::Linear { min: 0, max: 255 }),
            cutoff: IntParam::new("Filter Cutoff", 255, IntRange::Linear { min: 0, max: 255 }),
            resonance: IntParam::new("Filter Resonance", 0, IntRange::Linear { min: 0, max: 255 }),
            feg_ar: IntParam::new("Filter EG AR", 0, IntRange::Linear { min: 0, max: 255 }),
            feg_d1r: IntParam::new("Filter EG D1R", 0, IntRange::Linear { min: 0, max: 255 }),
            feg_d1l: IntParam::new("Filter EG D1L", 0, IntRange::Linear { min: 0, max: 255 }),
            feg_d2r: IntParam::new("Filter EG D2R", 0, IntRange::Linear { min: 0, max: 255 }),
            feg_rr: IntParam::new("Filter EG RR", 0, IntRange::Linear { min: 0, max: 255 }),
            feg_depth: IntParam::new("Filter EG Depth", 0, IntRange::Linear { min: 0, max: 255 }),
            vca_ar: IntParam::new("VCA EG AR", 255, IntRange::Linear { min: 0, max: 255 }),
            vca_d1r: IntParam::new("VCA EG D1R", 0, IntRange::Linear { min: 0, max: 255 }),
            vca_d1l: IntParam::new("VCA EG D1L", 255, IntRange::Linear { min: 0, max: 255 }),
            vca_d2r: IntParam::new("VCA EG D2R", 0, IntRange::Linear { min: 0, max: 255 }),
            vca_rr: IntParam::new("VCA EG RR", 255, IntRange::Linear { min: 0, max: 255 }),
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
