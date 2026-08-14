"""probe_results.csv から記述子最近傍を探して即レンダリング・再生するツール。

使い方:
    python lookup.py "brightness=0.5,distortion=0.5,odd_even=0.85,warmth=0.3"
    python lookup.py "brightness=0.5,distortion=0.5" --top 5 --note 60
    python lookup.py "brightness=0.5,distortion=0.5" --csv private/wave_probe/probe_results.csv
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

import features as ft
import op505_patch  # noqa: E402
import patchlab  # noqa: E402

SR = 44100.0
ON_SECS = 2.0
RELEASE_SECS = 1.0
VELOCITY = 100

DESCRIPTOR_KEYS = [
    "brightness", "warmth", "metallic", "roughness",
    "distortion", "odd_even", "noise", "attack", "decay", "brightness_decay",
]


def _parse_target(s: str) -> dict[str, float]:
    result: dict[str, float] = {}
    valid = set(DESCRIPTOR_KEYS)
    for part in s.split(","):
        part = part.strip()
        if "=" not in part:
            continue
        k, v = part.split("=", 1)
        k = k.strip()
        if k in valid:
            result[k] = float(v.strip())
        else:
            print(f"WARNING: 不明なキー '{k}' を無視", file=sys.stderr)
    return result


def _make_patch(mod_wf: int, car_wf: int, mod_tl: int,
                mul_mod: int, mul_car: int, feedback: int) -> str:
    def op(wf: int, tl: int, mul: int, ar: int = 255) -> dict:
        return op505_patch.make_op(tl=tl, ar=ar, d1r=30, d2r=0, d1l=255, rr=150,
                    mul=mul, dt1=128, ksr=0, am_enable=False,
                    velocity_sensitivity=0, waveform=wf, op_fine_tune=128)
    silent = op(wf=0, tl=255, mul=1, ar=0)
    ch = op505_patch.make_channel(algorithm=4, feedback=feedback)
    patch = {"operators": [op(mod_wf, mod_tl, mul_mod), op(car_wf, 160, mul_car),
                            silent, silent], "channel": ch}
    return json.dumps(patch)


def _write_wav(path: Path, samples, sr: float) -> None:
    x = np.asarray(samples, dtype=np.float32)
    peak = float(np.max(np.abs(x))) if x.size else 0.0
    if peak > 1e-6:
        x = x / peak * 0.9
    pcm = np.clip(x * 32767.0, -32768, 32767).astype("<i2")
    with _wave.open(str(path), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(int(sr))
        w.writeframes(pcm.tobytes())


def _play_wav(path: Path) -> None:
    try:
        import winsound
        winsound.PlaySound(str(path), winsound.SND_FILENAME)
    except ImportError:
        print(f"  (再生はWindows専用です。WAVを直接開いてください: {path})")


def main() -> int:
    ap = argparse.ArgumentParser(description="probe CSV から記述子最近傍を即検索・再生")
    ap.add_argument("target", type=str,
                    help="ターゲット記述子 (例: 'brightness=0.5,distortion=0.5')")
    ap.add_argument("--csv", type=str, default=None,
                    help="probe_results.csv のパス（省略時は private/wave_probe/probe_results.csv）")
    ap.add_argument("--top", type=int, default=5, help="表示する候補数（デフォルト5）")
    ap.add_argument("--note", type=int, default=60, help="MIDIノート番号（デフォルト60=C4）")
    ap.add_argument("--play", action="store_true", default=True,
                    help="1位をWAVに書き出して再生（デフォルトON）")
    ap.add_argument("--no-play", dest="play", action="store_false")
    ap.add_argument("--out-dir", type=str, default=None,
                    help="WAV出力先（省略時は private/lookup_wav/）")
    args = ap.parse_args()

    target = _parse_target(args.target)
    if not target:
        print("ERROR: 有効な記述子がありません", file=sys.stderr)
        return 1

    base = Path(__file__).resolve().parent.parent / "private"
    csv_path = Path(args.csv) if args.csv else base / "wave_probe" / "probe_results.csv"
    if not csv_path.exists():
        print(f"ERROR: {csv_path} が見つかりません。先に probe.py を実行してください",
              file=sys.stderr)
        return 1

    # ── CSV 読み込み ──────────────────────────────────────────────────────
    rows: list[dict] = []
    with open(csv_path, newline="", encoding="utf-8") as f:
        for r in csv.DictReader(f):
            desc = {k: float(r[k]) for k in DESCRIPTOR_KEYS if k in r}
            rows.append({**r, **desc})

    print(f"probe CSV: {csv_path.name} ({len(rows)} 件)")
    print(f"ターゲット: {target}")
    print()

    # ── 距離計算（指定キーのみ）──────────────────────────────────────────
    weights = {k: ft.DESCRIPTOR_WEIGHTS.get(k, 1.0) for k in target}
    scored: list[tuple[float, dict]] = []
    for r in rows:
        dist, _ = ft.descriptor_distance(target, r, weights=weights)
        scored.append((dist, r))
    scored.sort(key=lambda x: x[0])

    # ── 結果表示 ──────────────────────────────────────────────────────────
    key_w = max(len(k) for k in target)
    print(f"{'順位':>4}  {'dist':>6}  {'ラベル':<50}  " +
          "  ".join(f"{k:>{max(len(k),5)}}" for k in target))
    print("-" * (70 + len(target) * 8))
    for rank, (dist, r) in enumerate(scored[:args.top], 1):
        vals = "  ".join(f"{r.get(k, 0.0):>{max(len(k),5)}.3f}" for k in target)
        print(f"{rank:>4}  {dist:>6.3f}  {r['label']:<50}  {vals}")

    # ── 1位を再生 ─────────────────────────────────────────────────────────
    if args.play:
        best_r = scored[0][1]
        mod_wf  = int(best_r["mod_wf"])
        car_wf  = int(best_r["car_wf"])
        mod_tl  = int(best_r["mod_tl"])
        mul_mod = int(best_r["mul_mod"])
        mul_car = int(best_r["mul_car"])
        fb      = int(best_r.get("fb", 0))

        freq = 440.0 * 2 ** ((args.note - 69) / 12.0)
        patch_json = _make_patch(mod_wf, car_wf, mod_tl, mul_mod, mul_car, fb)
        samples = patchlab.render_patch(patch_json, freq, ON_SECS, RELEASE_SECS, VELOCITY, SR)

        out_dir = Path(args.out_dir) if args.out_dir else base / "lookup_wav"
        out_dir.mkdir(parents=True, exist_ok=True)
        label = best_r["label"].replace("/", "_")
        wav_path = out_dir / f"{label}.wav"
        _write_wav(wav_path, samples, SR)

        print(f"\n>> 再生: {wav_path.name}")
        _play_wav(wav_path)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
