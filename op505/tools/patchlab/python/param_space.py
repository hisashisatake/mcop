"""フェーズ6 MVP: op505パッチの制限サブセット ↔ 正規化パラメーターベクトル(35次元)の双方向変換。

回帰のみで解けるよう、離散(algorithm/waveform)・双極(op_fine_tune)・LFO/Filter EG等は固定し、
連続パラメーターだけを [0,1] で振る。後段で段階的に次元を増やす（waveform→algorithm→…）。

35次元はレート方式5段EG(ar/d1r/d2r/d1l/rr)という「探索空間の設計」であり、op505のパッチ形式
（TimeEgParams）そのものではない。TimeEgを素で振ると次元が爆発し大半が無効領域になるため、
`vector_to_patch()`の出口で`op505_patch.make_op/make_channel`（Rust側`eg_convert::
convert_eg_shape`のラッパー）を経由してTimeEgParamsへ変換する。

パッチdictはop505パッチdict（`op505_patch.make_op/make_channel`が返す形、`Op505Patch`の
serde表現と同形）。レート方式dict（このモジュール内部でのみ使う、`ar/d1r/d2r/d1l/rr`を
トップレベルに持つ中間表現）とは別物なので混同しないこと。
"""

from __future__ import annotations

import json

import numpy as np

import op505_patch

SPEC_VERSION = "op505-1"

# デフォルトのアルゴリズム（後方互換）
FIXED_ALGORITHM = 4

# アルゴリズム毎のキャリア op インデックス（algorithm.rs と同期）
# OPN/OPM 互換。フィードバック対象は常に op0。
ALGO_CARRIERS: dict[int, tuple[int, ...]] = {
    0: (3,),           # O1→O2→O3→O4（全直列）
    1: (3,),           # (O1+O2)→O3→O4
    2: (3,),           # (O1+(O2→O3))→O4
    3: (3,),           # ((O1→O2)+O3)→O4
    4: (1, 3),         # (O1→O2)+(O3→O4) ← デフォルト
    5: (1, 2, 3),      # O1→(O2+O3+O4)
    6: (1, 2, 3),      # (O1→O2)+O3+O4
    7: (0, 1, 2, 3),   # O1+O2+O3+O4（全並列）
}

# キャリアTLの下限（0だと無音頻発のため可聴域にバイアス）
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
# 振る連続パラメーター: チャンネル側 (field, min_value, max_value)
# filter_cutoff は 0 だと無音に近くなるため下限を 160 に設定（-6dBあたり）。
# これにより CMA-ES がフィルター半開きの局所解に落ちにくくなる。
_CH_FIELDS = [
    ("feedback",        0, 255),
    ("filter_cutoff", 160, 255),
    ("filter_resonance", 0, 255),
]


def build_param_spec(algorithm: int = FIXED_ALGORITHM,
                     field_ranges: dict[str, tuple[int, int]] | None = None) -> list:
    """指定アルゴリズム + パラメーター範囲制約で PARAM_SPEC を生成する。

    各要素は (label, target, field, vmin, vmax)。DIM は常に 35。

    field_ranges: フィールド名 → (vmin, vmax) の上書き辞書。全 op に同じ制約を適用。
                  例: {"ar": (200, 255), "d1l": (0, 80)} でピアノ系の envelope を制約。
                  キャリア TL は algorithm に基づく CARRIER_TL_MIN が優先される。
    """
    carriers = ALGO_CARRIERS[algorithm]
    overrides = field_ranges or {}
    spec = []
    for op_i in range(4):
        for name, mx in _OP_FIELDS:
            if name == "tl" and op_i in carriers:
                vmin, vmax = CARRIER_TL_MIN, 255
            elif name in overrides:
                vmin, vmax = overrides[name]
            else:
                vmin, vmax = 0, mx
            spec.append((f"op{op_i}.{name}", op_i, name, vmin, vmax))
    for name, vmin, vmax in _CH_FIELDS:
        spec.append((f"ch.{name}", "ch", name, vmin, vmax))
    return spec


# デフォルト(Algorithm=4)の定数（後方互換）
PARAM_SPEC = build_param_spec(FIXED_ALGORITHM)
DIM = len(PARAM_SPEC)   # 35
LABELS = [spec[0] for spec in PARAM_SPEC]


def _fixed_operator() -> dict:
    """振らないオペレーターパラメーターの既定値（レート方式dictのひな型、全フィールドを
    埋める）。この時点ではまだop505形式ではない（`vector_to_patch`の最後で
    `op505_patch.make_op`へ通す）。"""
    return dict(
        tl=0, ar=0, d1r=0, d2r=0, d1l=0, rr=0, mul=0, dt1=128,
        ksr=0, am_enable=False, velocity_sensitivity=0,
        waveform=0, op_fine_tune=128,
    )


def _fixed_channel(algorithm: int = FIXED_ALGORITHM) -> dict:
    """振らないチャンネルパラメーターの既定値（レート方式dictのひな型、全フィールドを
    埋める）。この時点ではまだop505形式ではない（`vector_to_patch`の最後で
    `op505_patch.make_channel`へ通す）。"""
    return dict(
        algorithm=algorithm, feedback=0,
        tone_lfo_freq=0, tone_lfo_pmd=0, tone_lfo_amd=0, tone_lfo_delay=0,
        pms=0, ams=0,
        filter_cutoff=255, filter_resonance=0, filter_type=0,
        filter_self_oscillation=False,
        filter_eg_ar=0, filter_eg_d1r=0, filter_eg_d1l=0,
        filter_eg_rr=0, filter_eg_depth=0,
    )


def expand_waveforms(mod_wf: int, car_wf: int, algorithm: int) -> tuple[int, ...]:
    """(モジュレーター波形, キャリア波形) ペアをアルゴリズムのキャリア定義に従い
    4-tuple (op0, op1, op2, op3) に展開する。

    波形インデックス: W1=0(sine), W2=1(木管), W3=2(弦), W4=3(撥弦mod),
                      W5=4(ギター/リードmod), W6=5(リード系car), W7=6(高域++), W8=7(高域+)
    """
    carriers = ALGO_CARRIERS[algorithm]
    return tuple(car_wf if i in carriers else mod_wf for i in range(4))


def random_vectors(n: int, rng: np.random.Generator) -> np.ndarray:
    """[n, DIM] の一様乱数ベクトル([0,1])を返す。"""
    return rng.random((n, DIM), dtype=np.float64)


def vector_to_patch(vec: np.ndarray, algorithm: int = FIXED_ALGORITHM,
                    waveforms: tuple[int, ...] = (0, 0, 0, 0),
                    field_ranges: dict[str, tuple[int, int]] | None = None) -> dict:
    """正規化ベクトル(DIM,) → op505パッチ相当のdict。

    algorithm   : キャリアTL範囲とpatch内のalgorithmフィールドを決定する。
    waveforms   : 各opの波形インデックス (op0, op1, op2, op3)。
    field_ranges: envelope 等のパラメーター範囲制約。build_param_spec() と同じ辞書。
    """
    spec = build_param_spec(algorithm, field_ranges)
    vec = np.asarray(vec, dtype=np.float64).reshape(-1)
    if vec.shape[0] != DIM:
        raise ValueError(f"expected {DIM}-dim vector, got {vec.shape[0]}")
    raw_ops = [_fixed_operator() for _ in range(4)]
    raw_ch = _fixed_channel(algorithm)
    for i, (_label, target, field, vmin, vmax) in enumerate(spec):
        val = int(round(vmin + float(vec[i]) * (vmax - vmin)))
        val = max(vmin, min(vmax, val))
        if target == "ch":
            raw_ch[field] = val
        else:
            raw_ops[target][field] = val
    for op_i, wf in enumerate(waveforms[:4]):
        raw_ops[op_i]["waveform"] = int(wf)
    # レート方式dict(raw_ops/raw_ch) → op505パッチdictへ、この出口で一度だけ変換する
    # （convert_eg_shapeはRust側の唯一の実装、ここではPython側に複製しない）。
    ops = [op505_patch.make_op(**raw) for raw in raw_ops]
    ch = op505_patch.make_channel(**raw_ch)
    return {"operators": ops, "channel": ch}


def vector_to_patch_json(vec: np.ndarray, algorithm: int = FIXED_ALGORITHM,
                         waveforms: tuple[int, ...] = (0, 0, 0, 0),
                         field_ranges: dict[str, tuple[int, int]] | None = None) -> str:
    """正規化ベクトル(DIM,) → パッチJSON文字列。"""
    return json.dumps(vector_to_patch(vec, algorithm, waveforms, field_ranges))


def patch_json_to_vector(
    patch: dict,
    algorithm: int | None = None,
    field_ranges: dict[str, tuple[int, int]] | None = None,
) -> np.ndarray:
    """レート方式パッチdict（`ar/d1r/d2r/d1l/rr`をトップレベルに持つ旧ym38x6スキーマ、
    "ref.38x6"参照バンク由来）→ [0,1] 正規化ベクトル(DIM,)。`vector_to_patch`の逆変換
    ではあるが、**op505パッチdict（`eg`にネストしたTimeEgParams形式）は受け付けない**
    （TimeEg→レート方式の逆変換は多対一のため数学的に不可能）。過去の`.38x6`音色資産を
    CMA-ES探索の初期値候補として使うための入口として残している（abys.pyの`--ref-patch`）。

    algorithm が None の場合は patch["channel"]["algorithm"] を使う。
    waveform / ksr / am_enable 等の非連続フィールドはベクトルに含まれないため無視する。
    """
    if algorithm is None:
        algorithm = int(patch["channel"]["algorithm"])
    ops = patch["operators"]
    if any("eg" in op for op in ops):
        raise ValueError(
            "patch_json_to_vector()はレート方式dict専用です。op505パッチ(eg付き)が渡されました。"
        )
    spec = build_param_spec(algorithm, field_ranges)
    ch = patch["channel"]
    vec = np.zeros(DIM, dtype=np.float64)
    for i, (_label, target, field, vmin, vmax) in enumerate(spec):
        if target == "ch":
            raw = int(ch.get(field, 0))
        else:
            raw = int(ops[target].get(field, 0))
        span = vmax - vmin
        vec[i] = float(np.clip((raw - vmin) / span if span > 0 else 0.0, 0.0, 1.0))
    return vec
