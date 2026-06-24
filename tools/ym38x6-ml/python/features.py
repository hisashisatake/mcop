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


def abys_features(
    samples,
    sr: float = 44100.0,
    n_fft: int = 1024,
    hop: int = 512,
    n_mels: int = 64,
    n_frames: int = 64,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """A-by-S用の多項特徴を1回のSTFTから返す。

    返り値 (log_mel, env, centroid):
      - log_mel  : [n_mels, n_frames] 対数メル（log_mel()と同一の粗スペクトル）
      - env      : [n_frames] フレーム毎ラウドネス曲線（最大1に正規化＝形だけ）。
                   アタック/ディケイのタイミングを直接拘束する（音量レベルではなく時間形状）。
      - centroid : [n_frames] スペクトル重心（mel尺度を0..1正規化）。明るさ＋音程に効く。
    1回のSTFTを共有して3特徴を作る（損失内で毎回呼ぶため効率優先）。
    """
    x = np.asarray(samples, dtype=np.float64)
    if x.size < n_fft:
        x = np.pad(x, (0, n_fft - x.size))
    win = np.hanning(n_fft)
    n_hop = 1 + (x.size - n_fft) // hop
    frames = np.stack([x[i * hop:i * hop + n_fft] * win for i in range(n_hop)])  # [n_hop, n_fft]
    power = np.abs(np.fft.rfft(frames, axis=1)) ** 2  # [n_hop, n_bins]

    # 対数メル（log_mel()と同じ経路）
    fb = mel_filterbank(sr, n_fft, n_mels)
    mel = power @ fb.T  # [n_hop, n_mels]
    log_mel_pooled = _pool_time(np.log1p(mel), n_frames).T.astype(np.float32)  # [n_mels, n_frames]

    # ラウドネス曲線（log圧縮→時間プール→最大1正規化で「形」だけ取り出す）
    energy = np.log1p(power.sum(axis=1))  # [n_hop]
    env = _pool_time(energy[:, None], n_frames)[:, 0]
    env = (env / (env.max() + 1e-9)).astype(np.float32)  # [n_frames]

    # スペクトル重心（mel尺度、0..1正規化）
    freqs = np.fft.rfftfreq(n_fft, d=1.0 / sr)  # [n_bins]
    psum = power.sum(axis=1) + 1e-12
    centroid_hz = (power @ freqs) / psum  # [n_hop]
    max_mel = float(_hz_to_mel(sr / 2.0))
    centroid_norm = _hz_to_mel(centroid_hz) / max_mel  # [n_hop] in [0,1]
    centroid = _pool_time(centroid_norm[:, None], n_frames)[:, 0].astype(np.float32)  # [n_frames]

    return log_mel_pooled, env, centroid


def abys_distance(
    a: tuple[np.ndarray, np.ndarray, np.ndarray],
    b: tuple[np.ndarray, np.ndarray, np.ndarray],
    w_env: float,
    w_centroid: float,
) -> tuple[float, tuple[float, float, float]]:
    """abys_features同士の多項距離。(total, (d_mel, d_env, d_centroid)) を返す。

    重心項は a（ターゲット）のラウドネス曲線で重み付けし、無音フレームの
    重心ノイズが距離を汚さないようにする。
    """
    lm_a, env_a, cen_a = a
    lm_b, env_b, cen_b = b
    d_mel = float(np.abs(lm_a - lm_b).mean())
    d_env = float(np.abs(env_a - env_b).mean())
    w = env_a  # ターゲットのラウドネスを重みに（鳴っている所だけ重心を見る）
    d_cen = float((w * np.abs(cen_a - cen_b)).sum() / (w.sum() + 1e-9))
    total = d_mel + w_env * d_env + w_centroid * d_cen
    return total, (d_mel, d_env, d_cen)


def harmonic_features(
    samples,
    freq: float,
    sr: float = 44100.0,
    n_fft: int = 4096,
    n_harmonics: int = 16,
    width_bins: int = 3,
) -> np.ndarray:
    """基音 freq の高調波振幅ベクトル [n_harmonics] を返す（最大1正規化）。

    全フレームの平均 magnitude を使い、各高調波周波数周辺 ±width_bins ビンの
    最大値を取り出すことで、小数ビンずれや軽微な音程揺れにロバストにする。
    n_fft=4096（周波数分解能 ≈ 10.8 Hz/bin @44100 Hz）で倍音構造を精細に比較する。
    """
    x = np.asarray(samples, dtype=np.float64)
    if len(x) < n_fft:
        x = np.pad(x, (0, n_fft - len(x)))
    win = np.hanning(n_fft)
    hop = n_fft // 2
    n_fr = 1 + (len(x) - n_fft) // hop
    mags = [np.abs(np.fft.rfft(x[i * hop:i * hop + n_fft] * win))
            for i in range(n_fr)]
    mag = np.stack(mags).mean(axis=0)  # [n_fft//2+1]

    n_bins = len(mag)
    amps = np.zeros(n_harmonics, dtype=np.float64)
    for k in range(1, n_harmonics + 1):
        hz = k * freq
        if hz >= sr / 2:
            break
        bin_i = int(round(hz * n_fft / sr))
        lo = max(0, bin_i - width_bins)
        hi = min(n_bins, bin_i + width_bins + 1)
        if lo < hi:
            amps[k - 1] = mag[lo:hi].max()

    peak = amps.max()
    if peak > 1e-12:
        amps /= peak
    return amps.astype(np.float32)


def harmonic_distance(a: np.ndarray, b: np.ndarray) -> float:
    """高調波振幅ベクトルのL1距離（最大1正規化済み前提、0..1 スケール）。"""
    return float(np.abs(a - b).mean())


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
