/// MASTER EFFECTS（Reverb/Chorus）パネル用の状態。エンジン非依存のため`op505_state::Op505State`と
/// 分離して保持し、`op505_state::Op505State::build_panel_params`から共有参照される
/// （`sound_core::MasterEffects`はエンジン側の状態で、ここはIPC送信用のフロント側ミラー）。
#[derive(Clone, Copy)]
pub struct MasterEffectsState {
    pub rev_send: i32,
    pub reverb_type: i32,
    pub reverb_time: i32,
    pub cho_send: i32,
    pub chorus_type: i32,
    pub chorus_mod_rate: i32,
    pub chorus_mod_depth: i32,
    pub chorus_feedback: i32,
    pub chorus_send_to_reverb: i32,
}

impl Default for MasterEffectsState {
    fn default() -> Self {
        Self {
            rev_send: 0,
            reverb_type: 3,
            reverb_time: 128,
            cho_send: 0,
            chorus_type: 0,
            chorus_mod_rate: 128,
            chorus_mod_depth: 128,
            chorus_feedback: 0,
            chorus_send_to_reverb: 0,
        }
    }
}
