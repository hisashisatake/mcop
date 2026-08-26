//! op505のCC/NRPN解釈を共有するクレート。`op505-vst`と`op505/tools/smf2op505`の両方が参照する
//! （fork-on-writeの限定的な例外。VSTと参照実装が食い違うと「どちらが正しいか」を決める基準
//! そのものが消えるため、詳細はspec-fm.md 8章参照）。

pub mod channel_state;
pub mod control;
pub mod expression;
pub mod pedal;
pub mod pitch_fg;
pub mod rhythm;
pub mod rpn;
pub mod value;

pub use channel_state::{ChannelState, DataEntryOutcome, EffectControlTarget};
pub use control::{control_target, needs_voice_update, ControlTarget};
pub use expression::{apply_expression_modulation, apply_soft_pedal, ExpressionDestination};
pub use pedal::{released_notes, PedalState};
pub use pitch_fg::apply_pitch_fg_expression;
pub use rhythm::{
    rhythm_bank, ChannelProgramState, ProgramSelection, MELODIC_BANK_MSB, RHYTHM_BANK_BASE, RHYTHM_BANK_MSB,
    RHYTHM_BANK_RANGE, RHYTHM_DEFAULT_CHANNEL,
};
pub use rpn::{RpnSelection, RpnTracker};
pub use value::{cc_byte_to_u7, cc_byte_to_u8, cc_to_u7, cc_to_u8};
