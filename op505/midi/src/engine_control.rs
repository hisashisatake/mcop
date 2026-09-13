//! エンジン全体に効くグローバル設定を扱うNRPN（NRPN(0,39)〜）の適用先。
//!
//! Reverb/Chorus系（`EffectControlTarget`）とは違い、適用先の`Op505Engine`は`op505-core`型
//! であり本クレートが既に依存を許されているため、`sound-midi`のような別クレート層は不要で
//! ここに直接実装する。

use op505_core::operator::ENV_AMP_TOLERANT_EPSILON;
use op505_core::Op505Engine;

/// NRPN(0,39)以降が指すエンジングローバル設定の適用先。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EngineControlTarget {
    /// NRPN(0,39): env_ampキャッシュの許容誤差（0〜255、`nrpn_to_env_amp_epsilon`で
    /// f32へ写像する）。
    EnvAmpEpsilon,
}

/// NRPN(0,39)の生値(0〜255)をenv_ampキャッシュの許容誤差(f32)へ写像する。
///
/// - `0` → `0.0`（Strict。`Operator::compute_env_amp`の`<=`比較が実質`==`と等価になる厳密一致）
/// - `1`〜`255` → `1e-6`（Tolerant。8e9c3f9で最初に検証・出荷した値）固定
///
/// 当初は`1e-6`〜`1e-3`を対数的に補間する連続値だったが、実測で**1e-5を超えたあたりから
/// EGステージ境界（Attack→Decay等）をまたぐサンプルでキャッシュが誤って外挿し、
/// env_ampが100%を超えてオーバーシュートする**（境界直前の比率をそのまま使ってしまうため）
/// ことが判明し、2値化した。1e-6より緩い値に安全域が存在しないため、Tolerantは
/// この1点のみをサポートする（詳細はop505-coreの`ENV_AMP_TOLERANT_EPSILON`のdoc参照）。
pub fn nrpn_to_env_amp_epsilon(value: u8) -> f32 {
    if value == 0 {
        0.0
    } else {
        ENV_AMP_TOLERANT_EPSILON
    }
}

/// `target`が指す`Op505Engine`のグローバル設定へ`value`（NRPN生値、0〜255）を書き込む。
pub fn apply_engine_control(engine: &mut Op505Engine, target: EngineControlTarget, value: u8) {
    match target {
        EngineControlTarget::EnvAmpEpsilon => {
            engine.set_env_amp_epsilon(nrpn_to_env_amp_epsilon(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nrpn_zero_is_exact_strict_mode() {
        assert_eq!(nrpn_to_env_amp_epsilon(0), 0.0);
    }

    #[test]
    fn nrpn_one_is_tolerant() {
        // NRPN値1は8e9c3f9で最初に検証・出荷したTolerant値(1e-6)そのもの。
        let epsilon = nrpn_to_env_amp_epsilon(1);
        assert!((epsilon - 1e-6).abs() < 1e-9, "epsilon={epsilon}");
    }

    #[test]
    fn nrpn_255_is_also_tolerant_not_looser() {
        // 1〜255は全てTolerant(1e-6)固定。かつて1e-3まで緩めていたが、境界サンプルの
        // オーバーシュートが実測で確認されたため2値化した（doc参照）。
        let epsilon = nrpn_to_env_amp_epsilon(255);
        assert!((epsilon - 1e-6).abs() < 1e-9, "epsilon={epsilon}");
    }

    #[test]
    fn mapping_is_two_valued() {
        assert_eq!(nrpn_to_env_amp_epsilon(0), 0.0);
        for v in 1..=255u8 {
            assert!((nrpn_to_env_amp_epsilon(v) - 1e-6).abs() < 1e-9, "value {v} should map to 1e-6");
        }
    }

    #[test]
    fn apply_engine_control_does_not_panic() {
        // env_amp_epsilonの実際の反映確認（発音中ボイスへの即時伝播含む）はop505-core側の
        // 責務（`Op505Engine::set_env_amp_epsilon`）。ここでは配線（NRPN値→Engineへの
        // 到達経路）自体がパニックしないことのみ確認する。
        let mut engine = Op505Engine::new(44100.0);
        for value in [0u8, 1, 128, 255] {
            apply_engine_control(&mut engine, EngineControlTarget::EnvAmpEpsilon, value);
        }
    }
}
