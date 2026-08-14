"""波形プローブ: op505 の非サイン波形を系統的に評価するツール。

2OP（OP0=モジュレーター、OP1=キャリア）・アルゴリズム4固定で、
全32波形の音をレンダリングして perceptual_descriptors で評価する。
WAV（試聴用）と CSV（数値比較用）を対にして出力する。

使い方:
    # 全キャリア波形 × モジュレーション3段階（デフォルト）
    python probe.py --note 60

    # モジュレーターも振る
    python probe.py --mod-wf 0,8,16,24 --car-wf 0-31 --tl 150

    # MUL比を変える
    python probe.py --mul-mod 2 --mul-car 1 --tl 150

出力: private/wave_probe/ 以下に WAV と probe_results.csv
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
import wave as _wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

import features as ft   # noqa: E402
import op505_patch       # noqa: E402
import patchlab          # noqa: E402

SR = 44100.0
ON_SECS = 1.5
RELEASE_SECS = 0.5
VELOCITY = 100

# abys.py のコメント（waveform.rs 番号体系）に準拠した波形名
WAVEFORM_NAMES = [
    # Sine group (0-7)
    "sine",        "sin2",        "half-sine",   "half-sin2",
    "2x-sine-h",   "2x-sin2-h",   "abs-sin-2x",  "sin2-2x",
    # Saw group (8-15)
    "saw",         "saw2",        "half-saw",    "half-saw2",
    "2x-saw-h",    "2x-saw2-h",   "abs-saw-2x",  "saw2-2x",
    # Square/PWM group (16-23)
    "sq50",        "sq33",        "sq25",        "sq17",
    "sq12",        "sq6",         "half-sq",     "2x-sq-h",
    # Triangle group (24-31)
    "tri",         "tri2",        "half-tri",    "half-tri2",
    "2x-tri-h",    "2x-tri2-h",   "abs-tri-2x",  "tri2-2x",
]

WAVEFORM_GROUPS = {
    "sine": range(0, 8),
    "saw":  range(8, 16),
    "sq":   range(16, 24),
    "tri":  range(24, 32),
}

DESCRIPTOR_KEYS = [
    "brightness", "warmth", "metallic", "roughness",
    "distortion", "odd_even", "noise", "attack", "decay", "brightness_decay",
]


def _midi_to_freq(note: int) -> float:
    return 440.0 * 2 ** ((note - 69) / 12.0)


def _parse_wf_list(s: str) -> list[int]:
    """'0,8,16,24' または '0-31' → int リスト。"""
    result: list[int] = []
    for part in s.split(","):
        part = part.strip()
        if "-" in part:
            a, b = part.split("-", 1)
            result.extend(range(int(a), int(b) + 1))
        else:
            result.append(int(part))
    return sorted(set(result))


def _make_patch(mod_wf: int, car_wf: int, mod_tl: int,
                mul_mod: int, mul_car: int, feedback: int) -> str:
    """2OP プローブ用パッチ JSON を生成する。

    Algorithm 4: (OP0→OP1) + (OP2→OP3) で、OP2/OP3 は AR=0 で完全無音化。
    """
    def op(wf: int, tl: int, mul: int, ar: int = 255) -> dict:
        return op505_patch.make_op(
            tl=tl, ar=ar, d1r=30, d2r=0, d1l=255, rr=150,
            mul=mul, dt1=128, ksr=0, am_enable=False,
            velocity_sensitivity=0, waveform=wf, op_fine_tune=128,
        )

    silent = op(wf=0, tl=255, mul=1, ar=0)   # AR=0 = 完全無音

    ch = op505_patch.make_channel(algorithm=4, feedback=feedback)
    patch = {
        "operators": [
            op(mod_wf, mod_tl, mul_mod),   # OP0: モジュレーター
            op(car_wf, 160, mul_car),       # OP1: キャリア（TL=160）
            silent,                          # OP2: 無音
            silent,                          # OP3: 無音
        ],
        "channel": ch,
    }
    return json.dumps(patch)


def _write_wav(path: Path, samples, sr: float) -> None:
    x = np.asarray(samples, dtype=np.float32)
    peak = float(np.max(np.abs(x))) if x.size else 0.0
    if peak > 1e-6:
        x = x / peak * 0.9
    pcm = np.clip(x * 32767.0, -32768, 32767).astype("<i2")
    with _wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(int(sr))
        w.writeframes(pcm.tobytes())


def main() -> int:
    ap = argparse.ArgumentParser(
        description="op505 波形プローブ: 2OP系統探索 + 知覚記述子評価")
    ap.add_argument("--note", type=int, default=60,
                    help="基音 MIDI ノート番号（デフォルト 60 = C4）")
    ap.add_argument("--freq", type=float, default=None,
                    help="基音周波数 Hz（--note を上書き）")
    ap.add_argument("--mod-wf", type=str, default="0",
                    help="モジュレーター波形番号リスト（例: 0,8,16,24 または 0-7）"
                         "。デフォルト: 0（サイン）")
    ap.add_argument("--car-wf", type=str, default="0-31",
                    help="キャリア波形番号リスト（デフォルト: 0-31 全波形）")
    ap.add_argument("--tl", type=str, default="255,180,120",
                    help="モジュレーター TL 値リスト（255=無変調, 小=深い変調）"
                         "。デフォルト: 255,180,120")
    ap.add_argument("--mul-mod", type=int, default=1,
                    help="モジュレーター MUL（デフォルト: 1）")
    ap.add_argument("--mul-car", type=int, default=1,
                    help="キャリア MUL（デフォルト: 1）")
    ap.add_argument("--fb", type=int, default=0,
                    help="フィードバック 0-255（デフォルト: 0）")
    ap.add_argument("--out-dir", type=str, default=None,
                    help="出力ディレクトリ（デフォルト: private/wave_probe）")
    ap.add_argument("--no-wav", action="store_true",
                    help="WAV を書き出さない（CSV のみ）")
    args = ap.parse_args()

    freq = args.freq if args.freq else _midi_to_freq(args.note)
    mod_wfs = _parse_wf_list(args.mod_wf)
    car_wfs = _parse_wf_list(args.car_wf)
    tl_list = [int(v.strip()) for v in args.tl.split(",")]

    base = Path(__file__).resolve().parent.parent / "private"
    out_dir = Path(args.out_dir) if args.out_dir else base / "wave_probe"
    out_dir.mkdir(parents=True, exist_ok=True)

    n_total = len(mod_wfs) * len(car_wfs) * len(tl_list)
    print(f"波形プローブ: {n_total} パッチ")
    print(f"  note={args.note} freq={freq:.2f}Hz  "
          f"MUL mod={args.mul_mod} car={args.mul_car}  FB={args.fb}")
    print(f"  mod_wf={mod_wfs}  car_wf=0-31({len(car_wfs)}種)  TL={tl_list}")
    print(f"  出力先: {out_dir}")

    rows: list[dict] = []

    for mod_wf in mod_wfs:
        for tl in tl_list:
            for car_wf in car_wfs:
                mod_name = WAVEFORM_NAMES[mod_wf]
                car_name = WAVEFORM_NAMES[car_wf]
                tl_label = {255: "none", 180: "light", 120: "med", 60: "heavy"}.get(tl, str(tl))
                label = f"m{mod_wf:02d}-{mod_name}_c{car_wf:02d}-{car_name}_tl{tl_label}"

                patch_json = _make_patch(
                    mod_wf, car_wf, tl,
                    args.mul_mod, args.mul_car, args.fb)
                samples = patchlab.render_patch(
                    patch_json, freq, ON_SECS, RELEASE_SECS, VELOCITY, SR)
                samples = np.asarray(samples, dtype=np.float32)

                if ft.is_silent(samples, thresh=1e-4):
                    print(f"  無音スキップ: {label}")
                    continue

                desc = ft.perceptual_descriptors(samples, freq, sr=SR)

                row = {"label": label,
                       "mod_wf": mod_wf, "mod_name": mod_name,
                       "car_wf": car_wf, "car_name": car_name,
                       "mod_tl": tl, "mul_mod": args.mul_mod,
                       "mul_car": args.mul_car, "fb": args.fb}
                row.update(desc)
                rows.append(row)

                if not args.no_wav:
                    _write_wav(out_dir / f"{label}.wav", samples, SR)

                print(f"  {label:55s} "
                      f"Br={desc['brightness']:.2f} Wa={desc['warmth']:.2f} "
                      f"Me={desc['metallic']:.2f} Ro={desc['roughness']:.2f} "
                      f"Di={desc['distortion']:.2f} OE={desc['odd_even']:.2f}")

    if not rows:
        print("ERROR: 有効パッチが 0 件", file=sys.stderr)
        return 1

    # CSV 出力
    csv_path = out_dir / "probe_results.csv"
    fieldnames = (["label", "mod_wf", "mod_name", "car_wf", "car_name",
                   "mod_tl", "mul_mod", "mul_car", "fb"] + DESCRIPTOR_KEYS)
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        for r in rows:
            writer.writerow({k: (f"{r[k]:.4f}" if isinstance(r[k], float) else r[k])
                             for k in fieldnames})
    print(f"\nCSV: {csv_path} ({len(rows)} 行)")

    # サマリー: roughness 低い順・distortion 低い順のトップ10
    print("\n── roughness 低い順（にごりが少ない）──")
    for r in sorted(rows, key=lambda x: x["roughness"])[:10]:
        print(f"  {r['label']:55s} Ro={r['roughness']:.3f} Di={r['distortion']:.3f} "
              f"Br={r['brightness']:.2f} OE={r['odd_even']:.2f}")

    print("\n── distortion 低い順（歪みが少ない）──")
    for r in sorted(rows, key=lambda x: x["distortion"])[:10]:
        print(f"  {r['label']:55s} Di={r['distortion']:.3f} Ro={r['roughness']:.3f} "
              f"Br={r['brightness']:.2f} OE={r['odd_even']:.2f}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
