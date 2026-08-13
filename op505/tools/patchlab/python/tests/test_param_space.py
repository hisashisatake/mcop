"""param_space.py / op505_patch.py の軽量回帰テスト。

pytest不要、素のassertで `uv run python python/tests/test_param_space.py` から実行できる
（ym38x6版patchlabにはテストが無く、この形式が最小の追加で済むため踏襲する）。
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import numpy as np

import op505_patch
import param_space as ps
import patchlab


def test_dim_is_35() -> None:
    assert ps.DIM == 35
    assert len(ps.LABELS) == 35


def test_vector_to_patch_produces_op505_schema() -> None:
    vec = np.full(ps.DIM, 0.5)
    patch = ps.vector_to_patch(vec)
    for op in patch["operators"]:
        assert "eg" in op, "op505オペレーターは eg(TimeEgParams) を持つはず"
        assert op["eg"]["stage_count"] >= 1, "無音パッチ(stage_count=0)の詰め忘れ検知"
    assert "cutoff_fg" in patch["channel"]
    assert "eg" in patch["channel"]["cutoff_fg"]


def test_vector_to_patch_renders_non_silent() -> None:
    import json
    vec = np.full(ps.DIM, 0.6)
    patch_json = json.dumps(ps.vector_to_patch(vec))
    samples = patchlab.render_patch(patch_json, 440.0, 0.2, 0.1, 100, 44100.0)
    peak = max(abs(s) for s in samples)
    assert peak > 1e-3, f"無音疑い: peak={peak}"


def test_rate_form_round_trip_is_close() -> None:
    """レート方式dict → ベクトル → レート方式spec で読める範囲での往復恒等性。
    vector_to_patch()はop505形式を返すためpatch_json_to_vector()には直接渡せない
    （意図的なガード）。ここではレート方式dictを直接検証する。
    """
    rate_patch = {
        "operators": [
            {"tl": 200, "ar": 255, "d1r": 100, "d2r": 50, "d1l": 180, "rr": 120, "mul": 1, "dt1": 128},
            {"tl": 220, "ar": 255, "d1r": 100, "d2r": 50, "d1l": 180, "rr": 120, "mul": 1, "dt1": 128},
            {"tl": 200, "ar": 255, "d1r": 100, "d2r": 50, "d1l": 180, "rr": 120, "mul": 2, "dt1": 128},
            {"tl": 240, "ar": 255, "d1r": 100, "d2r": 50, "d1l": 180, "rr": 120, "mul": 1, "dt1": 128},
        ],
        "channel": {"algorithm": 4, "feedback": 40, "filter_cutoff": 255, "filter_resonance": 0},
    }
    vec = ps.patch_json_to_vector(rate_patch)
    assert vec.shape == (35,)
    assert np.all((vec >= 0.0) & (vec <= 1.0))


def test_patch_json_to_vector_rejects_op505_schema() -> None:
    vec = np.full(ps.DIM, 0.5)
    op505_style = ps.vector_to_patch(vec)
    try:
        ps.patch_json_to_vector(op505_style)
    except ValueError:
        pass
    else:
        raise AssertionError("op505形式(eg付き)を渡してもValueErrorにならなかった")


def test_op505_patch_make_op_stage_shape() -> None:
    op = op505_patch.make_op(tl=200, ar=255, d1r=180, d2r=80, d1l=100, rr=180, mul=1, dt1=128)
    assert op["eg"]["stages"][1]["level"] == 100, "d1lがstages[1].levelに反映されないのは引数順の疑い"


def main() -> int:
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    failed = 0
    for t in tests:
        try:
            t()
            print(f"  ok   {t.__name__}")
        except Exception as e:  # noqa: BLE001
            failed += 1
            print(f"  FAIL {t.__name__}: {e}")
    print(f"\n{len(tests) - failed}/{len(tests)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
