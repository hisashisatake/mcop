//! ym38x6-ml — 38x6 FMエンジンのPythonバインディング（フェーズ6: ML逆算合成）。
//!
//! まずは forward 方向（パラメーター→音）を Python から呼べるようにする。Python側で
//! ランダムサンプリングして (パラメーター, 音響特徴) のデータセットを生成し、逆算モデル
//! （目標音 → FMパラメーター）の学習に使う想定。逆算モデルの方式はデータ生成が回ってから決める。

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use ym38x6_core::{SoundEngine, Ym38x6Engine, Ym38x6Patch};

/// JSONで与えた1パッチ（`Ym38x6Patch` のserde表現）を1ノート分レンダリングし、
/// モノラルf32サンプル列を返す。
///
/// - `patch_json`: `Ym38x6Patch` をserializeしたJSON文字列。
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
    let patch: Ym38x6Patch = serde_json::from_str(patch_json)
        .map_err(|e| PyValueError::new_err(format!("patch JSON parse error: {e}")))?;

    let mut engine = Ym38x6Engine::new(sample_rate);
    engine.note_on_with_velocity(0, freq, velocity, patch);

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

/// バインディングが生きているか確認する簡易関数（スモークテスト用）。
#[pyfunction]
fn engine_info() -> String {
    "ym38x6-ml: 38x6 FM engine binding (phase 6, forward render)".to_string()
}

#[pymodule]
fn ym38x6_ml(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render_patch, m)?)?;
    m.add_function(wrap_pyfunction!(engine_info, m)?)?;
    Ok(())
}
