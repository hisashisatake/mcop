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


# ── タンゴ風フレーズ（Aマイナー・ハバネラリズム） ────────────────────────────
def render_tango(patch_json: str) -> np.ndarray:
    BPM = 130.0; S16 = 60.0 / BPM / 4

    # (s16オフセット, midi, 音符長(s16単位), スタッカートか)
    # A3=57 B3=59 C4=60 D4=62 E3=52 E4=64 F4=65 G3=55
    PHRASE = [
        # 小節1: ハバネラ（付点四分=6 + 八分=2 + 四分=4 + 四分=4）
        ( 0, 57, 6, False), ( 6, 64, 2, True),
        ( 8, 60, 4, True),  (12, 64, 4, False),
        # 小節2: 変奏
        (16, 57, 6, False), (22, 55, 2, True),
        (24, 52, 7, False),
        # 小節3: 上昇ライン
        (32, 57, 2, True),  (34, 59, 2, True),
        (36, 60, 2, True),  (38, 62, 2, True),
        (40, 64, 4, True),  (44, 65, 4, False),
        # 小節4: カデンツ（解決）
        (48, 64, 6, False), (54, 62, 2, True),
        (56, 60, 4, True),  (60, 57, 8, False),
    ]

    REL = 0.01
    total = int((60 + 24) * S16 * SR)
    buf = np.zeros(total, dtype=np.float32)
    for s16, midi, dur, stac in PHRASE:
        on = max(dur * S16 * (0.25 if stac else 0.85), 0.02)
        offset = int(s16 * S16 * SR)
        freq = 440.0 * 2 ** ((midi - 69) / 12.0)
        x = np.asarray(ym38x6_ml.render_patch(patch_json, freq, on, REL, 100, SR), dtype=np.float32)
        end = min(offset + len(x), len(buf)); buf[offset:end] += x[:end - offset]
    return buf


PHRASE_TYPES = {
    "strum":   ("ストラム C2-C5",              render_strum),
    "funk":    ("ファンク Eマイナー 112BPM",    render_funk),
    "baroque": ("バロック Aマイナー 72BPM",     render_baroque),
    "tango":   ("タンゴ Aマイナー 130BPM",      render_tango),
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
                    help="フレーズ種別（strum / funk / baroque / tango）。省略時は全種")
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

    # 全テンプレートを統合した prog → (name, patch_fn) レジストリを構築
    registry: dict[int, tuple[str, object]] = {}
    try:
        from piano_template import PIANO_FAMILY, make_piano_patch
        for p, (n, k) in PIANO_FAMILY.items():
            registry[p] = (n, lambda k=k: make_piano_patch(**k))
    except ImportError:
        pass
    try:
        from brass_template import BRASS_FAMILY, make_brass_patch
        for p, (n, k) in BRASS_FAMILY.items():
            registry[p] = (n, lambda k=k: make_brass_patch(**k))
    except ImportError:
        pass
    try:
        from organ_template import ORGAN_FAMILY, make_organ_patch, make_reed_patch
        for p, (n, k) in ORGAN_FAMILY.items():
            kc = dict(k); maker = kc.pop("maker", "organ")
            fn = make_reed_patch if maker == "reed" else make_organ_patch
            registry[p] = (n, lambda kc=kc, fn=fn: fn(**kc))
    except ImportError:
        pass

    if not registry:
        print("ERROR: テンプレートが見つかりません", file=sys.stderr)
        return 1

    progs = [args.prog] if args.prog is not None else sorted(registry.keys())
    print(f"テンプレート統合  ({len(progs)} 音色)  types={types}")
    for prog in progs:
        if prog not in registry:
            print(f"  WARNING: prog {prog} 未定義"); continue
        name, patch_fn = registry[prog]
        audition_phrases(json.dumps(patch_fn()),
                         f"{prog:03d}_{name}", types, out_base)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
