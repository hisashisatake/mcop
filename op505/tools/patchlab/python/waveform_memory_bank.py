"""波形メモリ専用音色バンク（Waveform Memory Voice Bank）: bank=16383。

かつてWMS-1（波形メモリ音源プロトタイプ）→ym38x6-core::preset::waveform_memory_patchが
提供していた「OP1のみ可聴の1オペレーター音色を予約バンクに用意する」仕組みのop505移植版。
ym38x6一式削除（2026-08-20）に伴いop505側は未移植のままだったが、gesture-app（main.js）の
UIはBank=16383前提のまま残されている。

op505では「特殊バンクを実行時にコードでフォールバック生成する」パターン
（旧`PresetBank::patch_for_program`）自体を廃止し、GM2ドラムキット（gm2_drum_kit.py）と
同様に「実体の.op505ファイルを生成し、通常のプリセットバンクとして読み込む」方式へ統一した。
このスクリプトも同じ方式を踏襲する（op505-core/op505-vst/gesture-appのコード変更は不要）。

波形は元のym38x6版と同じ基本4波形（sine/saw/square/triangle、いずれもOPZ8変換の
「フル」バリアント）× ピアノ風/リード風ADSRの8パターンをProgram 0〜127に繰り返す
（`program % 8`。ym38x6-core::preset::waveform_memory_params_for_programと同じ規則）。
op505は32種の内蔵波形を持つが、忠実な移植を優先しここでは元の4波形のみを使う。

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

# ym38x6-core::preset::WAVEFORM_MEMORY_BASE_WAVEFORMSと同じ4値。各基本波はOPZ8変換で
# 8波形番号を占有するため（0-7=sine系/8-15=saw系/16-23=square系/24-31=triangle系）、
# 変換なしの「フル」バリアント=各グループの先頭番号を使う。
BASE_WAVEFORMS = [0, 8, 16, 24]
WAVE_NAMES = ["Sine", "Saw", "Square", "Triangle"]


def piano_adsr() -> dict:
    """Program 0〜3の「ピアノ風」ADSR。アタック即時、緩やかな減衰で中程度のサスティンへ
    落ち着く（鍵盤を保持していても音量が下がっていく、減衰楽器寄りの質感）。"""
    return dict(ar=255, d1r=80, d1l=120, rr=130)


def lead_adsr() -> dict:
    """Program 4〜7の「リード風」ADSR。decay=0・sustain=255で減衰せず、キーを保持する間は
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
    """Program番号(0〜127)から(waveform, ADSR, スタイル名)を決める。基本4波形×ピアノ/リードADSRの
    8パターンをprogram % 8で繰り返す（ym38x6-core::preset::waveform_memory_params_for_programと
    同じ規則。Program 0〜3=ピアノ風、4〜7=リード風、8以降は同じ並びの繰り返し）。"""
    slot = program % 8
    waveform = BASE_WAVEFORMS[slot % 4]
    if slot < 4:
        return waveform, piano_adsr(), "Piano"
    return waveform, lead_adsr(), "Lead"


def build_presets() -> list[dict]:
    presets = []
    for program in range(128):
        waveform, adsr, style = params_for_program(program)
        name = f"{WAVE_NAMES[program % 4]} ({style})"
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
    for program in range(8):
        waveform, _adsr, style = params_for_program(program)
        print(f"  [{program:3d}] {WAVE_NAMES[program % 4]:<10} ({style})  waveform={waveform}")
    print("  ... (8以降は同じ8パターンを繰り返し、Program 127まで)")
    print(f"\nバンク書き出し: {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
