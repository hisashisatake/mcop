"""旧ym38x6スキーマのノブ表現（オペレーター: ar/d1r/d2r/d1l/rr、チャンネル: tone_lfo_*/
filter_eg_*）からop505パッチdictへの唯一の変換入口。

EG変換の実装は持たない（`patchlab.rate_eg_to_time_eg` = Rust側`op505-core::eg_convert::
convert_eg_shape`のラッパー）。op505ネイティブの既定値も持たない（`patchlab.default_patch_json()`
由来）。これにより、`Op505OperatorParams`/`Op505ChannelParams`にフィールドが増減しても
このモジュール（と`patchlab`拡張モジュール自体）を直せば済み、呼び出し側
（piano/brass/organ_template・probe・lookup等）のノブ名・既定値は一切変えずに済む。
"""

from __future__ import annotations

import functools
import json

import patchlab


@functools.lru_cache(maxsize=1)
def _default_patch() -> dict:
    return json.loads(patchlab.default_patch_json())


def default_operator() -> dict:
    """op505オペレーターの既定値dict（`Op505OperatorParams::default()`）。"""
    return dict(_default_patch()["operators"][0])


def default_channel() -> dict:
    """op505チャンネルの既定値dict（`Op505ChannelParams::default()`。pitch_fg/gain_fg/
    texture_lfoは変調なしの中立値が既に入っている）。"""
    return dict(_default_patch()["channel"])


def pitch_fg_from_chip_lfo(pms: int, pmd: int, freq: int, delay: int = 0) -> dict:
    """CHIP LFOのピッチ変調パラメーター(pms/pmd/freq/delay)と同じビブラートを、
    Pitch FGの2段三角ループとして返す（`patchlab.chip_lfo_pitch_to_pitch_fg_json`の
    ラッパー、`op505-core::chip_lfo_pitch_to_pitch_fg`の変換式そのもの）。

    CHIP LFO完全退役に向け、手動設計テンプレートがCHIP LFOのpms/chip_lfo_pmdへ書き込む
    代わりにこちらを使う。`pms=0`または`pmd=0`のときは無変調の既定値を返す。
    """
    return json.loads(patchlab.chip_lfo_pitch_to_pitch_fg_json(pms, pmd, freq, delay))


@functools.lru_cache(maxsize=8192)
def _time_eg_json(ar: int, d1r: int, d1l: int, d2r: int, rr: int) -> str:
    # CMA-ES等の大量呼び出しでのFFIコストを吸収するためlru_cacheを噛ませる
    # （レート方式ノブの組み合わせは離散値なのでキャッシュヒット率が高い）。
    eg_json, _warnings = patchlab.rate_eg_to_time_eg(ar, d1r, d1l, d2r, rr)
    return eg_json


def make_op(*, tl: int, ar: int, d1r: int, d2r: int, d1l: int, rr: int, mul: int, dt1: int = 128,
            **extra) -> dict:
    """レート方式5段EG(ar/d1r/d2r/d1l/rr)からop505オペレーターdictを作る。

    `extra`にはksr/waveform/am_enable/velocity_sensitivity/op_fine_tune等、
    `Op505OperatorParams`の残りフィールドを渡せる（既定値は`default_operator()`由来）。
    """
    op = default_operator()
    op.update(tl=tl, mul=mul, dt1=dt1, eg=json.loads(_time_eg_json(ar, d1r, d1l, d2r, rr)))
    op.update(extra)
    return op


def make_channel(*, algorithm: int, feedback: int = 0,
                  tone_lfo_freq: int = 0, tone_lfo_pmd: int = 0,
                  tone_lfo_amd: int = 0, tone_lfo_delay: int = 0,
                  pms: int = 0, ams: int = 0,
                  filter_cutoff: int = 255, filter_resonance: int = 0, filter_type: int = 0,
                  filter_self_oscillation: bool = False,
                  filter_eg_ar: int = 0, filter_eg_d1r: int = 0, filter_eg_d1l: int = 0,
                  filter_eg_rr: int = 0, filter_eg_depth: int = 0,
                  **extra) -> dict:
    """旧ym38x6スキーマのチャンネルノブ名（`tone_lfo_*`/`filter_eg_*`）を受け、
    op505チャンネルdictへ変換する（呼び出し側テンプレートのコードを無改変で保つための
    互換レイヤー）。

    - `tone_lfo_*` → `chip_lfo_*`（直接リネームのみ、値は同じ意味）。
    - `filter_eg_*`（unipolar Filter EG、`ar/d1r/d1l/rr` + `depth`0〜255）→ op505の
      `cutoff_fg`（**レベルがバイポーラ**中心128・Depthは符号なしの振れ幅倍率）へ変換する。
      `_time_eg_json`が返すレベルは0〜1の大きさ（unipolar）なので、常に「開く」方向（正側）へ
      畳み込む（`op505_core::bipolar_fg_levels_from_magnitude(eg, positive=true)`と同じ式）。
      Depthはそのまま振れ幅として使う（旧方式の「中心128からのオフセット」ではない）。
    - `pitch_fg`/`gain_fg`/`texture_lfo`は`default_channel()`の中立値のまま
      （旧スキーマにはこれらに相当するノブが無かったため）。
    """
    ch = default_channel()
    ch.update(
        algorithm=algorithm,
        feedback=feedback,
        chip_lfo_freq=tone_lfo_freq,
        chip_lfo_pmd=tone_lfo_pmd,
        chip_lfo_amd=tone_lfo_amd,
        chip_lfo_delay=tone_lfo_delay,
        pms=pms,
        ams=ams,
        filter_cutoff=filter_cutoff,
        filter_resonance=filter_resonance,
        filter_type=filter_type,
        filter_self_oscillation=filter_self_oscillation,
    )
    cutoff_eg = json.loads(_time_eg_json(filter_eg_ar, filter_eg_d1r, filter_eg_d1l, 0, filter_eg_rr))
    for stage in cutoff_eg["stages"]:
        stage["level"] = max(0, min(255, round(128 + stage["level"] * 128 / 255)))
    ch["cutoff_fg"] = {
        "eg": cutoff_eg,
        "depth": max(0, min(255, filter_eg_depth)),
    }
    ch.update(extra)
    return ch
