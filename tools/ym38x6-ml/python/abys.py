"""フェーズ6 マイルストーン3b+: Analysis-by-Synthesis（A-by-S）改善版。

3a で「特徴→params のフィードフォワード回帰(params MSE)」は一対多のため音指標で
ベースライン以下と確定した。3b は実エンジンを直接ループに入れ、目標 log_mel との距離を
勾配なし最適化(CMA-ES)で最小化して params を求める（per-target・遅いが原理的に正しい）。

3b+: 初期化戦略として以下を統合。さらにマルチアルゴリズム・マルチ波形探索を追加。
  A. ML warm start    : model_f64.pt の推論結果を x0 に使う
  B. マルチスタート  : neutral/metallic/soft/percussive の4アーキタイプから並行探索
  C. 重心ヒューリスティック: ターゲットの明るさ/金属感で FB・MUL を初期化
  D. マルチアルゴリズム: GM2プログラム番号で推奨 Algorithm を自動選択（OPN/OPM語感準拠）
  E. マルチ波形    : GM2音種別に OPZ波形（W2=sin²…W8=sin²2倍速）を選択。
                     Algorithm ごとのキャリア/モジュレーター役割に応じて展開する。
  全組み合わせ(algo × waveform)に予算を均等分配し、全体ベストを採用。

実行モード:
  [1] 自己再構成テスト（--input-dir なし）:
      python abys.py --n-targets 5 --maxfevals 2400 --wav 5

  [2] バッチ推論（GM2 WAV → .38x6）:
      python abys.py --input-dir private/gm2_wav --out private/bank0_abys.38x6
      python abys.py --input-dir private/gm2_wav --prog-range 0-7,16,56-63
      python abys.py --input-dir private/gm2_wav --algorithms 4,6 --waveforms 0-0,2-0
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

_SILENT_PENALTY = 10.0

_DEFAULT_MODEL = Path(__file__).resolve().parent.parent / "private" / "model_f64.pt"

# ── GM2 プログラム番号 → 推奨アルゴリズムリスト（OPN/OPM語感準拠） ────────
GM2_PROG_ALGORITHMS: dict[int, list[int]] = {
    **{p: [4]    for p in range(0,   8)},   # 0-7   Piano
    **{p: [0, 2] for p in range(8,  16)},   # 8-15  Chromatic Perc（鉄琴・ベル）
    **{p: [7, 6] for p in range(16, 24)},   # 16-23 Organ
    **{p: [4, 0] for p in range(24, 32)},   # 24-31 Guitar
    **{p: [4, 0] for p in range(32, 40)},   # 32-39 Bass
    **{p: [5, 4] for p in range(40, 48)},   # 40-47 Strings
    **{p: [5, 6] for p in range(48, 56)},   # 48-55 Ensemble
    **{p: [6, 5] for p in range(56, 64)},   # 56-63 Brass
    **{p: [6, 5] for p in range(64, 72)},   # 64-71 Reed
    **{p: [7, 5] for p in range(72, 80)},   # 72-79 Pipe
    **{p: [0, 7] for p in range(80, 88)},   # 80-87 Synth Lead
    **{p: [5, 6] for p in range(88, 96)},   # 88-95 Synth Pad
    **{p: [0, 5] for p in range(96, 104)},  # 96-103 Synth Effects
    **{p: [4, 0] for p in range(104, 112)}, # 104-111 Ethnic
    **{p: [0, 1] for p in range(112, 120)}, # 112-119 Percussive
    **{p: [0, 1] for p in range(120, 128)}, # 120-127 Sound Effects
}

# ── GM2 プログラム番号 → 試す波形ペアリスト (mod_wf, car_wf) ─────────────
# waveform.rs の番号体系（4グループ × 8変換 = 32波形）:
#
# Sine group (0-7)   : W1=0(sine)～W8=7(sin²2倍速常正)  ← OPZ準拠
# Saw group  (8-15)  : Sine と同じ8変換をノコギリ基本波に適用
#                       8=full, 9=saw², 10=half, 11=half², 12=2x-half,
#                       13=2x-saw²-half, 14=2x-|saw|-half, 15=2x-pos-saw²-half
# Square group(16-23): OPZ変換が縮退するためPWMデューティスイープに差し替え
#                       16=50%(矩形), 17=33%, 18=25%, 19=16.7%, 20=12.5%,
#                       21=6.25%(超細), 22=ハーフ矩形, 23=2倍速矩形前半
# Triangle group(24-31): Sine と同じ8変換を三角波基本波に適用
#                       24=full, 25=tri², 26=half, 27=half², 28=2x-half, ...
#
# (mod_wf, car_wf) は Algorithm のキャリア定義に従い expand_waveforms() で4-tupleに展開する。

# Sine 系（OPZ W2-W8）
_WF_SINE    = (0, 0)   # 全 sine
_WF_STRINGS = (0, 2)   # carrier=half-sine(W3)  → 弦 carrier
_WF_PLUCKED = (3, 0)   # mod=half-sin²(W4)      → 撥弦 mod（ギター・ハープ・クラビ）
_WF_PLUCK5  = (4, 0)   # mod=2x-sine-half(W5)   → ギター・ハーモニカ mod
_WF_WOODWIN = (0, 1)   # carrier=sin²(W2)        → 木管 carrier
_WF_REED    = (0, 5)   # carrier=2x-sin²-half(W6)→ リード系 carrier
_WF_BRIGHT7 = (0, 6)   # carrier=W7 |sin|2倍速   → 高域強調++
_WF_BRIGHT8 = (0, 7)   # carrier=W8 sin²2倍速    → 高域強調+

# Saw 系（Sine と同じ変換体系、より倍音豊富）
_WF_SAW     = (0, 8)   # carrier=full-saw        → ブラス・シンセリード
_WF_SAW_HC  = (0, 10)  # carrier=half-saw        → 明るい弦（W3のsaw版）
_WF_SAW_HM  = (10, 0)  # mod=half-saw            → 芯の強いギター（W4のsaw版）
_WF_SAW_BM  = (8, 0)   # mod=full-saw            → 攻撃的FM（歪み系）

# Triangle 系（Sine と同じ変換体系、よりまろやか）
_WF_TRI     = (0, 24)  # carrier=full-tri        → フルート・ジェントルパッド
_WF_TRI_HC  = (0, 26)  # carrier=half-tri        → 穏やかな弦（W3のtri版）

# Square/PWM 系（独自 PWM スイープ、他3グループとは別体系）
_WF_SQ50    = (0, 16)  # carrier=50%矩形          → クラリネット（古典FM）
_WF_SQ25M   = (18, 0)  # mod=25%デューティ         → ハープシコード・薄いアタック
_WF_CLAVI_M = (21, 0)  # mod=6.25%デューティ(超細)  → クラビ・極細パルスアタック

GM2_PROG_WAVEFORM_PAIRS: dict[int, list[tuple[int, int]]] = {
    # Piano 0-5: square(50%)→中空なウッドピアノ感、triangle→柔らかなアコピ感
    **{p: [_WF_SINE, _WF_SQ50, _WF_TRI]             for p in range(0, 6)},
    # Harpsichord(6): 25%デューティ mod + square carrier → 薄いクリック感
    6:  [_WF_SINE, _WF_SQ25M, _WF_SQ50],
    # Clavi(7): 極細パルス mod + bright carrier で弾けるエッジ感
    7:  [_WF_SINE, _WF_CLAVI_M, _WF_BRIGHT8],

    # Chromatic Perc 8-14: triangle→マリンバ/ビブラフォンの柔らかさ、bright+saw→ベル/シロフォン
    **{p: [_WF_SINE, _WF_BRIGHT8, _WF_SAW, _WF_TRI] for p in range(8, 15)},
    # Dulcimer(15): 撥弦 mod + triangle carrier → 温かい撥弦感
    15: [_WF_SINE, _WF_PLUCKED, (17, 0), _WF_TRI],

    # Organ 16-23: square(50%)→ドローバー矩形波（オルガンの定番）、saw→リード系
    **{p: [_WF_SINE, _WF_SAW, _WF_SQ50]             for p in range(16, 24)},

    # Nylon Guitar(24): square→中空な胴鳴り感、triangle→柔らかな弦
    24: [_WF_SINE, _WF_PLUCKED, _WF_SQ50, _WF_TRI],
    # Guitar 25-31: 撥弦 mod + half-saw mod で倍音豊かな弦
    **{p: [_WF_SINE, _WF_PLUCKED, _WF_PLUCK5, _WF_SAW_HM] for p in range(25, 32)},

    # Bass 32-39: saw(倍音豊か) + triangle(スムーズなシンセベース)
    **{p: [_WF_SINE, _WF_SAW, _WF_TRI]              for p in range(32, 40)},

    # Strings 40-47: half-sine/half-saw/half-tri で明暗幅を確保
    **{p: [_WF_SINE, _WF_STRINGS, _WF_SAW_HC, _WF_TRI_HC] for p in range(40, 48)},

    # Ensemble 48-55: Strings と同系統
    **{p: [_WF_SINE, _WF_STRINGS, _WF_SAW_HC, _WF_TRI_HC] for p in range(48, 56)},

    # Brass 56-63: saw→古典FMブラス、bright8→明るいトランペット、tri→丸みあるホルン/チューバ
    **{p: [_WF_SINE, _WF_SAW, _WF_BRIGHT8, _WF_TRI] for p in range(56, 64)},

    # Reed 64-70: W2/W6/saw で木管〜金属リード幅
    **{p: [_WF_SINE, _WF_WOODWIN, _WF_REED, _WF_SAW] for p in range(64, 71)},
    # Clarinet(71): square carrier が古典FMクラリネット + tri で柔らかめ
    71: [_WF_SINE, _WF_SQ50, _WF_WOODWIN, _WF_TRI],

    # Pipe 72-79: triangle + half-sine(W3) でフルート〜リコーダー幅
    **{p: [_WF_SINE, _WF_WOODWIN, _WF_TRI, _WF_STRINGS] for p in range(72, 80)},

    # Synth Lead 80-87: square(50%)→矩形波リード（必須）、saw+bright でシンセらしさ
    **{p: [_WF_SINE, _WF_SAW, _WF_SQ50, _WF_BRIGHT8] for p in range(80, 88)},

    # Synth Pad 88-95: sine/half-sine/half-tri/half-saw で柔硬バリエーション
    **{p: [_WF_SINE, _WF_STRINGS, _WF_TRI_HC, _WF_SAW_HC] for p in range(88, 96)},

    # Synth Effects 96-103: bright系 + saw でFX幅
    **{p: [_WF_SINE, _WF_SAW, _WF_BRIGHT8, _WF_BRIGHT7] for p in range(96, 104)},

    # Ethnic 104-111: 撥弦 mod + triangle carrier でシタール・琴系の温かさ
    **{p: [_WF_SINE, _WF_PLUCKED, _WF_PLUCK5, _WF_TRI] for p in range(104, 112)},

    # Percussive 112-119: bright8→金属系ベル(Agogo/Bells/Woodblock)
    **{p: [_WF_SINE, _WF_BRIGHT8]                    for p in range(112, 120)},

    # Sound Effects 120-127: sine のみ（多様すぎて汎用化困難）
    **{p: [_WF_SINE]                                  for p in range(120, 128)},
}


# ── GM2 プログラム番号 → エンベロープ制約 field_ranges ─────────────────────
# 楽器カテゴリ毎に「この範囲に収まるべき」envelope パラメーターを制約し、
# CMA-ES が音楽的に意味のない领域（ピアノなのにAR=30等）を探索しないようにする。
# キャリアTLは param_space.CARRIER_TL_MIN で別途制約済み。モジュレーターTLは音色の核心なので制約しない。
# 値は [vmin, vmax]（0-255 スケール）。辞書にないフィールドは [0, 255] フリー。

# D1L の向き（operator.rs 準拠）:
#   d1l=255 → D1R が即 D2R へ移行（ピーク音量から D2R 開始） → サステイン大 → パッド/オルガン/弦向き
#   d1l=0   → D1R が無音まで全域減衰してから D2R へ           → サステイン小 → ピアノ/撥弦/打楽器向き
#
# AR/RR の目安（mapping.rs の指数カーブより）:
#   AR=80→830ms  AR=100→365ms  AR=150→48ms  AR=200→6ms  AR=255→0.68ms（瞬時）
#   RR=80→12s    RR=100→5.2s   RR=128→1.6s  RR=150→660ms  RR=180→190ms  RR=255→8.7ms
#   AR=0はフリーズ（無音）、RR=0は285秒で減衰（フリーズではない）

_FR_PIANO   = {"ar": (200, 255), "d1r": (80, 255),  "d1l": (0,  80),  "rr": (80, 255)}
_FR_HARPSI  = {"ar": (220, 255), "d1r": (180, 255), "d1l": (0,  40),  "rr": (180, 255)}
_FR_MALLET  = {"ar": (220, 255), "d1r": (50, 200),  "d1l": (30, 150), "d2r": (20, 150), "rr": (80, 200)}
_FR_ORGAN   = {"ar": (200, 255), "d1r": (0,  30),   "d1l": (220, 255),"d2r": (0,  20),  "rr": (80, 150)}
_FR_GUITAR  = {"ar": (200, 255), "d1r": (100, 255), "d1l": (0,  80),  "rr": (80, 255)}
_FR_BASS    = {"ar": (180, 255), "d1r": (50, 200),  "d1l": (0, 100),  "rr": (80, 255)}
_FR_STRINGS = {"ar": (40, 120),  "d1r": (0,  80),   "d1l": (150, 255),"d2r": (0,  60),  "rr": (80, 150)}
_FR_BRASS_LEAD    = {"ar": (150, 230), "d1r": (40, 180),  "d1l": (80, 200), "rr": (100, 200)}  # Trumpet/MutedTrumpet
_FR_BRASS_BACKING = {"ar": (60,  130), "d1r": (20, 100),  "d1l": (120, 240),"rr": (80,  150)}  # Trombone/Horn/Tuba/Section
_FR_SAX     = {"ar": (120, 210), "d1r": (30, 150),  "d1l": (80, 200), "rr": (80, 170)}  # 64-67 Saxophone
_FR_REED    = {"ar": (100, 200), "d1r": (20, 120),  "d1l": (80, 200), "rr": (80, 160)}  # 68-71 Oboe/Bassoon/Clarinet
_FR_PIPE    = {"ar": (50, 200),  "d1r": (20, 100),  "d1l": (150, 255),"d2r": (0,  40),  "rr": (80, 150)}
_FR_LEAD    = {"ar": (100, 255)}
_FR_PAD     = {"ar": (80, 150),  "d1r": (0,  60),   "d1l": (180, 255),"d2r": (0,  30),  "rr": (100, 160)}
_FR_ETHNIC  = {"ar": (150, 255), "d1r": (50, 200),  "d1l": (0, 120),  "rr": (80, 200)}
_FR_PERC    = {"ar": (220, 255), "d1r": (100, 255), "d1l": (0,  60),  "d2r": (50, 255), "rr": (100, 255)}

GM2_PROG_FIELD_RANGES: dict[int, dict] = {
    **{p: _FR_PIANO  for p in range(0, 6)},    # 0-5  Piano
    6:  _FR_HARPSI,                              # 6    Harpsichord
    7:  _FR_HARPSI,                              # 7    Clavi
    **{p: _FR_MALLET for p in range(8, 16)},    # 8-15 Chromatic Perc
    **{p: _FR_ORGAN  for p in range(16, 24)},   # 16-23 Organ
    **{p: _FR_GUITAR for p in range(24, 32)},   # 24-31 Guitar
    **{p: _FR_BASS   for p in range(32, 40)},   # 32-39 Bass
    **{p: _FR_STRINGS for p in range(40, 56)},  # 40-55 Strings / Ensemble
    56: _FR_BRASS_LEAD,                            # 56 Trumpet
    57: _FR_BRASS_BACKING,                         # 57 Trombone
    58: _FR_BRASS_BACKING,                         # 58 Tuba
    59: _FR_BRASS_LEAD,                            # 59 Muted Trumpet
    60: _FR_BRASS_BACKING,                         # 60 French Horn
    **{p: _FR_BRASS_BACKING for p in range(61, 64)},  # 61-63 Brass Section / Synth Brass
    **{p: _FR_SAX  for p in range(64, 68)},       # 64-67 Saxophone
    **{p: _FR_REED for p in range(68, 72)},        # 68-71 Oboe / Bassoon / Clarinet
    **{p: _FR_PIPE   for p in range(72, 80)},   # 72-79 Pipe
    **{p: _FR_LEAD   for p in range(80, 88)},   # 80-87 Synth Lead
    **{p: _FR_PAD    for p in range(88, 96)},   # 88-95 Synth Pad
    **{p: _FR_ETHNIC for p in range(104, 112)}, # 104-111 Ethnic
    **{p: _FR_PERC   for p in range(112, 120)}, # 112-119 Percussive
}


# ── ユーティリティ ─────────────────────────────────────────────────────────

def note_freq(name: str = "C", octave: int = 4) -> float:
    semis = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
    midi = (octave + 1) * 12 + semis[name]
    return 440.0 * 2 ** ((midi - 69) / 12)


def _midi_to_freq(note: int) -> float:
    return 440.0 * 2 ** ((note - 69) / 12.0)


def _parse_prog_set(s: str) -> set[int]:
    """'0-7,16,56-63' → {0..7, 16, 56..63}。カンマと範囲を混在可能。"""
    result: set[int] = set()
    for part in s.split(","):
        part = part.strip()
        if "-" in part:
            a, b = part.split("-", 1)
            result.update(range(int(a), int(b) + 1))
        else:
            result.add(int(part))
    return result


def _parse_wf_pairs(s: str) -> list[tuple[int, int]]:
    """'0-0,2-0,0-2' → [(0,0),(2,0),(0,2)]。--waveforms オプション用。"""
    pairs = []
    for part in s.split(","):
        m, c = part.strip().split("-", 1)
        pairs.append((int(m), int(c)))
    return pairs


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


def render(vec, freq, on, release, sr,
           algorithm: int = ps.FIXED_ALGORITHM,
           waveforms: tuple[int, ...] = (0, 0, 0, 0),
           field_ranges: dict | None = None):
    v = np.clip(np.asarray(vec, dtype=np.float64), 0.0, 1.0)
    return ym38x6_ml.render_patch(
        ps.vector_to_patch_json(v, algorithm, waveforms, field_ranges), freq, on, release, 115, sr)


def feats_of(samples, sr, n_mels, n_frames):
    return ft.abys_features(samples, sr=sr, n_mels=n_mels, n_frames=n_frames)


# ── 初期化戦略 A: ML warm start ────────────────────────────────────────────

def _ml_x0(target_samples, sr: float, model, ckpt) -> np.ndarray:
    """MLモデル推論から x0 を生成する（Algorithm=4・全 W1 で学習済み）。"""
    import torch

    n_mels = int(ckpt["n_mels"])
    n_frames = int(ckpt["n_frames"])
    mean = ckpt["mean"]
    std = ckpt["std"]

    mel = ft.log_mel(target_samples, sr=sr, n_mels=n_mels, n_frames=n_frames)
    x_norm = ((mel - mean) / std)[None, None, :, :]
    with torch.no_grad():
        vec = model(torch.from_numpy(x_norm.astype(np.float32)))[0].numpy()
    return np.clip(vec.astype(np.float64), 0.0, 1.0)


# ── 初期化戦略 B: アーキタイプ ────────────────────────────────────────────

def _make_archetypes() -> list[tuple[str, np.ndarray]]:
    """4種のアーキタイプ x0 を (name, vector) で返す（[0,1] 空間）。"""
    lab = ps.LABELS

    fc_idx = lab.index("ch.filter_cutoff")

    neutral = np.full(ps.DIM, 0.5)
    neutral[fc_idx] = 1.0

    metallic = np.full(ps.DIM, 0.5)
    metallic[lab.index("ch.feedback")] = 0.93
    metallic[lab.index("op0.mul")] = 7 / 15
    metallic[lab.index("op1.mul")] = 13 / 15
    metallic[lab.index("op2.mul")] = 7 / 15
    metallic[lab.index("op3.mul")] = 7 / 15
    metallic[lab.index("op0.ar")] = 0.9
    metallic[lab.index("op1.ar")] = 0.9
    metallic[lab.index("op0.dt1")] = 0.35
    metallic[lab.index("op2.dt1")] = 0.65
    metallic[fc_idx] = 1.0

    soft = np.full(ps.DIM, 0.45)
    soft[lab.index("ch.feedback")] = 0.05
    for op in range(4):
        soft[lab.index(f"op{op}.mul")] = 1 / 15
        soft[lab.index(f"op{op}.ar")] = 0.6
        soft[lab.index(f"op{op}.d1r")] = 0.08
        soft[lab.index(f"op{op}.d1l")] = 0.8
    soft[fc_idx] = 1.0

    percussive = np.full(ps.DIM, 0.5)
    for op in range(4):
        percussive[lab.index(f"op{op}.ar")] = 0.95
        percussive[lab.index(f"op{op}.d1r")] = 0.65
        percussive[lab.index(f"op{op}.d1l")] = 0.05
        percussive[lab.index(f"op{op}.rr")] = 0.75
    percussive[lab.index("ch.feedback")] = 0.25
    percussive[fc_idx] = 1.0

    return [("neutral", neutral), ("metallic", metallic),
            ("soft", soft), ("percussive", percussive)]


# ── 初期化戦略 C: 重心ヒューリスティック ─────────────────────────────────

def _heuristic_x0(target_feats: tuple) -> np.ndarray:
    """mean_centroid→MUL、centroid CV→feedback、env峰位置→AR のヒューリスティック。"""
    lab = ps.LABELS
    _, env, centroid = target_feats

    mean_cen = float(np.mean(centroid))
    cv_cen = float(np.std(centroid) / (mean_cen + 1e-9))

    x = np.full(ps.DIM, 0.5)
    mul_val = float(np.clip(0.15 + mean_cen * 0.85, 0.0, 1.0))
    for op in range(4):
        x[lab.index(f"op{op}.mul")] = mul_val
    x[lab.index("ch.feedback")] = float(np.clip(cv_cen * 1.5, 0.0, 1.0))
    peak_pos = float(np.argmax(env)) / max(len(env) - 1, 1)
    ar_val = float(np.clip(1.0 - peak_pos * 0.6, 0.2, 1.0))
    for op in range(4):
        x[lab.index(f"op{op}.ar")] = ar_val
    x[lab.index("ch.filter_cutoff")] = 1.0   # フィルターは基本的に全開
    return x


# ── 初期化戦略 D: 記述子ヒューリスティック ───────────────────────────────

def _heuristic_x0_desc(target_desc: dict) -> np.ndarray:
    """perceptual_descriptors から x0 を生成する（descriptor 損失モード用）。"""
    lab = ps.LABELS
    x = np.full(ps.DIM, 0.5)
    brightness = target_desc.get("brightness", 0.3)
    mul_val = float(np.clip(0.1 + brightness * 2.0, 0.0, 1.0))
    for op in range(4):
        x[lab.index(f"op{op}.mul")] = mul_val
    metallic = target_desc.get("metallic", 0.0)
    x[lab.index("ch.feedback")] = float(np.clip(metallic * 0.8, 0.0, 1.0))
    attack = target_desc.get("attack", 0.9)
    for op in range(4):
        x[lab.index(f"op{op}.ar")] = float(np.clip(attack * 0.8 + 0.1, 0.0, 1.0))
    decay = target_desc.get("decay", 0.5)
    for op in range(4):
        x[lab.index(f"op{op}.d1r")] = float(np.clip(decay * 0.7, 0.0, 1.0))
    x[lab.index("ch.filter_cutoff")] = 1.0
    return x


# ── 候補リスト構築 ────────────────────────────────────────────────────────

def _build_candidates(target_feats, target_samples, sr, model, ckpt,
                      ref_patches: list[dict] | None = None,
                      target_desc: dict | None = None):
    candidates: list[tuple[str, np.ndarray]] = []
    if model is not None:
        try:
            x0_ml = _ml_x0(np.asarray(target_samples, dtype=np.float64), sr, model, ckpt)
            candidates.append(("ml", x0_ml))
        except Exception as e:
            print(f"    [ML warm start スキップ: {e}]")
    candidates.extend(_make_archetypes())
    heur = (_heuristic_x0_desc(target_desc) if target_desc is not None
            else _heuristic_x0(target_feats))
    candidates.append(("heuristic", heur))
    if ref_patches:
        seen_names: set[str] = set()
        for preset in ref_patches:
            if preset["name"] not in seen_names:
                seen_names.add(preset["name"])
                vec = ps.patch_json_to_vector(preset["patch"])
                candidates.append((f"ref:{preset['name']}", vec))
    return candidates


# ── CMA-ES ランナー ────────────────────────────────────────────────────────

def _run_cma(f, x0: np.ndarray, sigma0: float, maxfevals: int,
             restarts: int, seed: int) -> tuple[np.ndarray, float, int]:
    opts = {"bounds": [0.0, 1.0], "maxfevals": maxfevals, "seed": seed, "verbose": -9}
    xbest, es = cma.fmin2(f, x0.copy(), sigma0, options=opts,
                          restarts=restarts, incpopsize=2)
    return (np.clip(np.asarray(xbest, dtype=np.float64), 0.0, 1.0),
            float(es.result.fbest),
            int(es.result.evaluations))


# ── fit_one: マルチアルゴリズム × マルチ波形 × マルチスタート ─────────────

def fit_one(
    target_feats: tuple,
    target_samples,
    freq: float, on: float, release: float, sr: float,
    n_mels: int, n_frames: int,
    w_env: float, w_centroid: float,
    sigma0: float, maxfevals: int, restarts: int, seed: int,
    algorithms: list[int] | None = None,
    wf_pairs: list[tuple[int, int]] | None = None,
    field_ranges: dict | None = None,
    model=None,
    ckpt=None,
    w_harmonic: float = 0.0,
    ref_patches: list[dict] | None = None,
    loss_mode: str = "descriptor",
    target_desc: dict | None = None,
    desc_n_fft: int = 2048,
) -> tuple[np.ndarray, float, int, str, list[tuple[str, float]]]:
    """マルチアルゴリズム × マルチ波形 × マルチスタートで A-by-S を実行。

    algorithms × wf_pairs の全組み合わせに予算を均等分配し、全体ベストを返す。
    field_ranges: envelope パラメーターの範囲制約（GM2_PROG_FIELD_RANGES から渡す）。
    w_harmonic: 高調波損失の重み（0 で無効）。

    戻り値:
        (xbest, best_dist, total_evals, winner, combo_summary)
        winner       : "alg4_w30/soft" のような "algN_wMC/候補名" 形式
        combo_summary: [(f"alg{N}_w{M}{C}", best_dist), ...] 組み合わせ毎のベスト
    """
    if algorithms is None:
        algorithms = [ps.FIXED_ALGORITHM]
    if wf_pairs is None:
        wf_pairs = [_WF_SINE]

    target_harms: np.ndarray | None = None
    if w_harmonic > 0.0:
        target_harms = ft.harmonic_features(
            np.asarray(target_samples, dtype=np.float64), freq, sr)

    candidates = _build_candidates(target_feats, target_samples, sr, model, ckpt, ref_patches,
                                   target_desc=target_desc)
    n_combos = len(algorithms) * len(wf_pairs)
    n_cands = len(candidates)
    budget_per_combo = max(100 * n_cands, maxfevals // n_combos)
    budget_each = max(100, budget_per_combo // n_cands)

    # (algo, mod_wf, car_wf, cand_name, xbest, fbest, evals)
    all_results: list[tuple[int, int, int, str, np.ndarray, float, int]] = []

    for algo in algorithms:
        for mod_wf, car_wf in wf_pairs:
            wf_config = ps.expand_waveforms(mod_wf, car_wf, algo)

            def f(vec, _algo=algo, _wf=wf_config, _fr=field_ranges):
                s = render(vec, freq, on, release, sr, _algo, _wf, _fr)
                if ft.is_silent(s):
                    return _SILENT_PENALTY
                if loss_mode == "descriptor" and target_desc is not None:
                    pred_desc = ft.perceptual_descriptors(
                        np.asarray(s, dtype=np.float64), freq, sr, n_fft_harm=desc_n_fft)
                    total, _ = ft.descriptor_distance(target_desc, pred_desc)
                    return total
                total, _ = ft.abys_distance(
                    target_feats, feats_of(s, sr, n_mels, n_frames), w_env, w_centroid)
                if target_harms is not None:
                    rh = ft.harmonic_features(
                        np.asarray(s, dtype=np.float64), freq, sr)
                    total += w_harmonic * ft.harmonic_distance(target_harms, rh)
                return total

            for name, x0 in candidates:
                xb, fb, ev = _run_cma(f, x0, sigma0, budget_each, restarts, seed)
                all_results.append((algo, mod_wf, car_wf, name, xb, fb, ev))

    best = min(all_results, key=lambda r: r[5])
    b_algo, b_mod, b_car, b_cand, xbest, fbest, _ = best
    total_evals = sum(r[6] for r in all_results)
    winner = f"alg{b_algo}_w{b_mod}{b_car}/{b_cand}"

    combo_summary: list[tuple[str, float]] = []
    for algo in algorithms:
        for mod_wf, car_wf in wf_pairs:
            key = f"alg{algo}_w{mod_wf}{car_wf}"
            best_for = min(
                (r[5] for r in all_results if r[0] == algo and r[1] == mod_wf and r[2] == car_wf),
                default=np.inf)
            combo_summary.append((key, best_for))

    return xbest, fbest, total_evals, winner, combo_summary


# ── 参照パッチユーティリティ ──────────────────────────────────────────────

def load_ref_patches(path: Path) -> list[dict]:
    """ref.38x6 ファイルから presets リストを返す。各要素: {program, name, patch}"""
    import json
    import re
    text = path.read_text(encoding="utf-8")
    # trailing comma before ] or } を除去（JSON5 非互換への対処）
    text = re.sub(r",\s*([}\]])", r"\1", text)
    data = json.loads(text)
    return data.get("presets", [])


def _filter_ref_patches(presets: list[dict], name_fragments: list[str]) -> list[dict]:
    """名前部分一致（OR）でフィルタ。fragments が空の場合は全件返す。"""
    if not name_fragments:
        return presets
    return [p for p in presets
            if any(frag in p["name"] for frag in name_fragments)]


# ── バッチ推論用ヘルパー ─────────────────────────────────────────────────

def _prog_from_filename(fname: str) -> int | None:
    stem = Path(fname).stem
    if len(stem) >= 3 and stem[:3].isdigit():
        return int(stem[:3])
    return None


def load_wav_mono(path: Path) -> tuple[np.ndarray, int]:
    import wave as _wave
    with _wave.open(str(path)) as w:
        sr = w.getframerate()
        raw = w.readframes(w.getnframes())
        ch = w.getnchannels()
        sw = w.getsampwidth()
    if sw == 2:
        x = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif sw == 4:
        x = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648.0
    else:
        raise ValueError(f"未対応サンプル幅: {sw}")
    if ch == 2:
        x = x.reshape(-1, 2).mean(axis=1)
    return x, sr


def _make_bank_json(entries: list[dict], bank: int = 0) -> str:
    import json
    presets = [{"program": e["program"], "name": e["name"], "patch": e["patch"]}
               for e in entries]
    return json.dumps({"bank": bank, "presets": presets}, indent=2)


# ── main ──────────────────────────────────────────────────────────────────

def main() -> int:
    ap = argparse.ArgumentParser(
        description="38x6 A-by-S: 自己再構成テスト(3b+) / バッチWAV推論(--input-dir)")
    # ── 共通 ──
    ap.add_argument("--maxfevals", type=int, default=2400,
                    help="1ターゲットあたりの総eval予算（algo×波形×候補数で均等分配）")
    ap.add_argument("--restarts", type=int, default=1)
    ap.add_argument("--sigma0", type=float, default=0.25)
    ap.add_argument("--cma-seed", type=int, default=1)
    ap.add_argument("--sr", type=float, default=44100.0)
    ap.add_argument("--n-mels", type=int, default=64)
    ap.add_argument("--n-frames", type=int, default=64)
    ap.add_argument("--w-env", type=float, default=2.0)
    ap.add_argument("--w-centroid", type=float, default=4.0)
    ap.add_argument("--w-harmonic", type=float, default=0.0,
                    help="高調波損失の重み（既定0=無効）。"
                         "FM合成ではキャリアMULが音高を決めるため、"
                         "ターゲットの倍音強度を誤ってMUL値に写像し音痴を引き起こす恐れあり")
    ap.add_argument("--loss-mode", choices=["spectral", "descriptor"], default="descriptor",
                    help="損失モード: descriptor=知覚記述子一致(既定), spectral=mel/env/centroid再構成(従来)")
    ap.add_argument("--desc-n-fft", type=int, default=2048,
                    help="descriptor損失の高調波FFTサイズ（速度と精度のトレードオフ、既定2048）")
    ap.add_argument("--model", type=str, default=None)
    ap.add_argument("--no-model", action="store_true")
    ap.add_argument("--ref-patch", type=str, default=None,
                    help="参照 .38x6 ファイルパス。マッチしたパッチを CMA-ES 初期値候補に追加する")
    ap.add_argument("--ref-name", type=str, default=None,
                    help="参照パッチの音色名部分一致フィルタ（カンマ区切り複数可、OR 論理）。"
                         "省略時はバッチモードでプログラム番号が一致するパッチを自動選択、"
                         "テストモードでは全件使用")
    # ── バッチ推論モード ──
    ap.add_argument("--input-dir", type=str, default=None,
                    help="[バッチ] WAV ディレクトリ（000_Name.wav 形式）")
    ap.add_argument("--out", type=str, default=None,
                    help="[バッチ] 出力 .38x6 パス")
    ap.add_argument("--note", type=int, default=60)
    ap.add_argument("--on", type=float, default=2.5)
    ap.add_argument("--release", type=float, default=1.5)
    ap.add_argument("--pre-delay", type=float, default=0.3)
    ap.add_argument("--prog-range", type=str, default=None,
                    help="[バッチ] 処理するプログラム番号（例: 0-7,16,56-63）。カンマ・範囲混在可")
    ap.add_argument("--algorithms", type=str, default=None,
                    help="[バッチ] アルゴリズム固定（例: 4,6）。省略時はGM2テーブルから自動選択")
    ap.add_argument("--waveforms", type=str, default=None,
                    help="[バッチ] 試す波形ペア mod-car のカンマ区切り（例: 0-0,3-0,0-2）。"
                         "省略時はGM2テーブルから自動選択")
    ap.add_argument("--bank", type=int, default=0)
    # ── 自己再構成テストモード ──
    ap.add_argument("--n-targets", type=int, default=5)
    ap.add_argument("--seed", type=int, default=12345)
    ap.add_argument("--wav", type=int, default=5)
    ap.add_argument("--wav-dir", type=str, default=None)
    ap.add_argument("--freq", type=float, default=note_freq("C", 4))
    ap.add_argument("--test-on", type=float, default=0.6)
    ap.add_argument("--test-release", type=float, default=0.3)
    ap.add_argument("--test-algorithms", type=str, default=None,
                    help="[テスト] 試すアルゴリズム（例: 4,6）。省略時は 4 のみ")
    ap.add_argument("--test-waveforms", type=str, default=None,
                    help="[テスト] 試す波形ペア（例: 0-0,3-0）。省略時は 0-0 のみ")
    args = ap.parse_args()

    # ── ML モデル読み込み ──
    model, ckpt = None, None
    if not args.no_model:
        model_path = Path(args.model) if args.model else _DEFAULT_MODEL
        if model_path.exists():
            try:
                import torch
                from model import InverseNet
                ckpt_raw = torch.load(str(model_path), map_location="cpu", weights_only=False)
                m = InverseNet(out_dim=ckpt_raw["out_dim"],
                               n_mels=ckpt_raw["n_mels"],
                               n_frames=ckpt_raw["n_frames"])
                m.load_state_dict(ckpt_raw["model_state"])
                m.eval()
                model, ckpt = m, ckpt_raw
                print(f"MLモデル: {model_path.name} (warm start 有効)")
            except Exception as e:
                print(f"WARNING: MLモデル読み込み失敗 ({e})", file=sys.stderr)
        else:
            print(f"  (MLモデル未検出: {model_path.name} → warm start なし)")

    # ── 参照パッチ読み込み ──
    all_ref_presets: list[dict] = []
    ref_name_fragments: list[str] = []
    if args.ref_patch:
        ref_path = Path(args.ref_patch)
        if ref_path.exists():
            all_ref_presets = load_ref_patches(ref_path)
            ref_name_fragments = (
                [f.strip() for f in args.ref_name.split(",") if f.strip()]
                if args.ref_name else []
            )
            name_desc = f"名前フィルタ={ref_name_fragments}" if ref_name_fragments else "プログラム番号自動照合"
            print(f"参照パッチ: {ref_path.name} ({len(all_ref_presets)} 件) [{name_desc}]")
        else:
            print(f"WARNING: 参照パッチ未検出: {ref_path}", file=sys.stderr)

    # ── バッチ推論モード ──────────────────────────────────────────────────
    if args.input_dir:
        wav_dir = Path(args.input_dir)
        if not wav_dir.is_dir():
            print(f"ERROR: {wav_dir} が見つかりません", file=sys.stderr)
            return 1

        prog_set = _parse_prog_set(args.prog_range) if args.prog_range else None
        fixed_algos = ([int(a) for a in args.algorithms.split(",")]
                       if args.algorithms else None)
        fixed_wf_pairs = _parse_wf_pairs(args.waveforms) if args.waveforms else None

        wavs = [p for p in sorted(wav_dir.glob("*.wav"))
                if (prog_set is None or _prog_from_filename(p.name) in prog_set)
                and _prog_from_filename(p.name) is not None]

        if not wavs:
            print("ERROR: 条件に一致する WAV が見つかりません", file=sys.stderr)
            return 1

        default_out = wav_dir.parent / "bank0_abys.38x6"
        out_path = Path(args.out) if args.out else default_out
        freq = _midi_to_freq(args.note)

        print(f"バッチ A-by-S: {wav_dir} ({len(wavs)} 件) → {out_path}")
        print(f"  note={args.note}(freq={freq:.2f}Hz)  on={args.on}s  "
              f"release={args.release}s  pre_delay={args.pre_delay}s")
        src_algo = f"固定: {fixed_algos}" if fixed_algos else "GM2テーブルから自動選択"
        src_wf   = f"固定: {fixed_wf_pairs}" if fixed_wf_pairs else "GM2テーブルから自動選択"
        print(f"  アルゴリズム: {src_algo}")
        print(f"  波形ペア    : {src_wf}")
        print(f"  maxfevals={args.maxfevals}  restarts={args.restarts}")

        entries: list[dict] = []
        failed: list[str] = []
        t0 = time.time()

        for wav_path in wavs:
            prog = _prog_from_filename(wav_path.name)
            name = wav_path.stem[4:] if len(wav_path.stem) > 4 else wav_path.stem

            algos   = fixed_algos    or GM2_PROG_ALGORITHMS.get(prog, [ps.FIXED_ALGORITHM])
            wf_prs  = fixed_wf_pairs or GM2_PROG_WAVEFORM_PAIRS.get(prog, [_WF_SINE])
            fr      = GM2_PROG_FIELD_RANGES.get(prog)
            if all_ref_presets:
                if ref_name_fragments:
                    ref_patches = _filter_ref_patches(all_ref_presets, ref_name_fragments)
                else:
                    ref_patches = [p for p in all_ref_presets if p["program"] == prog]
            else:
                ref_patches = []
            # ref パッチのアルゴリズムを試行リストに自動追加
            # （--algorithms 未指定 かつ プログラム番号照合時のみ。--ref-name 時は
            #   複数カテゴリのref が混在しアルゴリズムが爆発するため追加しない）
            if ref_patches and not fixed_algos and not ref_name_fragments:
                seen = set(algos)
                for p in ref_patches:
                    a = int(p["patch"]["channel"]["algorithm"])
                    if a not in seen:
                        algos = algos + [a]
                        seen.add(a)
            ref_str = f" ref={[p['name'] for p in ref_patches]}" if ref_patches else ""
            print(f"  [{prog:3d}] {name:<28} alg={algos} wf={wf_prs}{ref_str} ", end="", flush=True)

            try:
                samples, src_sr = load_wav_mono(wav_path)
                if src_sr != int(args.sr):
                    n_new = int(len(samples) * args.sr / src_sr)
                    samples = np.interp(
                        np.linspace(0, len(samples) - 1, n_new),
                        np.arange(len(samples)), samples,
                    ).astype(np.float32)
                trim = int(args.pre_delay * args.sr)
                samples = samples[trim:] if trim < len(samples) else samples
                if ft.is_silent(samples):
                    print("無音スキップ")
                    continue

                target_feats = feats_of(samples, args.sr, args.n_mels, args.n_frames)
                target_desc = (ft.perceptual_descriptors(
                                   np.asarray(samples, dtype=np.float64), freq, args.sr)
                               if args.loss_mode == "descriptor" else None)
                t1 = time.time()
                xbest, best_d, evals, winner, combo_summary = fit_one(
                    target_feats, samples,
                    freq, args.on, args.release, args.sr,
                    args.n_mels, args.n_frames,
                    args.w_env, args.w_centroid,
                    args.sigma0, args.maxfevals, args.restarts, args.cma_seed,
                    algorithms=algos, wf_pairs=wf_prs, field_ranges=fr,
                    model=model, ckpt=ckpt,
                    w_harmonic=args.w_harmonic,
                    ref_patches=ref_patches or None,
                    loss_mode=args.loss_mode,
                    target_desc=target_desc,
                    desc_n_fft=args.desc_n_fft,
                )
                # winner から algo/wf を取り出してパッチを生成
                tag = winner.split("/")[0]          # "alg4_w30"
                best_algo = int(tag.split("_")[0].replace("alg", ""))
                wf_tag = tag.split("_")[1]          # "w30"
                b_mod, b_car = int(wf_tag[1]), int(wf_tag[2])
                best_wf = ps.expand_waveforms(b_mod, b_car, best_algo)
                patch = ps.vector_to_patch(xbest, best_algo, best_wf, fr)
                entries.append({"program": prog, "name": name, "patch": patch})

                combo_str = "  ".join(f"{k}={v:.3f}" for k, v in combo_summary)
                print(f"loss={best_d:.4f} winner={winner} evals={evals} "
                      f"{time.time()-t1:.1f}s")
                print(f"      {combo_str}")

            except Exception as e:
                import traceback; traceback.print_exc()
                print(f"FAIL: {e}")
                failed.append(wav_path.name)

        if not entries:
            print("ERROR: 有効なエントリーが0件です", file=sys.stderr)
            return 1

        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(_make_bank_json(entries, bank=args.bank), encoding="utf-8")
        print(f"\n{len(entries)} パッチ → {out_path}  (合計 {time.time()-t0:.1f}s)")
        if failed:
            print(f"失敗: {failed}")
        return 0 if not failed else 1

    # ── 自己再構成テストモード ────────────────────────────────────────────
    test_algos = ([int(a) for a in args.test_algorithms.split(",")]
                  if args.test_algorithms else [ps.FIXED_ALGORITHM])
    test_wf = _parse_wf_pairs(args.test_waveforms) if args.test_waveforms else [_WF_SINE]
    test_ref = _filter_ref_patches(all_ref_presets, ref_name_fragments) if all_ref_presets else []
    # ref パッチのアルゴリズムを試行リストに自動追加
    # （--test-algorithms 未指定 かつ プログラム番号照合時のみ。--ref-name 時は追加しない）
    if test_ref and not args.test_algorithms and not ref_name_fragments:
        seen = set(test_algos)
        for p in test_ref:
            a = int(p["patch"]["channel"]["algorithm"])
            if a not in seen:
                test_algos = test_algos + [a]
                seen.add(a)

    base = Path(__file__).resolve().parent.parent / "private"
    wav_out_dir = Path(args.wav_dir) if args.wav_dir else base / "abys_wav"
    if args.wav > 0:
        wav_out_dir.mkdir(parents=True, exist_ok=True)

    rng = np.random.default_rng(args.seed)
    base_vec = np.full(ps.DIM, 0.5)
    base_feats = feats_of(
        render(base_vec, args.freq, args.test_on, args.test_release, args.sr),
        args.sr, args.n_mels, args.n_frames)

    init_dists, best_dists, param_mse = [], [], []
    best_terms = []
    harm_dists: list[float] = []
    done = 0
    tries = 0
    t0 = time.time()

    while done < args.n_targets and tries < args.n_targets * 8:
        tries += 1
        true_vec = rng.random(ps.DIM)
        target = render(true_vec, args.freq, args.test_on, args.test_release, args.sr)
        if ft.is_silent(target):
            continue
        target_feats = feats_of(target, args.sr, args.n_mels, args.n_frames)
        target_desc = (ft.perceptual_descriptors(
                           np.asarray(target, dtype=np.float64), args.freq, args.sr)
                       if args.loss_mode == "descriptor" else None)
        if args.loss_mode == "descriptor" and target_desc is not None:
            base_desc_tmp = ft.perceptual_descriptors(
                np.asarray(render(base_vec, args.freq, args.test_on, args.test_release, args.sr),
                           dtype=np.float64), args.freq, args.sr)
            init_d, _ = ft.descriptor_distance(target_desc, base_desc_tmp)
        else:
            init_d, _ = ft.abys_distance(target_feats, base_feats, args.w_env, args.w_centroid)

        xbest, best_d, evals, winner, combo_summary = fit_one(
            target_feats, target,
            args.freq, args.test_on, args.test_release, args.sr,
            args.n_mels, args.n_frames,
            args.w_env, args.w_centroid,
            args.sigma0, args.maxfevals, args.restarts, args.cma_seed,
            algorithms=test_algos, wf_pairs=test_wf, model=model, ckpt=ckpt,
            w_harmonic=args.w_harmonic,
            ref_patches=test_ref or None,
            loss_mode=args.loss_mode,
            target_desc=target_desc,
            desc_n_fft=args.desc_n_fft,
        )

        tag = winner.split("/")[0]
        best_algo = int(tag.split("_")[0].replace("alg", ""))
        wf_tag = tag.split("_")[1]
        b_mod, b_car = int(wf_tag[1]), int(wf_tag[2])
        best_wf = ps.expand_waveforms(b_mod, b_car, best_algo)
        recon = render(xbest, args.freq, args.test_on, args.test_release, args.sr,
                       algorithm=best_algo, waveforms=best_wf)
        if args.loss_mode == "descriptor" and target_desc is not None:
            recon_desc = ft.perceptual_descriptors(
                np.asarray(recon, dtype=np.float64), args.freq, args.sr)
            _, desc_terms = ft.descriptor_distance(target_desc, recon_desc)
            terms = desc_terms  # type: ignore[assignment]
        else:
            _, terms = ft.abys_distance(
                target_feats,
                feats_of(recon, args.sr, args.n_mels, args.n_frames),
                args.w_env, args.w_centroid)

        d_harm = 0.0
        if args.w_harmonic > 0:
            t_harms = ft.harmonic_features(
                np.asarray(target, dtype=np.float64), args.freq, args.sr)
            r_harms = ft.harmonic_features(
                np.asarray(recon, dtype=np.float64), args.freq, args.sr)
            d_harm = ft.harmonic_distance(t_harms, r_harms)
        harm_dists.append(d_harm)

        init_dists.append(init_d)
        best_dists.append(best_d)
        best_terms.append(terms)
        param_mse.append(float(np.mean((xbest - true_vec) ** 2)))

        if done < args.wav:
            write_wav(wav_out_dir / f"{done:02d}_target.wav", target, args.sr)
            write_wav(wav_out_dir / f"{done:02d}_recon.wav", recon, args.sr)

        impr = (1 - best_d / init_d) * 100
        print(f"  target {done}: init={init_d:.4f} -> best={best_d:.4f} "
              f"(改善率={impr:.1f}%, evals={evals}, winner={winner})")
        if args.loss_mode == "descriptor" and isinstance(terms, dict):
            term_str = "  ".join(f"{k[:2]}={v:.3f}" for k, v in terms.items())
            print(f"    [{term_str}]")
        else:
            d_mel, d_env, d_cen = terms  # type: ignore[misc]
            harm_str = f" harm={d_harm:.4f}" if args.w_harmonic > 0 else ""
            print(f"    [mel={d_mel:.4f} env={d_env:.4f} cen={d_cen:.4f}{harm_str}]")
        combo_str = "  ".join(f"{k}={v:.4f}" for k, v in combo_summary)
        print(f"    {combo_str}")
        done += 1

    if not best_dists:
        print("ERROR: 有効ターゲットが0件", file=sys.stderr)
        return 1

    dt = time.time() - t0
    init_m = float(np.mean(init_dists))
    best_m = float(np.mean(best_dists))
    terms_m = np.mean(best_terms, axis=0)
    w_harm_str = f" w_harmonic={args.w_harmonic}" if args.w_harmonic > 0 else ""
    print(f"\nA-by-S 自己再構成 {done}件 "
          f"(所要={dt:.1f}s, w_env={args.w_env} w_centroid={args.w_centroid}{w_harm_str})")
    print(f"  合算距離     初期0.5固定={init_m:.4f}  A-by-S後={best_m:.4f}"
          f"  改善率={(1 - best_m / init_m) * 100:.1f}%")
    harm_avg_str = (f"  harmonic={float(np.mean(harm_dists)):.4f}"
                    if args.w_harmonic > 0 else "")
    print(f"  項別(A-by-S後) mel={terms_m[0]:.4f}  env={terms_m[1]:.4f}"
          f"  centroid={terms_m[2]:.4f}{harm_avg_str}")
    print(f"  paramMSE     復元={float(np.mean(param_mse)):.4f}")
    if args.wav > 0:
        print(f"  WAV書き出し: {wav_out_dir}（{min(done, args.wav)}ペア）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
