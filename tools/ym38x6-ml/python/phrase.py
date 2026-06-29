"""定型フレーズ試聴ツール: ストラム / ファンク / バロックの3種を任意音色で生成。

使い方:
    python phrase.py --prog 7                         # 全3フレーズ
    python phrase.py --prog 6 --type baroque          # バロックのみ
    python phrase.py --bank private/foo.38x6 --prog 4 --type funk

出力: private/audition_wav/<label>/phrase_<type>.wav
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


def _write_wav(path: Path, x: np.ndarray) -> None:
    peak = float(np.max(np.abs(x)))
    if peak > 1e-6:
        x = x / peak * 0.9
    pcm = np.clip(x * 32767, -32768, 32767).astype("<i2")
    with _wave.open(str(path), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(int(SR))
        w.writeframes(pcm.tobytes())


# ── ストラム ─────────────────────────────────────────────────────────────────
def render_strum(patch_json: str) -> np.ndarray:
    BPM = 120.0; Q = 60.0/BPM; W = Q*4; STRUM = 0.04
    CHORDS = [("C2",[36,43,47]),("C3",[48,55,59]),("C4",[60,67,71]),("C5",[72,79,83])]
    frames = []
    for i, (_, midis) in enumerate(CHORDS):
        is_last = (i == len(CHORDS)-1)
        slot = W if is_last else Q
        on = slot*0.90; rel = W*0.6 if is_last else slot*0.15
        buf = np.zeros(int((slot + STRUM*(len(midis)-1))*SR), dtype=np.float32)
        for j, midi in enumerate(midis):
            off = int(j*STRUM*SR); freq = 440.0*2**((midi-69)/12)
            s = ym38x6_ml.render_patch(patch_json, freq, on, rel, 100, SR)
            x = np.asarray(s, dtype=np.float32)
            end = min(off+len(x), len(buf)); buf[off:end] += x[:end-off]
        buf /= len(midis); frames.append(buf)
    return np.concatenate(frames)


# ── ファンクフレーズ（Eマイナーペンタ・トライアド） ───────────────────────
def render_funk(patch_json: str) -> np.ndarray:
    BPM = 112.0; S16 = 60.0/BPM/4
    TRIADS = {
        64:[64,67,71], 67:[67,71,74], 69:[69,72,76],
        71:[71,74,78], 74:[74,78,81],
    }
    PHRASE = [
        ( 0,64,1,110),( 2,67,1, 95),( 3,69,1,105),( 5,67,1, 90),
        ( 6,64,1,100),( 8,69,1,110),(10,71,1, 95),(11,69,1,100),
        (13,67,1, 90),(14,64,2, 85),(16,64,1,110),(18,67,1, 95),
        (19,69,1,105),(21,71,1,100),(22,74,1,115),(24,71,1, 95),
        (25,69,1,100),(27,67,1, 90),(28,64,1,105),(30,67,1, 90),
        (31,69,4,100),
    ]
    ON = S16*0.30; REL = 0.008; FADE = int(0.004*SR)
    total = int((max(s+d for s,_,d,_ in PHRASE)+8)*S16*SR)
    buf = np.zeros(total, dtype=np.float32)
    for s16, midi, dur, vel in PHRASE:
        on = max(ON, dur*S16*0.45); offset = int(s16*S16*SR)
        notes = TRIADS.get(midi, [midi])
        chord = np.zeros(int((on+REL)*SR), dtype=np.float32)
        for n in notes:
            freq = 440.0*2**((n-69)/12)
            x = np.asarray(ym38x6_ml.render_patch(patch_json, freq, on, REL, vel, SR), dtype=np.float32)
            l = min(len(chord), len(x)); chord[:l] += x[:l]
        chord /= len(notes)
        if len(chord) > FADE: chord[-FADE:] *= np.linspace(1,0,FADE)
        end = min(offset+len(chord), len(buf)); buf[offset:end] += chord[:end-offset]
    return buf


# ── バロック風プレリュード（Aマイナー） ─────────────────────────────────────
def render_baroque(patch_json: str) -> np.ndarray:
    BPM = 72.0; S16 = 60.0/BPM/4
    NOTE_ON = S16*5.5; NOTE_REL = 0.02
    PHRASE = [
        ( 0,57),( 1,64),( 2,69),( 3,72),( 4,64),( 5,69),( 6,72),( 7,64),
        ( 8,56),( 9,64),(10,68),(11,71),(12,64),(13,68),(14,71),(15,64),
        (16,55),(17,64),(18,69),(19,72),(20,64),(21,69),(22,72),(23,64),
        (24,62),(25,69),(26,74),(27,77),(28,69),(29,74),(30,77),(31,69),
        (32,52),(33,59),(34,62),(35,68),(36,59),(37,62),(38,68),(39,64),
        (40,57),(41,64),(42,69),(43,72),
    ]
    total = int((max(s for s,_ in PHRASE)+32)*S16*SR)
    buf = np.zeros(total, dtype=np.float32)
    for s16, midi in PHRASE:
        is_final = s16 >= 40
        on = NOTE_ON*4 if is_final else NOTE_ON
        rel = 1.5 if is_final else NOTE_REL
        offset = int(s16*S16*SR); freq = 440.0*2**((midi-69)/12)
        x = np.asarray(ym38x6_ml.render_patch(patch_json, freq, on, rel, 100, SR), dtype=np.float32)
        end = min(offset+len(x), len(buf)); buf[offset:end] += x[:end-offset]
    return buf


PHRASE_TYPES = {
    "strum":   ("ストラム C2-C5",          render_strum),
    "funk":    ("ファンク Eマイナー 112BPM", render_funk),
    "baroque": ("バロック Aマイナー 72BPM",  render_baroque),
}


def audition_phrases(patch_json: str, label: str, types: list[str], out_base: Path) -> None:
    out_dir = out_base / label
    out_dir.mkdir(parents=True, exist_ok=True)
    print(f"  [{label}]")
    for t in types:
        desc, fn = PHRASE_TYPES[t]
        print(f"    {desc}...")
        audio = fn(patch_json)
        peak = float(np.max(np.abs(audio)))
        if peak > 1e-6: audio = audio / peak * 0.9
        _write_wav(out_dir / f"phrase_{t}.wav", audio)
    print(f"    出力: {out_dir}")


def main() -> int:
    ap = argparse.ArgumentParser(description="定型フレーズ試聴ツール")
    ap.add_argument("--prog", type=int, default=None)
    ap.add_argument("--bank", type=str, default=None)
    ap.add_argument("--type", type=str, default=None,
                    help="フレーズ種別（strum / funk / baroque）。省略時は全種")
    ap.add_argument("--out-dir", type=str, default=None)
    args = ap.parse_args()

    types = [args.type] if args.type else list(PHRASE_TYPES.keys())
    invalid = [t for t in types if t not in PHRASE_TYPES]
    if invalid:
        print(f"ERROR: 不明なタイプ {invalid}。使用可能: {list(PHRASE_TYPES)}", file=sys.stderr)
        return 1

    out_base = Path(args.out_dir) if args.out_dir else BASE / "private" / "audition_wav"

    if args.bank:
        bank_path = Path(args.bank)
        data = json.loads(bank_path.read_text(encoding="utf-8"))
        presets = data.get("presets", [])
        if args.prog is not None:
            presets = [p for p in presets if p["program"] == args.prog]
        print(f"バンク: {bank_path.name}  ({len(presets)} 音色)")
        for p in presets:
            audition_phrases(json.dumps(p["patch"]),
                             f"{p['program']:03d}_{p['name']}", types, out_base)
        return 0

    try:
        from piano_template import make_piano_patch, PIANO_FAMILY
    except ImportError:
        print("ERROR: piano_template.py が見つかりません", file=sys.stderr)
        return 1

    progs = [args.prog] if args.prog is not None else list(PIANO_FAMILY.keys())
    print(f"piano_template.py  ({len(progs)} 音色)  types={types}")
    for prog in progs:
        if prog not in PIANO_FAMILY:
            print(f"  WARNING: prog {prog} 未定義"); continue
        name, knobs = PIANO_FAMILY[prog]
        audition_phrases(json.dumps(make_piano_patch(**knobs)),
                         f"{prog:03d}_{name}", types, out_base)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
