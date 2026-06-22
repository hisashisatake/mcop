"""フェーズ6 MVP: 自己教師ありデータセット生成CLI。

ランダムな制限サブセットparams → 38x6で音(ym38x6_ml) → 無音はリジェクト → 対数メル特徴 → npz保存。
学習データ「(音響特徴, 正解params)」を作る。エンジン自身が正解ラベラー。

実行例（.venvのpythonで）:
    python generate.py --n 16
出力既定: ../private/dataset_mvp.npz
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import numpy as np

# 同ディレクトリのモジュールをcwd非依存でimportできるようにする。
sys.path.insert(0, str(Path(__file__).resolve().parent))

import features as ft  # noqa: E402
import param_space as ps  # noqa: E402
import ym38x6_ml  # noqa: E402  (maturin developで.venvにインストール済み)

_SEMITONES = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}


def note_freq(name: str = "C", octave: int = 4) -> float:
    midi = (octave + 1) * 12 + _SEMITONES[name]
    return 440.0 * 2 ** ((midi - 69) / 12)


def main() -> int:
    ap = argparse.ArgumentParser(description="38x6 自己教師ありデータセット生成 (MVP)")
    ap.add_argument("--n", type=int, default=16, help="採用サンプル数")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--out", type=str, default=None, help="出力npz（既定 ../private/dataset_mvp.npz）")
    ap.add_argument("--freq", type=float, default=note_freq("C", 4))
    ap.add_argument("--on", type=float, default=0.6, help="キーオン保持秒")
    ap.add_argument("--release", type=float, default=0.3, help="キーオフ後余韻秒")
    ap.add_argument("--sr", type=float, default=44100.0)
    ap.add_argument("--n-mels", type=int, default=64)
    ap.add_argument("--n-frames", type=int, default=16)
    ap.add_argument("--max-tries-factor", type=int, default=8, help="無音リジェクト込みの試行上限=n×この値")
    args = ap.parse_args()

    out = Path(args.out) if args.out else Path(__file__).resolve().parent.parent / "private" / "dataset_mvp.npz"
    out.parent.mkdir(parents=True, exist_ok=True)

    rng = np.random.default_rng(args.seed)
    params_list: list[np.ndarray] = []
    feats_list: list[np.ndarray] = []
    rejected = 0
    tries = 0
    max_total = args.n * args.max_tries_factor
    t0 = time.time()

    while len(params_list) < args.n and tries < max_total:
        tries += 1
        vec = ps.random_vectors(1, rng)[0]
        patch_json = ps.vector_to_patch_json(vec)
        samples = ym38x6_ml.render_patch(patch_json, args.freq, args.on, args.release, 115, args.sr)
        if ft.is_silent(samples):
            rejected += 1
            continue
        feat = ft.log_mel(samples, sr=args.sr, n_mels=args.n_mels, n_frames=args.n_frames)
        params_list.append(vec.astype(np.float32))
        feats_list.append(feat)

    if not params_list:
        print("ERROR: 有効サンプルが0件（全て無音？閾値や固定値を見直してください）", file=sys.stderr)
        return 1

    params = np.stack(params_list)  # [n, DIM]
    feats = np.stack(feats_list)    # [n, n_mels, n_frames]

    meta = dict(
        spec_version=ps.SPEC_VERSION,
        dim=ps.DIM,
        labels=ps.LABELS,
        fixed_algorithm=ps.FIXED_ALGORITHM,
        freq=args.freq,
        sr=args.sr,
        on=args.on,
        release=args.release,
        n_mels=args.n_mels,
        n_frames=args.n_frames,
    )
    np.savez_compressed(out, params=params, features=feats, meta=np.array(json.dumps(meta)))

    dt = time.time() - t0
    print(f"保存: {out}")
    print(f"採用={len(params_list)} 無音リジェクト={rejected} 試行={tries} 所要={dt:.1f}s")
    print(f"params.shape={params.shape} features.shape={feats.shape}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
