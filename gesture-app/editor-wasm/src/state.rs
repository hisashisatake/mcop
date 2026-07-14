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
        }
    }
}

#[derive(Clone, Copy)]
pub struct EditorState {
    // CHANNEL
    pub algorithm: i32,
    pub feedback: i32,
    // PERF LFO Rate/Depth/Delayはgesture-app側ではローカル編集のみ（main.jsのホイール/Vキー制御とは
    // 未連動、ym38x6_set_performance_lfoへのブロードキャストは行わない。將来の連動はTODO）。
    pub lfo_rate: i32,
    pub lfo_depth: i32,
    pub lfo_delay: i32,
    // Waveform/Fade Mode/Fade Time/Offsetは`ChannelParams.perf_lfo_shape`の一部としてパッチに
    // 保存され、Rate/Depth/Delayとは異なりym38x6_set_patch経由でエンジンへ送られる
    // （宣言順インデックス。waveform=0〜7、fade_mode=0〜3。offsetは中心128＝オフセットなし）。
    pub lfo_waveform: i32,
    pub lfo_fade_mode: i32,
    pub lfo_fade_time: i32,
    pub lfo_offset: i32,
    // TONE LFO
    pub tone_freq: i32,
    pub tone_pmd: i32,
    pub tone_amd: i32,
    pub tone_delay: i32,
    pub pms: i32,
    pub ams: i32,
    // FILTER
    pub cutoff: i32,
    pub resonance: i32,
    pub filter_type: i32,
    pub filter_self_oscillation: bool,
    pub feg_ar: i32,
    pub feg_d1r: i32,
    pub feg_d1l: i32,
    pub feg_d2r: i32,
    pub feg_rr: i32,
    pub feg_depth: i32,
    // VCA (TVAオーバーレイ)
    pub vca_ar: i32,
    pub vca_d1r: i32,
    pub vca_d1l: i32,
    pub vca_d2r: i32,
    pub vca_rr: i32,
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
            lfo_rate: 0,
            lfo_depth: 0,
            lfo_delay: 0,
            lfo_waveform: 0,
            lfo_fade_mode: 0,
            lfo_fade_time: 0,
            lfo_offset: 128,
            tone_freq: 0,
            tone_pmd: 0,
            tone_amd: 0,
            tone_delay: 0,
            pms: 0,
            ams: 0,
            cutoff: 255,
            resonance: 0,
            filter_type: 0,
            filter_self_oscillation: true,
            feg_ar: 0,
            feg_d1r: 0,
            feg_d1l: 0,
            feg_d2r: 0,
            feg_rr: 0,
            feg_depth: 0,
            vca_ar: 255,
            vca_d1r: 0,
            vca_d1l: 255,
            vca_d2r: 0,
            vca_rr: 255,
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
