"""波形メモリ専用音色バンク（Waveform Memory Voice Bank）: bank=16383。

かつてWMS-1（波形メモリ音源プロトタイプ）→ym38x6-core::preset::waveform_memory_patchが
提供していた「OP1のみ可聴の1オペレーター音色を予約バンクに用意する」仕組みのop505移植版。
ym38x6一式削除（2026-08-20）に伴いop505側は未移植のままだったが、gesture-app（main.js）の
UIはBank=16383前提のまま残されている。

op505では「特殊バンクを実行時にコードでフォールバック生成する」パターン
（旧`PresetBank::patch_for_program`）自体を廃止し、GM2ドラムキット（gm2_drum_kit.py）と
同様に「実体の.op505ファイルを生成し、通常のプリセットバンクとして読み込む」方式へ統一した。
このスクリプトも同じ方式を踏襲する（op505-core/op505-vst/gesture-appのコード変更は不要）。

波形はym38x6版の基本4波形（sine/saw/square/triangle、いずれもOPZ8変換の「フル」バリアント）
ではなく、**op505の内蔵波形32種すべて**（`sound_fm::waveform::BUILTIN_WAVEFORM_COUNT=32`、
0-7=サイン系×8変換/8-15=ノコギリ系×8変換/16-23=矩形系(PWM+独自2種)/24-31=三角系×8変換）を使う
（ユーザー指定、2026-08-25。当初は元の4波形のみだったが、32種を使うよう再作成した）。

Program番号は**前半32個（0〜31）をピアノ風ADSR、後半32個（32〜63）をリード風ADSR**に割り当てる
（ym38x6版のピアノ/リードを4個ずつ交互に並べる`program % 8`からの変更、ユーザー指定2026-08-25）。
波形32種×ADSR2種＝重複のない64通りがちょうど収まるため、Program 64〜127は使わない（当初は
128個まで波形を2周させて埋めていたが、「同一波形があるなら64個にまとめてほしい」との指示で
重複を無くす形に変更した）。

使い方:
    python waveform_memory_bank.py                                  # 既定 private/waveform_memory/waveform_memory_bank.op505
    python waveform_memory_bank.py --bank private/waveform_memory/waveform_memory_bank.op505
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.stdout.reconfigure(encoding="utf-8", errors="replace")

import op505_patch  # noqa: E402

WAVEFORM_MEMORY_BANK = 16383  # gesture-app/src/main.js の WAVEFORM_MEMORY_BANK と一致させる

# リード風ADSRが始まるProgram番号（0〜31がピアノ風、32〜63がリード風）。波形32種と重複なく
# ちょうど収まる境界のため、この値は波形数(len(WAVE_NAMES))と一致させる必要がある。
# gesture-app/src/main.js の WAVEFORM_MEMORY_LEAD_START と一致させること。
LEAD_RANGE_START = 32

# 生成するProgram総数（0〜TOTAL_PROGRAMS-1）。波形32種×ADSR2種＝64がちょうど重複なしの上限。
TOTAL_PROGRAMS = 64

# op505ビルトイン波形32種（sound_fm::waveform::gen_builtin_waveform）の表示名。
# 基本波4種×変換8種。サイン/ノコギリ/三角は同じOPZ8変換（full/sin2/half/half-sin2/
# half2x/half-sin2-2x/half-abs-2x/half-pos-2x）を共有するが、矩形はOPZの2乗/絶対値変換が
# 縮退するため専用のPWMデューティスイープ+独自2種（gen_square_family、waveform.rsコメント参照）。
_BASE_NAMES = ["Sine", "Saw", "Square", "Triangle"]
_OPZ_VARIANT_NAMES = ["Full", "Sin2", "Half", "HalfSin2", "Half2x", "HalfSin2x2", "HalfAbs2x", "HalfPos2x"]
_SQUARE_VARIANT_NAMES = ["PWM50", "PWM33", "PWM25", "PWM17", "PWM13", "PWM6", "HalfSquare", "Square2x"]


def _waveform_name(waveform: int) -> str:
    base_index, variant = divmod(waveform, 8)
    base_name = _BASE_NAMES[base_index]
    variant_name = _SQUARE_VARIANT_NAMES[variant] if base_index == 2 else _OPZ_VARIANT_NAMES[variant]
    return f"{base_name} {variant_name}"


WAVE_NAMES = [_waveform_name(i) for i in range(32)]

assert LEAD_RANGE_START == len(WAVE_NAMES), "波形32種と重複なく収まる前提が崩れている"
assert TOTAL_PROGRAMS == 2 * len(WAVE_NAMES), "Piano/Lead 2スタイル分の前提が崩れている"


def piano_adsr() -> dict:
    """Program 0〜31の「ピアノ風」ADSR。アタック即時、緩やかな減衰で中程度のサスティンへ
    落ち着く（鍵盤を保持していても音量が下がっていく、減衰楽器寄りの質感）。"""
    return dict(ar=255, d1r=80, d1l=120, rr=130)


def lead_adsr() -> dict:
    """Program 32〜63の「リード風」ADSR。decay=0・sustain=255で減衰せず、キーを保持する間は
    最大レベルを維持する無限サスティン。RRはピアノ風と同じ値にし余韻の長さを揃える。"""
    return dict(ar=255, d1r=0, d1l=255, rr=130)


def make_waveform_memory_patch(waveform: int, adsr: dict) -> dict:
    """OP1のみ可聴の1オペレーター音色。Algorithm 7（全並列）でOP2〜4はtl=0でミュートする
    （ym38x6-core::preset::waveform_memory_patchのop505移植版）。OP2〜4にもOP1と同じEGタイミング
    (ar/d1r/d1l/rr)を持たせるのは、is_idle()が「全4オペレーターがidle」条件のため
    （gm2_drum_kit.pyのmake_drum_patchと同じ理由、そうしないとOP2〜4がホールド段に停滞し
    ボイスが解放されない）。"""
    op1 = op505_patch.make_op(
        tl=255, ar=adsr["ar"], d1r=adsr["d1r"], d2r=0, d1l=adsr["d1l"], rr=adsr["rr"],
        mul=1, dt1=128, ksr=0, am_enable=False, velocity_sensitivity=0,
        waveform=waveform, op_fine_tune=128,
    )
    muted = dict(op1, tl=0)
    ch = op505_patch.make_channel(algorithm=7, feedback=0)
    return {"operators": [op1, muted, muted, muted], "channel": ch}


def params_for_program(program: int) -> tuple[int, dict, str]:
    """Program番号(0〜63)から(waveform, ADSR, スタイル名)を決める。

    前半32個(0〜31)をピアノ風ADSR・波形0〜31、後半32個(32〜63)をリード風ADSR・波形0〜31
    （同じ32波形をもう一度）に割り当てる。波形32種×ADSR2種で重複なくちょうど64通りになる
    （「下半分がピアノ系・上半分がリード系」という把握しやすさを優先、ym38x6版の`program % 8`＝
    4個ずつ交互に並べる方式からの変更、ユーザー指定2026-08-25）。
    """
    waveform = program % len(WAVE_NAMES)
    if program < LEAD_RANGE_START:
        return waveform, piano_adsr(), "Piano"
    return waveform, lead_adsr(), "Lead"


def build_presets() -> list[dict]:
    presets = []
    for program in range(TOTAL_PROGRAMS):
        waveform, adsr, style = params_for_program(program)
        name = f"{WAVE_NAMES[waveform]} ({style})"
        presets.append({
            "program": program,
            "name": name,
            "patch": make_waveform_memory_patch(waveform, adsr),
        })
    return presets


def main() -> int:
    ap = argparse.ArgumentParser(description="波形メモリ専用音色バンク（bank=16383）生成")
    ap.add_argument("--bank", type=str, default=None,
                     help="出力先.op505パス（既定 private/waveform_memory/waveform_memory_bank.op505）")
    args = ap.parse_args()

    base = Path(__file__).resolve().parent.parent
    out_path = Path(args.bank) if args.bank else base / "private" / "waveform_memory" / "waveform_memory_bank.op505"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    presets = build_presets()
    out_path.write_text(json.dumps({"bank": WAVEFORM_MEMORY_BANK, "presets": presets}, indent=2), encoding="utf-8")

    print(f"波形メモリ専用音色バンク: {len(presets)}音色  bank={WAVEFORM_MEMORY_BANK}")
    for program in [*range(4), *range(LEAD_RANGE_START, LEAD_RANGE_START + 4)]:
        waveform, _adsr, style = params_for_program(program)
        print(f"  [{program:3d}] {WAVE_NAMES[waveform]:<14} ({style})  waveform={waveform}")
    print(f"  Program 0〜{LEAD_RANGE_START - 1}=Piano・波形0〜{LEAD_RANGE_START - 1} / "
          f"{LEAD_RANGE_START}〜{TOTAL_PROGRAMS - 1}=Lead・波形0〜{LEAD_RANGE_START - 1}（重複なし）")
    print(f"\nバンク書き出し: {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
