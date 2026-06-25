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


DESCRIPTOR_WEIGHTS: dict[str, float] = {
    "brightness":       3.0,
    "warmth":           2.0,
    "distortion":       4.0,  # 倍音の豊かさ（ブラス・リード選別に重要）
    "attack":           0.5,  # 録音依存のため低減（field_rangesで制約）
    "decay":            0.0,  # 録音の減衰を拾うため除外（同上）
    "metallic":         1.5,
    "odd_even":         1.5,
    "brightness_decay": 1.0,
    "roughness":        0.5,
    "noise":            0.3,
}


def descriptor_distance(
    target: dict[str, float],
    pred: dict[str, float],
    weights: dict[str, float] | None = None,
) -> tuple[float, dict[str, float]]:
    """知覚記述子ベクトル間の重み付きL1距離。(total, per_key_terms) を返す。"""
    if weights is None:
        weights = DESCRIPTOR_WEIGHTS
    terms = {k: w * abs(target.get(k, 0.0) - pred.get(k, 0.0)) for k, w in weights.items()}
    return float(sum(terms.values())), terms


def perceptual_descriptors(
    samples,
    freq: float,
    sr: float = 44100.0,
    n_fft_harm: int = 4096,
    n_harmonics: int = 16,
    width_bins: int = 3,
    n_frames: int = 64,
    hop: int = 512,
) -> dict[str, float]:
    """知覚記述子 10 軸を計算して dict で返す。

    A-by-S の損失項および GM2 音色空間の可視化に共用する。
    freq: 基音周波数 Hz（倍音解析に使用）

    Keys:
        brightness       : 平均スペクトル重心（0=暗, 1=明）
        warmth           : 低次倍音(1-3)エネルギー比（0=冷, 1=暖）
        metallic         : 非整数倍音エネルギー比（インハーモニシティ）
        roughness        : 低域帯の倍音間エネルギー比（DT1 ノイズ・不協和）
        distortion       : 高次倍音(8+)エネルギー比（歪み・倍音密度）
        odd_even         : 奇数倍音エネルギー比（1=矩形波感）
        noise            : スペクトル平坦度（0=調性音, 1=白色雑音）
        attack           : アタック速度（env ピーク位置; 0=遅い, 1=瞬時）
        decay            : 減衰速度（ピーク後の絶対傾き; 0=持続, 1=急減衰）
        brightness_decay : 明るさの時間変化（前半重心 - 後半重心）
    """
    x = np.asarray(samples, dtype=np.float64)

    # ── 高分解能 FFT（倍音構造解析）───────────────────────────────────────
    x_h = x if len(x) >= n_fft_harm else np.pad(x, (0, n_fft_harm - len(x)))
    win_h = np.hanning(n_fft_harm)
    hop_h = n_fft_harm // 2
    n_fr_h = max(1, 1 + (len(x_h) - n_fft_harm) // hop_h)
    mags_h = np.stack([
        np.abs(np.fft.rfft(x_h[i * hop_h: i * hop_h + n_fft_harm] * win_h))
        for i in range(n_fr_h)
    ])
    mag = mags_h.mean(axis=0)   # [n_bins]
    pwr = mag ** 2               # [n_bins]
    n_bins = len(mag)
    total_energy = float(pwr.sum()) + 1e-12

    # ── 倍音ビン収集 ──────────────────────────────────────────────────────
    amps = np.zeros(n_harmonics, dtype=np.float64)
    harm_energy = 0.0
    for k in range(1, n_harmonics + 1):
        hz_k = k * freq
        if hz_k >= sr / 2.0:
            break
        bin_c = int(round(hz_k * n_fft_harm / sr))
        lo = max(0, bin_c - width_bins)
        hi = min(n_bins, bin_c + width_bins + 1)
        if lo < hi:
            amps[k - 1] = float(mag[lo:hi].max())
            harm_energy += float(pwr[lo:hi].sum())

    amp2 = amps ** 2
    total_amp2 = float(amp2.sum()) + 1e-12

    # ── 1. 暖かさ（1-3 倍音エネルギー比）────────────────────────────────
    warmth = float(amp2[:3].sum() / total_amp2)

    # ── 2. 金属度（非整数倍音エネルギー比）──────────────────────────────
    metallic = float(1.0 - harm_energy / total_energy)

    # ── 3. ラフネス（低域帯の非倍音エネルギー比）─────────────────────────
    # 1-7 倍音の範囲内で、倍音ピーク以外に残るエネルギー。
    # FM の DT1 サイドバンドや PCM の不協和成分を捉える。
    low_max_bin = min(int(7.5 * freq * n_fft_harm / sr) + 1, n_bins)
    harm_mask = np.zeros(n_bins, dtype=bool)
    for k in range(1, 9):
        hz_k = k * freq
        if hz_k >= sr / 2.0:
            break
        bin_c = int(round(hz_k * n_fft_harm / sr))
        lo = max(0, bin_c - width_bins * 2)
        hi = min(n_bins, bin_c + width_bins * 2 + 1)
        harm_mask[lo:hi] = True
    low_total = float(pwr[:low_max_bin].sum()) + 1e-12
    between_energy = float(pwr[:low_max_bin][~harm_mask[:low_max_bin]].sum())
    roughness = float(between_energy / low_total)

    # ── 4. 歪み / 倍音の豊かさ（8 倍音以降エネルギー比）────────────────
    distortion = float(amp2[7:].sum() / total_amp2)

    # ── 5. 奇偶比（奇数倍音エネルギー比）────────────────────────────────
    odd_e = float(amp2[0::2].sum())   # index 0,2,4,… = 1,3,5,… 倍音
    even_e = float(amp2[1::2].sum())  # index 1,3,5,… = 2,4,6,… 倍音
    odd_even = float(odd_e / (odd_e + even_e + 1e-12))

    # ── 6. ノイズ性（スペクトル平坦度: 幾何平均 / 算術平均）────────────
    p_pos = pwr[pwr > 0]
    if len(p_pos) > 1:
        log_geo = float(np.mean(np.log(p_pos)))
        arith = float(p_pos.mean())
        noise = float(np.clip(np.exp(log_geo) / (arith + 1e-30), 0.0, 1.0))
    else:
        noise = 0.0

    # ── 7. 時間軸（エンベロープ + 重心）──────────────────────────────────
    n_fft_env = 1024
    x_e = x if len(x) >= n_fft_env else np.pad(x, (0, n_fft_env - len(x)))
    win_e = np.hanning(n_fft_env)
    n_hop_e = max(1, 1 + (len(x_e) - n_fft_env) // hop)
    frames_e = np.stack([
        x_e[i * hop: i * hop + n_fft_env] * win_e
        for i in range(n_hop_e)
    ])
    pwr_e = np.abs(np.fft.rfft(frames_e, axis=1)) ** 2  # [n_hop, n_bins_env]

    e_log = np.log1p(pwr_e.sum(axis=1))
    env_pooled = _pool_time(e_log[:, None], n_frames)[:, 0]
    env = env_pooled / (env_pooled.max() + 1e-9)

    f_env = np.fft.rfftfreq(n_fft_env, d=1.0 / sr)
    p_sum_e = pwr_e.sum(axis=1) + 1e-12
    cen_hz = (pwr_e @ f_env) / p_sum_e
    max_mel_v = float(_hz_to_mel(sr / 2.0))
    cen_norm = _hz_to_mel(cen_hz) / max_mel_v
    centroid = _pool_time(cen_norm[:, None], n_frames)[:, 0]

    brightness = float(centroid.mean())

    peak_i = int(np.argmax(env))
    attack = float(1.0 - peak_i / max(n_frames - 1, 1))

    post = env[peak_i:]
    if len(post) > 2:
        t_post = np.arange(len(post), dtype=np.float64) / max(n_frames - 1, 1)
        slope = float(np.polyfit(t_post, post, 1)[0])
        # tanh で非線形圧縮（|slope|=1→0.76, |slope|=0.5→0.46, |slope|=0→0）
        decay = float(np.tanh(max(-slope, 0.0)))
    else:
        decay = 0.0

    half = n_frames // 2
    brightness_decay = float(centroid[:half].mean() - centroid[half:].mean())

    return {
        "brightness":       brightness,
        "warmth":           warmth,
        "metallic":         metallic,
        "roughness":        roughness,
        "distortion":       distortion,
        "odd_even":         odd_even,
        "noise":            noise,
        "attack":           attack,
        "decay":            decay,
        "brightness_decay": brightness_decay,
    }
