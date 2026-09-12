//! エンジン全体に効くグローバル設定を扱うNRPN（NRPN(0,39)〜）の適用先。
//!
//! Reverb/Chorus系（`EffectControlTarget`）とは違い、適用先の`Op505Engine`は`op505-core`型
//! であり本クレートが既に依存を許されているため、`sound-midi`のような別クレート層は不要で
//! ここに直接実装する。

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
/// - `0` → `0.0`（`Operator::compute_env_amp`の`<=`比較が実質`==`と等価になる、
///   8e9c3f9以前の厳密一致相当）
/// - `1`〜`255` → `1e-6`（8e9c3f9で導入した現行既定値）から`1e-3`（かなり緩い許容度）まで
///   対数的に補間する。NRPN値1がほぼ現行既定値に一致するため、「NRPNを送っていない状態」
///   （エンジンの初期値、こちらも1e-6）との連続性が保たれる。
pub fn nrpn_to_env_amp_epsilon(value: u8) -> f32 {
    const MIN_EPSILON: f32 = 1e-6;
    const MAX_EPSILON: f32 = 1e-3;
    if value == 0 {
        return 0.0;
    }
    let t = (value - 1) as f32 / 254.0;
    MIN_EPSILON * (MAX_EPSILON / MIN_EPSILON).powf(t)
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
    fn nrpn_one_matches_current_default_closely() {
        // NRPN値1は「NRPNを送っていない状態」（エンジン初期値1e-6）と実質同じ挙動になる
        // べき——ここが飛び値だと「NRPNを触った瞬間に急に速くなる/遅くなる」という
        // 不連続な体感になってしまう。
        let epsilon = nrpn_to_env_amp_epsilon(1);
        assert!((epsilon - 1e-6).abs() < 1e-9, "epsilon={epsilon}");
    }

    #[test]
    fn nrpn_255_is_loosest() {
        let epsilon = nrpn_to_env_amp_epsilon(255);
        assert!((epsilon - 1e-3).abs() < 1e-9, "epsilon={epsilon}");
    }

    #[test]
    fn mapping_is_monotonically_non_decreasing() {
        let mut prev = nrpn_to_env_amp_epsilon(0);
        for v in 1..=255u8 {
            let cur = nrpn_to_env_amp_epsilon(v);
            assert!(cur >= prev, "not monotonic at {v}: {prev} -> {cur}");
            prev = cur;
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
