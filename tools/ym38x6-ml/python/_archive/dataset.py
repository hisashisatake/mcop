"""フェーズ6 マイルストーン3a: generate.py が出した npz を torch Dataset に載せる。

npz は (features=[n, n_mels, n_frames], params=[n, DIM]) を持つ（generate.py 参照）。
入力=対数メル特徴（CNN用に channel 次元を足して [1, n_mels, n_frames]）、目標=正規化params([0,1])。

特徴は log1p メルでスケールがバラつくため、train 統計(平均/標準偏差)で標準化する。
推論時に同じ統計を使えるよう、統計は学習チェックポイントへ保存する（train.py 側）。
"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import Dataset


def load_npz(path: str | Path) -> tuple[np.ndarray, np.ndarray, dict]:
    """npz を読み、(features[n,n_mels,n_frames], params[n,DIM], meta dict) を返す。"""
    data = np.load(path, allow_pickle=False)
    features = data["features"].astype(np.float32)
    params = data["params"].astype(np.float32)
    meta = json.loads(str(data["meta"]))
    return features, params, meta


def feature_stats(features: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """train 特徴からメルビンごとの標準化統計 (mean[n_mels,1], std[n_mels,1]) を算出する。

    グローバル標準化だとメルビン間のスケール差（低域が大きく高域が小さい等）を潰し、
    弱いが識別に効く帯域が無視されがち。ビンごとに標準化して各帯域を対等に扱う。
    std の下限でゼロ割を防ぐ。
    """
    mean = features.mean(axis=(0, 2), keepdims=False)[:, None].astype(np.float32)  # [n_mels,1]
    std = features.std(axis=(0, 2), keepdims=False)[:, None].astype(np.float32)
    std = np.maximum(std, 1e-6).astype(np.float32)
    return mean, std


class PatchDataset(Dataset):
    """(標準化済み特徴[1,n_mels,n_frames], 正規化params[DIM]) を返す Dataset。"""

    def __init__(self, features: np.ndarray, params: np.ndarray, mean: np.ndarray, std: np.ndarray):
        # mean/std は [n_mels,1]。[n, n_mels, n_frames] にブロードキャストして標準化。
        feats = (features - mean[None]) / std[None]
        self.x = torch.from_numpy(feats[:, None, :, :].astype(np.float32))
        self.y = torch.from_numpy(params.astype(np.float32))

    def __len__(self) -> int:
        return self.x.shape[0]

    def __getitem__(self, i: int):
        return self.x[i], self.y[i]


def train_val_split(
    features: np.ndarray,
    params: np.ndarray,
    val_frac: float = 0.1,
    seed: int = 0,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """[n,...] を train/val にシャッフル分割して (f_tr, p_tr, f_val, p_val) を返す。"""
    n = features.shape[0]
    rng = np.random.default_rng(seed)
    perm = rng.permutation(n)
    n_val = max(1, int(round(n * val_frac)))
    val_idx, tr_idx = perm[:n_val], perm[n_val:]
    return features[tr_idx], params[tr_idx], features[val_idx], params[val_idx]
