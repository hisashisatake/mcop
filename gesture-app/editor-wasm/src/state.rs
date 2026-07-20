/// gesture-app側エディタが保持するパッチ状態。値はVST(`ym38x6-vst/src/params.rs`)の
/// `Ym38x6Params::default()`/`OperatorVstParams::default()`と同じデフォルト値を使う
/// （音が出る初期状態を保つため）。Tauri IPC経由で`ym38x6_set_patch`/`set_master_effects`
/// へ渡せる形をそのまま保持する。
#[derive(Clone, Copy)]
pub struct OperatorState {
    pub tl: i32,
    pub ar: i32,
    pub d1r: i32,
    pub d2r: i32,
    pub d1l: i32,
    pub rr: i32,
    pub mul: i32,
    pub dt1: i32,
    pub ksr: i32,
    pub ame: bool,
    pub vel_sens: i32,
    pub op_fine_tune: i32,
    pub waveform: i32,
    pub floor: i32,
    pub op_loop: bool,
    pub curve: bool,
    pub eg_shift: i32,
    pub level_scale: i32,
}

impl Default for OperatorState {
    fn default() -> Self {
        Self {
            tl: 200,
            ar: 255,
            d1r: 100,
            d2r: 80,
            d1l: 180,
            rr: 150,
            mul: 1,
            dt1: 128,
            ksr: 64,
            ame: false,
            vel_sens: 0,
            op_fine_tune: 128,
            waveform: 0,
            floor: 0,
            op_loop: false,
            curve: false,
            eg_shift: 0,
            level_scale: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct EditorState {
    // CHANNEL
    pub algorithm: i32,
    pub feedback: i32,
    // TEXTURE LFO（旧PERF LFO、5波形専用・焼き込み専用）。全項目`ChannelParams.texture_lfo`の
    // 一部としてパッチに保存され、ym38x6_set_patch経由でエンジンへ送られる
    // （宣言順インデックス。waveform=0〜4、fade_mode=0〜3。offsetは中心128＝オフセットなし）。
    pub texture_lfo_rate: i32,
    pub texture_lfo_depth: i32,
    pub texture_lfo_delay: i32,
    pub texture_lfo_waveform: i32,
    /// main.js（V キー）のランタイム制御が書き込む宛先(0=Pitch/1=Volume/2=TL/3=Cutoff)。
    /// ym38x6-ui::PanelParamsにノブは無い（VSTのDAWパラメーターにも存在しないNRPN専用項目、
    /// spec-sound.md「質感LFO」節）が、エディタ経由のsend_patchが上書きしないようDTOでは
    /// 往復保持する。
    pub texture_lfo_destination: i32,
    pub texture_lfo_fade_mode: i32,
    pub texture_lfo_fade_time: i32,
    pub texture_lfo_offset: i32,
    // CHIP LFO（旧TONE LFO）
    pub chip_lfo_freq: i32,
    pub chip_lfo_pmd: i32,
    pub chip_lfo_amd: i32,
    pub chip_lfo_delay: i32,
    pub pms: i32,
    pub ams: i32,
    // PITCH FG（新規。②③層のCC1/76/77/78補正はmain.js/main.rs側で焼き込む対象で、
    // ここは①パッチが持つ基準値）。
    pub pitch_fg_ar: i32,
    pub pitch_fg_d1r: i32,
    pub pitch_fg_d1l: i32,
    pub pitch_fg_d2r: i32,
    pub pitch_fg_rr: i32,
    /// バイポーラDepth（0〜255、中心128＝変調なし）。
    pub pitch_fg_depth: i32,
    pub pitch_fg_floor: i32,
    pub pitch_fg_delay: i32,
    pub pitch_fg_loop: bool,
    pub pitch_fg_curve: bool,
    // FILTER（CUTOFF FG、旧Filter EG）
    pub cutoff: i32,
    pub resonance: i32,
    pub filter_type: i32,
    pub filter_self_oscillation: bool,
    pub cutoff_fg_ar: i32,
    pub cutoff_fg_d1r: i32,
    pub cutoff_fg_d1l: i32,
    pub cutoff_fg_d2r: i32,
    pub cutoff_fg_rr: i32,
    /// バイポーラDepth（0〜255、中心128＝変調なし）。
    pub cutoff_fg_depth: i32,
    pub cutoff_fg_floor: i32,
    pub cutoff_fg_delay: i32,
    pub cutoff_fg_loop: bool,
    pub cutoff_fg_curve: bool,
    // GAIN FG（旧VCA EG。音量に負値は無いためDepthを持たずFloorが深さ役）
    pub gain_fg_ar: i32,
    pub gain_fg_d1r: i32,
    pub gain_fg_d1l: i32,
    pub gain_fg_d2r: i32,
    pub gain_fg_rr: i32,
    pub gain_fg_floor: i32,
    pub gain_fg_delay: i32,
    pub gain_fg_loop: bool,
    pub gain_fg_curve: bool,
    // MASTER EFFECT
    pub rev_send: i32,
    pub reverb_type: i32,
    pub reverb_time: i32,
    pub cho_send: i32,
    pub chorus_type: i32,
    pub chorus_mod_rate: i32,
    pub chorus_mod_depth: i32,
    pub chorus_feedback: i32,
    pub chorus_send_to_reverb: i32,
    // OPERATORS
    pub operators: [OperatorState; 4],
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            algorithm: 0,
            feedback: 0,
            texture_lfo_rate: 0,
            texture_lfo_depth: 0,
            texture_lfo_delay: 0,
            texture_lfo_waveform: 0,
            texture_lfo_destination: 0,
            texture_lfo_fade_mode: 0,
            texture_lfo_fade_time: 0,
            texture_lfo_offset: 128,
            chip_lfo_freq: 0,
            chip_lfo_pmd: 0,
            chip_lfo_amd: 0,
            chip_lfo_delay: 0,
            pms: 0,
            ams: 0,
            // Pitch FG既定＝ym38x6-core::default_pitch_fg()と一致（「無効」状態）。
            pitch_fg_ar: 0,
            pitch_fg_d1r: 0,
            pitch_fg_d1l: 255,
            pitch_fg_d2r: 0,
            pitch_fg_rr: 255,
            pitch_fg_depth: 128,
            pitch_fg_floor: 0,
            pitch_fg_delay: 0,
            pitch_fg_loop: false,
            pitch_fg_curve: false,
            cutoff: 255,
            resonance: 0,
            filter_type: 0,
            filter_self_oscillation: true,
            cutoff_fg_ar: 0,
            cutoff_fg_d1r: 0,
            cutoff_fg_d1l: 0,
            cutoff_fg_d2r: 0,
            cutoff_fg_rr: 0,
            cutoff_fg_depth: 128,
            cutoff_fg_floor: 0,
            cutoff_fg_delay: 0,
            cutoff_fg_loop: false,
            cutoff_fg_curve: false,
            // Gain FG既定＝ym38x6-core::default_gain_fg()と一致（rr=0＝透過的既定、
            // オペレーター本来のリリース尾を打ち消さない）。
            gain_fg_ar: 255,
            gain_fg_d1r: 0,
            gain_fg_d1l: 255,
            gain_fg_d2r: 0,
            gain_fg_rr: 0,
            gain_fg_floor: 0,
            gain_fg_delay: 0,
            gain_fg_loop: false,
            gain_fg_curve: false,
            rev_send: 0,
            reverb_type: 3,
            reverb_time: 128,
            cho_send: 0,
            chorus_type: 0,
            chorus_mod_rate: 128,
            chorus_mod_depth: 128,
            chorus_feedback: 0,
            chorus_send_to_reverb: 0,
            operators: [OperatorState::default(); 4],
        }
    }
}
