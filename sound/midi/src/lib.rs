//! op505に依存しない中立なMIDI解釈クレート。
//!
//! GM2 Universal SysExパーサ（`universal_sysex`）と、エフェクト系NRPN/CCの
//! `MasterEffects`への適用（`effect_control`）の2つを持つ。どちらもop505固有の
//! 要素（NRPNアドレス表・チャンネル状態機械等）を含まないため、`op505-midi`より
//! 下の層に置く（`op505-midi`はNRPNアドレス解決を担い、実際のMasterEffects書き込みは
//! 本クレートの`apply_effect_control`へ委譲する）。

pub mod effect_control;
pub mod universal_sysex;

pub use effect_control::{apply_effect_control, EffectControlTarget};
pub use universal_sysex::{parse_universal_sysex, value14_to_u8, UniversalSysEx};
