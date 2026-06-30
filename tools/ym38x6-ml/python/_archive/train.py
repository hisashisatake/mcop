"""フェーズ6 マイルストーン3a: 逆算回帰モデルの学習CLI（paramsMSE + Adam）。

ベースライン戦略はパラメーター損失(MSE)。FMは非線形で params 一致＝音一致とは限らないので、
ここでの val_loss はあくまで学習が回っているかの目安。最終的な合否は eval.py の音指標で判定する。

実行例（.venvのpythonで）:
    python train.py --data ../private/dataset_mvp.npz --epochs 60
出力既定: ../private/model_mvp.pt
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import DataLoader

sys.path.insert(0, str(Path(__file__).resolve().parent))

import dataset as ds  # noqa: E402
from model import InverseNet  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(description="38x6 逆算回帰モデル 学習 (MVP 3a)")
    ap.add_argument("--data", type=str, default=None, help="入力npz（既定 ../private/dataset_mvp.npz）")
    ap.add_argument("--out", type=str, default=None, help="出力チェックポイント（既定 ../private/model_mvp.pt）")
    ap.add_argument("--epochs", type=int, default=60)
    ap.add_argument("--batch", type=int, default=128)
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--weight-decay", type=float, default=1e-4, help="Adamのweight decay(過学習抑制)")
    ap.add_argument("--val-frac", type=float, default=0.1)
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()

    base = Path(__file__).resolve().parent.parent / "private"
    data_path = Path(args.data) if args.data else base / "dataset_mvp.npz"
    out_path = Path(args.out) if args.out else base / "model_mvp.pt"

    torch.manual_seed(args.seed)
    features, params, meta = ds.load_npz(data_path)
    print(f"データ: features={features.shape} params={params.shape} spec={meta.get('spec_version')}")

    f_tr, p_tr, f_val, p_val = ds.train_val_split(features, params, args.val_frac, args.seed)
    mean, std = ds.feature_stats(f_tr)
    train_set = ds.PatchDataset(f_tr, p_tr, mean, std)
    val_set = ds.PatchDataset(f_val, p_val, mean, std)
    train_loader = DataLoader(train_set, batch_size=args.batch, shuffle=True)
    val_loader = DataLoader(val_set, batch_size=args.batch, shuffle=False)

    out_dim = int(meta["dim"])
    n_mels = int(meta["n_mels"])
    n_frames = int(meta["n_frames"])
    model = InverseNet(out_dim=out_dim, n_mels=n_mels, n_frames=n_frames)
    opt = torch.optim.Adam(model.parameters(), lr=args.lr, weight_decay=args.weight_decay)
    loss_fn = torch.nn.MSELoss()

    best_val = float("inf")
    t0 = time.time()
    for epoch in range(1, args.epochs + 1):
        model.train()
        tr_loss = 0.0
        for x, y in train_loader:
            opt.zero_grad()
            loss = loss_fn(model(x), y)
            loss.backward()
            opt.step()
            tr_loss += loss.item() * x.shape[0]
        tr_loss /= len(train_set)

        model.eval()
        val_loss = 0.0
        with torch.no_grad():
            for x, y in val_loader:
                val_loss += loss_fn(model(x), y).item() * x.shape[0]
        val_loss /= len(val_set)

        if val_loss < best_val:
            best_val = val_loss
            torch.save(
                {
                    "model_state": model.state_dict(),
                    "mean": mean,  # [n_mels,1] ndarray（ビンごと標準化）
                    "std": std,
                    "out_dim": out_dim,
                    "n_mels": n_mels,
                    "n_frames": n_frames,
                    "labels": meta.get("labels"),
                    "meta": meta,
                },
                out_path,
            )
        if epoch == 1 or epoch % 5 == 0 or epoch == args.epochs:
            print(f"epoch {epoch:3d}  train_mse={tr_loss:.5f}  val_mse={val_loss:.5f}  best={best_val:.5f}")

    dt = time.time() - t0
    print(f"完了: best_val_mse={best_val:.5f}  所要={dt:.1f}s  保存={out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
