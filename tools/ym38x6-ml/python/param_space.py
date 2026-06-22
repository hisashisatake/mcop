"""フェーズ6 MVP: 38x6パッチの制限サブセット ↔ 正規化パラメーターベクトル(35次元)の双方向変換。

回帰のみで解けるよう、離散(algorithm/waveform)・双極(op_fine_tune)・LFO/Filter EG等は固定し、
連続パラメーターだけを [0,1] で振る。後段で段階的に次元を増やす（waveform→algorithm→…）。

パッチJSONは `Ym38x6Patch` のserde表現: {"operators": [op0..op3], "channel": {...}}。
"""

from __future__ import annotations

import json

import numpy as np

SPEC_VERSION = "mvp-2"

# 固定するアルゴリズム: (O1→O2)+(O3→O4)。FM変調を含む代表的な結線。
FIXED_ALGORITHM = 4

# algorithm=4 のキャリア（出力に直接合算されるop）= O2/O4。モジュレーターは O1/O3。
CARRIERS_ALGO4 = (1, 3)
# キャリアTLの下限。0だとキャリアが小さく出て無音が頻発し採用率が落ちるため、
# 高め（可聴域）にバイアスする。モジュレーターTLは0〜255のまま（FM変調量＝音色を広く振る）。
CARRIER_TL_MIN = 160

# 振る連続パラメーター: オペレーター側 (field, max_value)
_OP_FIELDS = [
    ("tl", 255),
    ("ar", 255),
    ("d1r", 255),
    ("d2r", 255),
    ("d1l", 255),
    ("rr", 255),
    ("mul", 15),
    ("dt1", 255),
]
# 振る連続パラメーター: チャンネル側 (field, max_value)
_CH_FIELDS = [
    ("feedback", 255),
    ("filter_cutoff", 255),
    ("filter_resonance", 255),
]

# ベクトルの並び(35): op0の8項目 → op1の8 → op2の8 → op3の8 → channelの3項目。
# 各要素 = (label, target, field, vmin, vmax)。target は 0..3(オペ index) または "ch"。
# op別レンジ対応: キャリアのTLのみ下限を上げ、他は 0〜max。


def _op_field_range(op_i: int, field: str, default_max: int) -> tuple[int, int]:
    if field == "tl" and op_i in CARRIERS_ALGO4:
        return (CARRIER_TL_MIN, 255)
    return (0, default_max)


PARAM_SPEC = []
for _op_i in range(4):
    for _name, _mx in _OP_FIELDS:
        _vmin, _vmax = _op_field_range(_op_i, _name, _mx)
        PARAM_SPEC.append((f"op{_op_i}.{_name}", _op_i, _name, _vmin, _vmax))
for _name, _mx in _CH_FIELDS:
    PARAM_SPEC.append((f"ch.{_name}", "ch", _name, 0, _mx))

DIM = len(PARAM_SPEC)  # 35

LABELS = [spec[0] for spec in PARAM_SPEC]


def _fixed_operator() -> dict:
    """振らないオペレーターパラメーターの既定値（全フィールドを埋める）。"""
    return dict(
        tl=0, ar=0, d1r=0, d2r=0, d1l=0, rr=0, mul=0, dt1=128,
        ksr=0, am_enable=False, velocity_sensitivity=0,
        waveform=0, op_fine_tune=128,
    )


def _fixed_channel() -> dict:
    """振らないチャンネルパラメーターの既定値（全フィールドを埋める）。"""
    return dict(
        algorithm=FIXED_ALGORITHM, feedback=0,
        tone_lfo_freq=0, tone_lfo_pmd=0, tone_lfo_amd=0, tone_lfo_delay=0,
        pms=0, ams=0,
        filter_cutoff=255, filter_resonance=0, filter_type=0,
        filter_self_oscillation=False,
        filter_eg_attack=0, filter_eg_decay=0, filter_eg_sustain=0,
        filter_eg_release=0, filter_eg_depth=0,
    )


def random_vectors(n: int, rng: np.random.Generator) -> np.ndarray:
    """[n, DIM] の一様乱数ベクトル([0,1])を返す。"""
    return rng.random((n, DIM), dtype=np.float64)


def vector_to_patch(vec: np.ndarray) -> dict:
    """正規化ベクトル(DIM,) → Ym38x6Patch 相当のdict。

    各次元は [0,1] とみなし value = round(v * max) で整数化（範囲クランプ）。
    """
    vec = np.asarray(vec, dtype=np.float64).reshape(-1)
    if vec.shape[0] != DIM:
        raise ValueError(f"expected {DIM}-dim vector, got {vec.shape[0]}")
    ops = [_fixed_operator() for _ in range(4)]
    ch = _fixed_channel()
    for i, (_label, target, field, vmin, vmax) in enumerate(PARAM_SPEC):
        val = int(round(vmin + float(vec[i]) * (vmax - vmin)))
        val = max(vmin, min(vmax, val))
        if target == "ch":
            ch[field] = val
        else:
            ops[target][field] = val
    return {"operators": ops, "channel": ch}


def vector_to_patch_json(vec: np.ndarray) -> str:
    """正規化ベクトル(DIM,) → パッチJSON文字列。"""
    return json.dumps(vector_to_patch(vec))
