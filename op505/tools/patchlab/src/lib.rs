//! patchlab — op505 FMエンジンのPythonバインディング。
//!
//! forward 方向（パラメーター→音）を Python から呼べるようにする。音色設計テンプレート・
//! 試聴ツール（piano/brass/organ_template, audition, phrase等）と、ML逆算合成系ツール
//! （abys, probe, lookup等）の両方がこのバインディング経由でエンジンを呼ぶ。
//!
//! 由来: ym38x6/tools/patchlab/src/lib.rs（コミット 5cccac8 時点の複製、2026-08-13）。
//! デフォーク後のop505ツール群向け複製（fork-on-write）。

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use op505_core::{eg_convert::convert_eg_shape, Op505Engine, Op505Patch};
use sound_core::Vco;

/// JSONで与えた1パッチ（`Op505Patch` のserde表現）を1ノート分レンダリングし、
/// モノラルf32サンプル列を返す。
///
/// - `patch_json`: `Op505Patch` をserializeしたJSON文字列。
/// - `freq`: 発音周波数（Hz）。
/// - `on_secs` / `release_secs`: キーオン保持時間 / キーオフ後の余韻（秒）。
/// - `velocity`: ノートオンベロシティ（0〜127）。
/// - `sample_rate`: サンプルレート（Hz）。
#[pyfunction]
#[pyo3(signature = (patch_json, freq, on_secs=1.0, release_secs=0.5, velocity=115, sample_rate=44100.0))]
fn render_patch(
    patch_json: &str,
    freq: f32,
    on_secs: f32,
    release_secs: f32,
    velocity: u8,
    sample_rate: f32,
) -> PyResult<Vec<f32>> {
    let patch: Op505Patch = serde_json::from_str(patch_json)
        .map_err(|e| PyValueError::new_err(format!("patch JSON parse error: {e}")))?;

    let mut engine = Op505Engine::new(sample_rate);
    engine.set_patch(patch);
    engine.note_on(0, freq, velocity);

    let on = (sample_rate * on_secs).max(0.0) as usize;
    let off = (sample_rate * release_secs).max(0.0) as usize;
    let mut samples = vec![0.0f32; on];
    engine.render(&mut samples, 1);
    engine.note_off(0);
    let mut tail = vec![0.0f32; off];
    engine.render(&mut tail, 1);
    samples.extend_from_slice(&tail);
    Ok(samples)
}

/// レート方式5段EG(ar/d1r/d1l/d2r/rr)の数値を`op505-core::eg_convert::convert_eg_shape`で
/// `TimeEgParams`へ変換し、そのJSON表現と警告リストを返す。EG変換の実装をPython側に
/// 複製させないための唯一の入口（Python側からTimeEg変換ロジックを再実装しない）。
#[pyfunction]
#[pyo3(signature = (ar, d1r, d1l, d2r, rr, floor=0, loop_enabled=0, curve=0, label="patchlab"))]
fn rate_eg_to_time_eg(
    ar: u8,
    d1r: u8,
    d1l: u8,
    d2r: u8,
    rr: u8,
    floor: u8,
    loop_enabled: u8,
    curve: u8,
    label: &str,
) -> PyResult<(String, Vec<String>)> {
    let mut warnings = Vec::new();
    let eg = convert_eg_shape(ar, d1r, d1l, d2r, rr, floor, loop_enabled, curve, &mut warnings, label);
    let json = serde_json::to_string(&eg).map_err(|e| PyValueError::new_err(format!("eg JSON serialize error: {e}")))?;
    Ok((json, warnings))
}

/// `Op505Patch::default()` のJSON表現。Python側がop505パッチの既定フィールド値を
/// ハードコードせずに済むようにする（`Op505ChannelParams`等にフィールドが増えても
/// Python側の`_fixed_operator`/`_fixed_channel`相当が壊れない）。
#[pyfunction]
fn default_patch_json() -> PyResult<String> {
    serde_json::to_string(&Op505Patch::default())
        .map_err(|e| PyValueError::new_err(format!("default patch JSON serialize error: {e}")))
}

/// バインディングが生きているか確認する簡易関数（スモークテスト用）。
#[pyfunction]
fn engine_info() -> String {
    "patchlab: op505 FM engine binding (forward render)".to_string()
}

#[pymodule]
fn patchlab(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render_patch, m)?)?;
    m.add_function(wrap_pyfunction!(rate_eg_to_time_eg, m)?)?;
    m.add_function(wrap_pyfunction!(default_patch_json, m)?)?;
    m.add_function(wrap_pyfunction!(engine_info, m)?)?;
    Ok(())
}
