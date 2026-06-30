"""FMパラメーター比較ツール: 1パラメーターを振って比較WAVを一括生成する。

使い方:
    # piano_template.py の prog 4 の brightness を3段階比較
    python fm_compare.py --prog 4 --param brightness --values 138,155,168

    # 複数パラメーター同時指定（直積）
    python fm_compare.py --prog 4 --param feedback --values 40,72,108

出力: private/audition_wav/compare_<prog>_<param>/<label>.wav
      + サマリー（何がどう変わるか）
"""

from __future__ import annotations

import argparse
import json
import sys
import wave as _wave
from pathlib import Path

import numpy as np

BASE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.stdout.reconfigure(encoding="utf-8", errors="replace")

import ym38x6_ml  # noqa: E402

SR = 44100.0
BPM = 120.0
QUARTER = 60.0 / BPM
WHOLE   = QUARTER * 4
STRUM   = 0.04

STRUM_CHORDS = [
    ("C2", [36, 43, 47]),
    ("C3", [48, 55, 59]),
    ("C4", [60, 67, 71]),
    ("C5", [72, 79, 83]),
]


def _write_wav(path: Path, x: np.ndarray) -> None:
    pcm = np.clip(x * 32767, -32768, 32767).astype("<i2")
    with _wave.open(str(path), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(int(SR))
        w.writeframes(pcm.tobytes())


def render_strum(patch_json: str) -> np.ndarray:
    frames = []
    for i, (name, midis) in enumerate(STRUM_CHORDS):
        is_last = (i == len(STRUM_CHORDS) - 1)
        slot_dur = WHOLE if is_last else QUARTER
        on_time  = slot_dur * 0.90
        rel_time = WHOLE * 0.6 if is_last else slot_dur * 0.15
        buf = np.zeros(int((slot_dur + STRUM * (len(midis) - 1)) * SR), dtype=np.float32)
        for j, midi in enumerate(midis):
            offset = int(j * STRUM * SR)
            freq = 440.0 * 2 ** ((midi - 69) / 12.0)
            s = ym38x6_ml.render_patch(patch_json, freq, on_time, rel_time, 100, SR)
            x = np.asarray(s, dtype=np.float32)
            end = min(offset + len(x), len(buf))
            buf[offset:end] += x[:end - offset]
        buf /= len(midis)
        frames.append(buf)
    return np.concatenate(frames)


def main() -> int:
    ap = argparse.ArgumentParser(description="FMパラメーター比較ツール")
    ap.add_argument("--prog", type=int, required=True,
                    help="piano_template.py のプログラム番号")
    ap.add_argument("--param", type=str, required=True,
                    help="変化させるパラメーター名（例: brightness, feedback, mod_wf）")
    ap.add_argument("--values", type=str, required=True,
                    help="カンマ区切りの値リスト（例: 138,155,168）")
    ap.add_argument("--out-dir", type=str, default=None)
    args = ap.parse_args()

    try:
        from piano_template import make_piano_patch, PIANO_FAMILY
    except ImportError:
        print("ERROR: piano_template.py が見つかりません", file=sys.stderr)
        return 1

    if args.prog not in PIANO_FAMILY:
        print(f"ERROR: program {args.prog} は未定義", file=sys.stderr)
        return 1

    name, base_knobs = PIANO_FAMILY[args.prog]
    values = [int(v) if v.lstrip("-").isdigit() else float(v)
              for v in args.values.split(",")]

    out_base = Path(args.out_dir) if args.out_dir else \
        BASE / "private" / "audition_wav" / f"compare_{args.prog:03d}_{args.param}"
    out_base.mkdir(parents=True, exist_ok=True)

    print(f"比較: prog={args.prog} ({name})  param={args.param}  values={values}")
    print(f"出力: {out_base}\n")

    for val in values:
        knobs = {**base_knobs, args.param: val}
        try:
            patch = make_piano_patch(**knobs)
        except TypeError as e:
            print(f"  ERROR {args.param}={val}: {e}", file=sys.stderr)
            continue
        audio = render_strum(json.dumps(patch))
        label = f"{args.param}_{val}"
        _write_wav(out_base / f"{label}.wav", audio)
        print(f"  {label}")

    print(f"\n全{len(values)}バリアント生成完了")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
