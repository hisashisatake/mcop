"""音色試聴ツール: 単音 C2-C5 + ストラムアルペジオを生成する。

使い方:
    # piano_template.py のプログラム番号を指定
    python audition.py --prog 4

    # .38x6 バンクファイルから program 番号を指定
    python audition.py --bank private/piano_template.38x6 --prog 4

    # バンク全プリセットを一括生成
    python audition.py --bank private/piano_template.38x6

出力: private/audition_wav/<label>/ に single_*.wav + strum.wav
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

import patchlab  # noqa: E402

SR = 44100.0
BPM = 120.0
QUARTER = 60.0 / BPM
WHOLE   = QUARTER * 4
STRUM   = 0.04

SINGLE_NOTES = [
    ("C2", 36), ("C3", 48), ("C4", 60), ("C5", 72),
]
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


def render_single(patch_json: str, out_dir: Path) -> None:
    """C2〜C5 単音を各2秒鳴らして2秒リリース。"""
    frames = []
    for name, midi in SINGLE_NOTES:
        freq = 440.0 * 2 ** ((midi - 69) / 12.0)
        s = patchlab.render_patch(patch_json, freq, 2.0, 2.0, 100, SR)
        frames.append(np.asarray(s, dtype=np.float32))
        print(f"    {name}")
    audio = np.concatenate(frames)
    _write_wav(out_dir / "single_C2toC5.wav", audio)


def render_strum(patch_json: str, out_dir: Path) -> None:
    """C2→C5 ストラムアルペジオ（4分音符 × 3 + 全音符）。"""
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
            s = patchlab.render_patch(patch_json, freq, on_time, rel_time, 100, SR)
            x = np.asarray(s, dtype=np.float32)
            end = min(offset + len(x), len(buf))
            buf[offset:end] += x[:end - offset]
        buf /= len(midis)
        frames.append(buf)
    audio = np.concatenate(frames)
    _write_wav(out_dir / "strum.wav", audio)


def audition(patch_json: str, label: str, out_base: Path) -> None:
    out_dir = out_base / label
    out_dir.mkdir(parents=True, exist_ok=True)
    print(f"  [{label}]")
    print(f"    単音 C2-C5")
    render_single(patch_json, out_dir)
    print(f"    ストラム")
    render_strum(patch_json, out_dir)
    print(f"    出力: {out_dir}")


def main() -> int:
    ap = argparse.ArgumentParser(description="音色試聴ツール")
    ap.add_argument("--prog", type=int, default=None,
                    help="プログラム番号（piano_template.py / バンクファイル共通）")
    ap.add_argument("--bank", type=str, default=None,
                    help=".38x6 バンクファイルのパス")
    ap.add_argument("--out-dir", type=str, default=None,
                    help="出力先（デフォルト: private/audition_wav）")
    args = ap.parse_args()

    out_base = Path(args.out_dir) if args.out_dir else BASE / "private" / "audition_wav"

    # ── バンクファイルから読み込む ──────────────────────────────────────────
    if args.bank:
        bank_path = Path(args.bank)
        if not bank_path.exists():
            print(f"ERROR: {bank_path} が見つかりません", file=sys.stderr)
            return 1
        data = json.loads(bank_path.read_text(encoding="utf-8"))
        presets = data.get("presets", [])
        if args.prog is not None:
            presets = [p for p in presets if p["program"] == args.prog]
            if not presets:
                print(f"ERROR: program {args.prog} がバンクに存在しません", file=sys.stderr)
                return 1
        print(f"バンク: {bank_path.name}  ({len(presets)} 音色)")
        for p in presets:
            label = f"{p['program']:03d}_{p['name']}"
            audition(json.dumps(p["patch"]), label, out_base)
        return 0

    # ── piano_template.py から読み込む ─────────────────────────────────────
    try:
        from piano_template import make_piano_patch, PIANO_FAMILY
    except ImportError:
        print("ERROR: piano_template.py が見つかりません", file=sys.stderr)
        return 1

    if args.prog is not None:
        progs = [args.prog]
    else:
        progs = list(PIANO_FAMILY.keys())

    print(f"piano_template.py  ({len(progs)} 音色)")
    for prog in progs:
        if prog not in PIANO_FAMILY:
            print(f"  WARNING: program {prog} は未定義")
            continue
        name, knobs = PIANO_FAMILY[prog]
        patch = make_piano_patch(**knobs)
        label = f"{prog:03d}_{name}"
        audition(json.dumps(patch), label, out_base)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
