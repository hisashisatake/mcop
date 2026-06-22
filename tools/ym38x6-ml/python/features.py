"""フェーズ6 MVP: レンダリング音 → 固定形状の対数メルスペクトログラム（numpyのみ）。

librosa等の重い依存を避け、STFT・メルフィルタバンク・プールをnumpyで実装する。
出力形状は [n_mels, n_frames] に揃える（学習用の固定長特徴）。
"""

from __future__ import annotations

import numpy as np


def _hz_to_mel(f: np.ndarray | float) -> np.ndarray | float:
    return 2595.0 * np.log10(1.0 + np.asarray(f) / 700.0)


def _mel_to_hz(m: np.ndarray | float) -> np.ndarray | float:
    return 700.0 * (10.0 ** (np.asarray(m) / 2595.0) - 1.0)


def mel_filterbank(sr: float, n_fft: int, n_mels: int, fmin: float = 0.0, fmax: float | None = None) -> np.ndarray:
    """[n_mels, n_bins] の三角メルフィルタ行列を返す（n_bins = n_fft//2 + 1）。"""
    if fmax is None:
        fmax = sr / 2.0
    n_bins = n_fft // 2 + 1
    mel_pts = np.linspace(_hz_to_mel(fmin), _hz_to_mel(fmax), n_mels + 2)
    hz_pts = _mel_to_hz(mel_pts)
    bin_pts = np.floor((n_fft + 1) * hz_pts / sr).astype(int)
    bin_pts = np.clip(bin_pts, 0, n_bins - 1)
    fb = np.zeros((n_mels, n_bins), dtype=np.float64)
    for m in range(1, n_mels + 1):
        left, center, right = bin_pts[m - 1], bin_pts[m], bin_pts[m + 1]
        if center > left:
            fb[m - 1, left:center] = (np.arange(left, center) - left) / (center - left)
        if right > center:
            fb[m - 1, center:right] = (right - np.arange(center, right)) / (right - center)
    return fb


def _pool_time(arr: np.ndarray, n_frames: int) -> np.ndarray:
    """[T, n_mels] を時間方向で [n_frames, n_mels] へ平均プールする。"""
    t = arr.shape[0]
    if t == n_frames:
        return arr
    idx = np.linspace(0, t, n_frames + 1).astype(int)
    out = np.empty((n_frames, arr.shape[1]), dtype=arr.dtype)
    for i in range(n_frames):
        a = idx[i]
        b = max(a + 1, idx[i + 1])
        out[i] = arr[a:b].mean(axis=0)
    return out


def log_mel(
    samples,
    sr: float = 44100.0,
    n_fft: int = 1024,
    hop: int = 512,
    n_mels: int = 64,
    n_frames: int = 16,
) -> np.ndarray:
    """モノラルサンプル列 → 対数メルスペクトログラム [n_mels, n_frames]（float32）。"""
    x = np.asarray(samples, dtype=np.float64)
    if x.size < n_fft:
        x = np.pad(x, (0, n_fft - x.size))
    win = np.hanning(n_fft)
    n_hop = 1 + (x.size - n_fft) // hop
    frames = np.stack([x[i * hop:i * hop + n_fft] * win for i in range(n_hop)])  # [n_hop, n_fft]
    power = np.abs(np.fft.rfft(frames, axis=1)) ** 2  # [n_hop, n_bins]
    fb = mel_filterbank(sr, n_fft, n_mels)  # [n_mels, n_bins]
    mel = power @ fb.T  # [n_hop, n_mels]
    log = np.log1p(mel)
    pooled = _pool_time(log, n_frames)  # [n_frames, n_mels]
    return pooled.T.astype(np.float32)  # [n_mels, n_frames]


def is_silent(samples, thresh: float = 1e-3) -> bool:
    """RMS(実効値)が閾値未満なら無音(低情報)とみなす。

    ピークだと単発スパイクで誤判定しやすいためRMSで判定する。ランダムパッチは大半が
    極小音(peak中央値≈1e-3)なので、低情報サンプルを弾いてデータ品質を保つ閾値にする。
    """
    x = np.asarray(samples, dtype=np.float64)
    if x.size == 0:
        return True
    rms = float(np.sqrt(np.mean(x ** 2)))
    return rms < thresh
