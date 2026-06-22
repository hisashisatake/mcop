"""フェーズ6 マイルストーン3b: Analysis-by-Synthesis（A-by-S）。

3a で「特徴→params のフィードフォワード回帰(params MSE)」は一対多のため音指標で
ベースライン以下と確定した。3b は実エンジンを直接ループに入れ、目標 log_mel との距離を
勾配なし最適化(CMA-ES)で最小化して params を求める（per-target・遅いが原理的に正しい）。

まずは自己再構成テスト: ランダム真値params → render → 目標音。その目標音だけを与え、
A-by-S で params を復元できるか（pred音 が目標音に十分近づくか）を測る。FM射程内・単音の
音なら復元できる、という設計仮説の検証。初期値は 0.5 固定（3aの予測は使わない）。

実行例（.venvのpythonで）:
    python abys.py --n-targets 5 --maxfevals 800 --wav 5
"""

from __future__ import annotations

import argparse
import sys
import time
import wave
from pathlib import Path

import cma
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

import features as ft  # noqa: E402
import param_space as ps  # noqa: E402
import ym38x6_ml  # noqa: E402

_SILENT_PENALTY = 10.0  # 無音解は探索から強く排除する


def note_freq(name: str = "C", octave: int = 4) -> float:
    semis = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
    midi = (octave + 1) * 12 + semis[name]
    return 440.0 * 2 ** ((midi - 69) / 12)


def write_wav(path: Path, samples, sr: float) -> None:
    x = np.asarray(samples, dtype=np.float32)
    peak = float(np.max(np.abs(x))) if x.size else 0.0
    if peak > 1.0:
        x = x / peak
    pcm = np.clip(x * 32767.0, -32768, 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(int(sr))
        w.writeframes(pcm.tobytes())


def render(vec, freq, on, release, sr):
    v = np.clip(np.asarray(vec, dtype=np.float64), 0.0, 1.0)
    return ym38x6_ml.render_patch(ps.vector_to_patch_json(v), freq, on, release, 115, sr)


def feats_of(samples, sr, n_mels, n_frames):
    return ft.abys_features(samples, sr=sr, n_mels=n_mels, n_frames=n_frames)


def fit_one(target_feats, freq, on, release, sr, n_mels, n_frames,
            w_env, w_centroid, sigma0, maxfevals, restarts, seed):
    """1ターゲットを A-by-S で復元。(xbest, best_dist, evals) を返す。

    restarts>0 で IPOP-CMA-ES（停滞時に集団サイズを倍増して再スタート）。
    「重心は合うが倍音が鈍い」局所解から抜け、金属感など高次倍音の復元を助ける。
    maxfevals は再スタート群を含む総評価予算。
    """

    def f(vec):
        s = render(vec, freq, on, release, sr)
        if ft.is_silent(s):
            return _SILENT_PENALTY
        total, _ = ft.abys_distance(
            target_feats, feats_of(s, sr, n_mels, n_frames), w_env, w_centroid)
        return total

    x0 = np.full(ps.DIM, 0.5)
    opts = {"bounds": [0.0, 1.0], "maxfevals": maxfevals, "seed": seed, "verbose": -9}
    xbest, es = cma.fmin2(f, x0, sigma0, options=opts,
                          restarts=restarts, incpopsize=2)
    xbest = np.clip(np.asarray(xbest, dtype=np.float64), 0.0, 1.0)
    return xbest, float(es.result.fbest), int(es.result.evaluations)


def main() -> int:
    ap = argparse.ArgumentParser(description="38x6 Analysis-by-Synthesis 自己再構成テスト (3b)")
    ap.add_argument("--n-targets", type=int, default=5, help="復元を試すターゲット数")
    ap.add_argument("--maxfevals", type=int, default=2400, help="1ターゲットあたりのrender評価回数上限(再スタート群を含む総予算)")
    ap.add_argument("--restarts", type=int, default=2, help="IPOP再スタート回数(停滞時に集団を倍増。0で無効)")
    ap.add_argument("--sigma0", type=float, default=0.25, help="CMA-ES初期ステップ(0..1空間)")
    ap.add_argument("--seed", type=int, default=12345, help="ターゲット生成seed(学習seedと変える)")
    ap.add_argument("--cma-seed", type=int, default=1, help="CMA-ES内部seed")
    ap.add_argument("--wav", type=int, default=5, help="WAV書き出しするペア数")
    ap.add_argument("--wav-dir", type=str, default=None, help="WAV出力先(既定 ../private/abys_wav)")
    ap.add_argument("--freq", type=float, default=note_freq("C", 4))
    ap.add_argument("--on", type=float, default=0.6)
    ap.add_argument("--release", type=float, default=0.3)
    ap.add_argument("--sr", type=float, default=44100.0)
    ap.add_argument("--n-mels", type=int, default=64)
    ap.add_argument("--n-frames", type=int, default=64)
    ap.add_argument("--w-env", type=float, default=2.0,
                    help="ラウドネス曲線項の重み(アタック/ディケイ整形)")
    ap.add_argument("--w-centroid", type=float, default=4.0,
                    help="スペクトル重心項の重み(明るさ・音程)")
    args = ap.parse_args()

    base = Path(__file__).resolve().parent.parent / "private"
    wav_dir = Path(args.wav_dir) if args.wav_dir else base / "abys_wav"
    if args.wav > 0:
        wav_dir.mkdir(parents=True, exist_ok=True)

    rng = np.random.default_rng(args.seed)
    # 0.5固定の無情報ベースライン音/特徴（比較用）。
    base_vec = np.full(ps.DIM, 0.5)
    base_feats = feats_of(render(base_vec, args.freq, args.on, args.release, args.sr),
                          args.sr, args.n_mels, args.n_frames)

    init_dists, best_dists, param_mse = [], [], []
    best_terms = []  # 各target の (d_mel, d_env, d_centroid)
    done = 0
    tries = 0
    t0 = time.time()
    while done < args.n_targets and tries < args.n_targets * 8:
        tries += 1
        true_vec = rng.random(ps.DIM)
        target = render(true_vec, args.freq, args.on, args.release, args.sr)
        if ft.is_silent(target):
            continue
        target_feats = feats_of(target, args.sr, args.n_mels, args.n_frames)

        # 初期(0.5固定)の目標距離（多項合算）。
        init_d, _ = ft.abys_distance(target_feats, base_feats, args.w_env, args.w_centroid)
        xbest, best_d, evals = fit_one(
            target_feats, args.freq, args.on, args.release, args.sr,
            args.n_mels, args.n_frames, args.w_env, args.w_centroid,
            args.sigma0, args.maxfevals, args.restarts, args.cma_seed,
        )
        recon = render(xbest, args.freq, args.on, args.release, args.sr)
        _, terms = ft.abys_distance(
            target_feats, feats_of(recon, args.sr, args.n_mels, args.n_frames),
            args.w_env, args.w_centroid)
        init_dists.append(init_d)
        best_dists.append(best_d)
        best_terms.append(terms)
        param_mse.append(float(np.mean((xbest - true_vec) ** 2)))

        if done < args.wav:
            write_wav(wav_dir / f"{done:02d}_target.wav", target, args.sr)
            write_wav(wav_dir / f"{done:02d}_recon.wav", recon, args.sr)
        d_mel, d_env, d_cen = terms
        print(f"  target {done}: init={init_d:.4f} -> best={best_d:.4f} "
              f"(改善率={(1 - best_d / init_d) * 100:.1f}%, evals={evals}) "
              f"[mel={d_mel:.4f} env={d_env:.4f} cen={d_cen:.4f}]")
        done += 1

    if not best_dists:
        print("ERROR: 有効ターゲットが0件", file=sys.stderr)
        return 1

    dt = time.time() - t0
    init_m = float(np.mean(init_dists))
    best_m = float(np.mean(best_dists))
    terms_m = np.mean(best_terms, axis=0)  # (d_mel, d_env, d_centroid)
    print(f"\nA-by-S 自己再構成 {done}件 (所要={dt:.1f}s, w_env={args.w_env} w_centroid={args.w_centroid})")
    print(f"  合算距離     初期0.5固定={init_m:.4f}  A-by-S後={best_m:.4f}  改善率={(1 - best_m / init_m) * 100:.1f}%")
    print(f"  項別(A-by-S後) mel={terms_m[0]:.4f}  env={terms_m[1]:.4f}  centroid={terms_m[2]:.4f}")
    print(f"  paramMSE     復元={float(np.mean(param_mse)):.4f}")
    if args.wav > 0:
        print(f"  WAV書き出し: {wav_dir}（{min(done, args.wav)}ペア, *_target / *_recon）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
