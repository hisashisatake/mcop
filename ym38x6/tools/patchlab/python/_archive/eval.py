"""フェーズ6 マイルストーン3a: 逆算モデルを「音指標」で評価する。

paramsMSE だけでは FM の非線形性で良し悪しが測れないため、ここが本当の合否判定:
  ランダム真値params → render → 目標音/目標mel
  目標mel → モデル → 予測params → render → 予測mel
  距離(目標mel, 予測mel) を集計し、無情報ベースライン(平均params固定)と比べる。

数件は目標音/予測音のWAVを書き出して聴き比べできるようにする（predict→render→聴くループ）。

実行例（.venvのpythonで）:
    python eval.py --model ../private/model_mvp.pt --n 200 --wav 6
"""

from __future__ import annotations

import argparse
import sys
import wave
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import features as ft  # noqa: E402
import param_space as ps  # noqa: E402
import patchlab  # noqa: E402
from model import InverseNet  # noqa: E402


def write_wav(path: Path, samples, sr: float) -> None:
    """モノラルf32(おおむね[-1,1]) → 16bit PCM WAV。"""
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


def main() -> int:
    ap = argparse.ArgumentParser(description="38x6 逆算モデル 音指標評価 (MVP 3a)")
    ap.add_argument("--model", type=str, default=None, help="チェックポイント（既定 ../private/model_mvp.pt）")
    ap.add_argument("--n", type=int, default=200, help="評価サンプル数（無音はスキップ）")
    ap.add_argument("--wav", type=int, default=6, help="WAV書き出しするペア数")
    ap.add_argument("--seed", type=int, default=12345, help="学習seedと変える（未知ターゲット）")
    ap.add_argument("--wav-dir", type=str, default=None, help="WAV出力先（既定 ../private/eval_wav）")
    args = ap.parse_args()

    base = Path(__file__).resolve().parent.parent / "private"
    model_path = Path(args.model) if args.model else base / "model_mvp.pt"
    wav_dir = Path(args.wav_dir) if args.wav_dir else base / "eval_wav"

    ckpt = torch.load(model_path, map_location="cpu", weights_only=False)
    meta = ckpt["meta"]
    mean, std = ckpt["mean"], ckpt["std"]
    n_mels, n_frames = ckpt["n_mels"], ckpt["n_frames"]
    freq, sr = float(meta["freq"]), float(meta["sr"])
    on, release = float(meta["on"]), float(meta["release"])

    model = InverseNet(out_dim=ckpt["out_dim"], n_mels=n_mels, n_frames=n_frames)
    model.load_state_dict(ckpt["model_state"])
    model.eval()

    labels = ckpt.get("labels") or [f"p{i}" for i in range(ckpt["out_dim"])]
    rng = np.random.default_rng(args.seed)
    mel_pred, mel_base, param_mse = [], [], []
    pred_vecs, true_vecs = [], []
    wav_written = 0
    if args.wav > 0:
        wav_dir.mkdir(parents=True, exist_ok=True)

    def render(vec):
        pj = ps.vector_to_patch_json(vec)
        return patchlab.render_patch(pj, freq, on, release, 115, sr)

    def feat(samples):
        return ft.log_mel(samples, sr=sr, n_mels=n_mels, n_frames=n_frames)

    baseline_vec = np.full(ckpt["out_dim"], 0.5, dtype=np.float32)  # 無情報: 中央値固定

    tries = 0
    while len(mel_pred) < args.n and tries < args.n * 8:
        tries += 1
        true_vec = ps.random_vectors(1, rng)[0].astype(np.float32)
        target = render(true_vec)
        if ft.is_silent(target):
            continue
        target_mel = feat(target)

        # mean/std は [n_mels,1]。target_mel[n_mels,n_frames] を標準化して [1,1,n_mels,n_frames]。
        x = ((target_mel - mean) / std)[None, None, :, :]
        with torch.no_grad():
            pred_vec = model(torch.from_numpy(x.astype(np.float32)))[0].numpy()

        pred_mel = feat(render(pred_vec))
        base_mel = feat(render(baseline_vec))

        mel_pred.append(float(np.abs(pred_mel - target_mel).mean()))
        mel_base.append(float(np.abs(base_mel - target_mel).mean()))
        param_mse.append(float(np.mean((pred_vec - true_vec) ** 2)))
        pred_vecs.append(pred_vec)
        true_vecs.append(true_vec)

        if wav_written < args.wav:
            write_wav(wav_dir / f"{wav_written:02d}_target.wav", target, sr)
            write_wav(wav_dir / f"{wav_written:02d}_pred.wav", render(pred_vec), sr)
            wav_written += 1

    if not mel_pred:
        print("ERROR: 有効サンプルが0件", file=sys.stderr)
        return 1

    mp = float(np.mean(mel_pred))
    mb = float(np.mean(mel_base))
    print(f"評価={len(mel_pred)}件 (有音のみ, 試行={tries})")
    print(f"  mel距離(L1)  予測={mp:.4f}  ベースライン(0.5固定)={mb:.4f}  改善率={(1 - mp / mb) * 100:.1f}%")
    print(f"  paramMSE     予測={float(np.mean(param_mse)):.4f}")
    if wav_written:
        print(f"  WAV書き出し: {wav_dir}（{wav_written}ペア, *_target / *_pred）")

    # param別診断: 各次元の |pred-true| 平均(MAE)と予測の標準偏差(pstd)。
    # pstd が小さい=入力に依らずほぼ定数を出している(学べていない)。
    P = np.stack(pred_vecs)  # [N, DIM]
    T = np.stack(true_vecs)
    mae = np.abs(P - T).mean(axis=0)
    pstd = P.std(axis=0)
    # MAEの悪い順に表示（true は uniform[0,1] なので無情報MAE≒0.33 が上限目安）。
    order = np.argsort(-mae)
    print("  param別診断 (MAE降順, pstd=予測の標準偏差; 無情報MAE≒0.33):")
    for i in order:
        print(f"    {labels[i]:>16}  MAE={mae[i]:.3f}  pstd={pstd[i]:.3f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
