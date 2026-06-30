"""フェーズ6 マイルストーン3a: 逆算回帰モデル（小型CNN）。

入力=標準化済み対数メル特徴 [B,1,n_mels,n_frames]、出力=正規化params [B,out_dim]（[0,1]）。
出力に sigmoid を掛け、param_space の vector_to_patch がそのまま食える範囲に収める。

まずは素直な 3conv→GAP→MLP。識別力が足りなければ後段で深く/広くする。
"""

from __future__ import annotations

import torch
import torch.nn as nn


class InverseNet(nn.Module):
    def __init__(self, out_dim: int = 35, n_mels: int = 64, n_frames: int = 16):
        super().__init__()
        # GAPは使わない: 音色の核心「どの帯域・どの時刻が強いか」を潰さないよう、
        # conv後の特徴マップを位置情報ごとFlattenしてMLPに渡す。
        self.features = nn.Sequential(
            nn.Conv2d(1, 16, 3, padding=1),
            nn.BatchNorm2d(16),
            nn.ReLU(inplace=True),
            nn.MaxPool2d(2),  # [16, n_mels/2, n_frames/2]
            nn.Conv2d(16, 32, 3, padding=1),
            nn.BatchNorm2d(32),
            nn.ReLU(inplace=True),
            nn.MaxPool2d(2),  # [32, n_mels/4, n_frames/4]
        )
        flat = 32 * (n_mels // 4) * (n_frames // 4)
        self.head = nn.Sequential(
            nn.Flatten(),
            nn.Linear(flat, 256),
            nn.ReLU(inplace=True),
            nn.Dropout(0.2),
            nn.Linear(256, 128),
            nn.ReLU(inplace=True),
            nn.Linear(128, out_dim),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        z = self.features(x)
        return torch.sigmoid(self.head(z))
