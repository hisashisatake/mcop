"""gm2_opz_map.py: TX81Z Yamaha Factory Bank A-D (opz2op505変換済み) から
GM2 Bank 0 (128プログラム) を組み立てる第一稿。

背景:
    手打ちPiano/Organ/Brassテンプレート (piano_template.py等、Algorithm4ノブ方式) を
    ユーザーが試聴し「安っぽい」と評価。一方でTX81Z実機音色 (opz2op505経由) は
    「OPNより厚みがある」と好評だったため、方針を転換しGM2の128プログラムに
    TX81Z Factory Bank A-D (計128プリセット、opz2op505 --split --wav で変換済み、
    op505/tools/opz2op505/private/bank{A,B,C,D}_out/) を名前ベースで対応づける。

    MAPPING の confidence は名前からの類推の強さ:
        high   = ほぼ同一の楽器名 (GrandPiano→Acoustic_Grand_Piano等)
        medium = 楽器ファミリーは一致するが細部は推測
        low    = 手がかりが弱い代替選定（優先的に試聴・差し替え検討すること）

使い方:
    python gm2_opz_map.py                       # バンク書き出し + 全128音色WAV
    python gm2_opz_map.py --only 0-7             # 指定プログラム範囲のみ
    python gm2_opz_map.py --report-only          # マッピング表の確認だけ (生成しない)
    python gm2_opz_map.py --bank private/hand_designed/gm2_bank0_opz.op505
"""

from __future__ import annotations

import argparse
import json
import sys
import wave as _wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.stdout.reconfigure(encoding="utf-8", errors="replace")

import patchlab  # noqa: E402

SR = 44100.0

OPZ_ROOT = Path(__file__).resolve().parent.parent.parent / "opz2op505" / "private"
BANK_DIRS = {b: OPZ_ROOT / f"bank{b}_out" for b in "ABCD"}

GM2_NAMES: list[str] = [
    # Piano (0-7)
    "Acoustic_Grand_Piano", "Bright_Acoustic_Piano", "Electric_Grand_Piano",
    "Honky_tonk_Piano", "Electric_Piano_1", "Electric_Piano_2",
    "Harpsichord", "Clavi",
    # Chromatic Percussion (8-15)
    "Celesta", "Glockenspiel", "Music_Box", "Vibraphone",
    "Marimba", "Xylophone", "Tubular_Bells", "Dulcimer",
    # Organ (16-23)
    "Drawbar_Organ", "Percussive_Organ", "Rock_Organ", "Church_Organ",
    "Reed_Organ", "Accordion", "Harmonica", "Tango_Accordion",
    # Guitar (24-31)
    "Acoustic_Guitar_nylon", "Acoustic_Guitar_steel",
    "Electric_Guitar_jazz", "Electric_Guitar_clean",
    "Electric_Guitar_muted", "Overdriven_Guitar",
    "Distortion_Guitar", "Guitar_harmonics",
    # Bass (32-39)
    "Acoustic_Bass", "Electric_Bass_finger", "Electric_Bass_pick",
    "Fretless_Bass", "Slap_Bass_1", "Slap_Bass_2",
    "Synth_Bass_1", "Synth_Bass_2",
    # Strings (40-47)
    "Violin", "Viola", "Cello", "Contrabass",
    "Tremolo_Strings", "Pizzicato_Strings", "Orchestral_Harp", "Timpani",
    # Ensemble (48-55)
    "String_Ensemble_1", "String_Ensemble_2",
    "SynthStrings_1", "SynthStrings_2",
    "Choir_Aahs", "Voice_Oohs", "Synth_Voice", "Orchestra_Hit",
    # Brass (56-63)
    "Trumpet", "Trombone", "Tuba", "Muted_Trumpet",
    "French_Horn", "Brass_Section", "SynthBrass_1", "SynthBrass_2",
    # Reed (64-71)
    "Soprano_Sax", "Alto_Sax", "Tenor_Sax", "Baritone_Sax",
    "Oboe", "English_Horn", "Bassoon", "Clarinet",
    # Pipe (72-79)
    "Piccolo", "Flute", "Recorder", "Pan_Flute",
    "Blown_Bottle", "Shakuhachi", "Whistle", "Ocarina",
    # Synth Lead (80-87)
    "Lead_1_square", "Lead_2_sawtooth", "Lead_3_calliope", "Lead_4_chiff",
    "Lead_5_charang", "Lead_6_voice", "Lead_7_fifths", "Lead_8_bass_lead",
    # Synth Pad (88-95)
    "Pad_1_new_age", "Pad_2_warm", "Pad_3_polysynth", "Pad_4_choir",
    "Pad_5_bowed", "Pad_6_metallic", "Pad_7_halo", "Pad_8_sweep",
    # Synth Effects (96-103)
    "FX_1_rain", "FX_2_soundtrack", "FX_3_crystal", "FX_4_atmosphere",
    "FX_5_brightness", "FX_6_goblins", "FX_7_echoes", "FX_8_sci_fi",
    # Ethnic (104-111)
    "Sitar", "Banjo", "Shamisen", "Koto",
    "Kalimba", "Bag_pipe", "Fiddle", "Shanai",
    # Percussive (112-119)
    "Tinkle_Bell", "Agogo", "Steel_Drums", "Woodblock",
    "Taiko_Drum", "Melodic_Tom", "Synth_Drum", "Reverse_Cymbal",
    # Sound Effects (120-127)
    "Guitar_Fret_Noise", "Breath_Noise", "Seashore", "Bird_Tweet",
    "Telephone_Ring", "Helicopter", "Applause", "Gunshot",
]

# program -> (bank, index, confidence)
MAPPING: dict[int, tuple[str, int, str]] = {
    # Piano
    0: ("A", 0, "high"), 1: ("A", 1, "medium"), 2: ("A", 4, "high"),
    3: ("A", 3, "high"), 4: ("A", 8, "medium"), 5: ("A", 9, "medium"),
    6: ("A", 28, "high"), 7: ("A", 26, "medium"),
    # Chromatic Percussion
    8: ("A", 29, "high"), 9: ("A", 30, "medium"), 10: ("D", 4, "low"),
    11: ("C", 27, "high"), 12: ("D", 3, "medium"), 13: ("C", 26, "high"),
    14: ("D", 25, "high"), 15: ("C", 7, "medium"),
    # Organ
    16: ("A", 17, "high"), 17: ("A", 16, "high"), 18: ("A", 23, "medium"),
    19: ("A", 22, "high"), 20: ("A", 18, "high"), 21: ("A", 20, "low"),
    22: ("B", 15, "high"), 23: ("B", 31, "low"),
    # Guitar
    24: ("C", 0, "high"), 25: ("C", 2, "medium"), 26: ("C", 4, "high"),
    27: ("C", 1, "medium"), 28: ("C", 3, "medium"), 29: ("C", 5, "medium"),
    30: ("C", 24, "low"), 31: ("D", 6, "low"),
    # Bass
    32: ("B", 16, "high"), 33: ("C", 8, "high"), 34: ("C", 11, "high"),
    35: ("C", 13, "high"), 36: ("C", 10, "medium"), 37: ("C", 14, "medium"),
    38: ("C", 12, "high"), 39: ("C", 15, "high"),
    # Strings
    40: ("B", 19, "high"), 41: ("B", 20, "medium"), 42: ("B", 17, "high"),
    43: ("B", 18, "medium"), 44: ("B", 24, "medium"), 45: ("B", 22, "high"),
    46: ("B", 23, "high"), 47: ("D", 22, "high"),
    # Ensemble
    48: ("B", 21, "medium"), 49: ("B", 29, "medium"), 50: ("B", 25, "high"),
    51: ("B", 27, "medium"), 52: ("D", 2, "medium"), 53: ("B", 26, "high"),
    54: ("D", 1, "medium"), 55: ("C", 30, "medium"),
    # Brass
    56: ("B", 0, "high"), 57: ("B", 6, "high"), 58: ("B", 7, "medium"),
    59: ("B", 2, "medium"), 60: ("B", 4, "high"), 61: ("B", 1, "high"),
    62: ("B", 3, "medium"), 63: ("B", 5, "high"),
    # Reed
    64: ("C", 18, "low"), 65: ("B", 9, "high"), 66: ("B", 8, "high"),
    67: ("C", 16, "low"), 68: ("B", 13, "high"), 69: ("C", 19, "low"),
    70: ("B", 12, "high"), 71: ("B", 14, "high"),
    # Pipe
    72: ("B", 30, "medium"), 73: ("B", 10, "high"), 74: ("C", 23, "medium"),
    75: ("B", 11, "high"), 76: ("D", 16, "low"), 77: ("D", 15, "low"),
    78: ("D", 19, "high"), 79: ("A", 21, "low"),
    # Synth Lead
    80: ("C", 20, "high"), 81: ("C", 24, "medium"), 82: ("C", 22, "medium"),
    83: ("C", 21, "medium"), 84: ("C", 5, "medium"), 85: ("D", 1, "medium"),
    86: ("C", 20, "medium"), 87: ("C", 9, "medium"),
    # Synth Pad
    88: ("D", 12, "low"), 89: ("C", 22, "medium"), 90: ("C", 20, "low"),
    91: ("D", 2, "medium"), 92: ("B", 17, "low"), 93: ("D", 3, "medium"),
    94: ("D", 5, "medium"), 95: ("D", 21, "medium"),
    # Synth Effects
    96: ("D", 9, "low"), 97: ("D", 14, "low"), 98: ("D", 2, "high"),
    99: ("D", 16, "low"), 100: ("A", 30, "medium"), 101: ("D", 6, "medium"),
    102: ("D", 8, "low"), 103: ("D", 20, "high"),
    # Ethnic
    104: ("C", 28, "high"), 105: ("C", 6, "high"), 106: ("C", 2, "low"),
    107: ("D", 7, "high"), 108: ("D", 3, "high"), 109: ("A", 21, "low"),
    110: ("B", 19, "medium"), 111: ("B", 14, "low"),
    # Percussive
    112: ("D", 25, "medium"), 113: ("A", 29, "low"), 114: ("D", 30, "medium"),
    115: ("C", 30, "low"), 116: ("D", 24, "medium"), 117: ("D", 31, "high"),
    118: ("D", 30, "medium"), 119: ("D", 28, "low"),
    # Sound Effects
    120: ("C", 3, "low"), 121: ("D", 0, "medium"), 122: ("D", 9, "medium"),
    123: ("D", 11, "high"), 124: ("D", 17, "medium"), 125: ("D", 13, "high"),
    126: ("D", 29, "low"), 127: ("D", 26, "high"),
}


def _load_source_patch(bank: str, index: int) -> tuple[str, dict]:
    """bank{A..D}_out/{index:03d}_*.op505 を読み、(元の音色名, patch dict) を返す。"""
    matches = sorted(BANK_DIRS[bank].glob(f"{index:03d}_*.op505"))
    if not matches:
        raise FileNotFoundError(f"bank{bank} index={index} が見つかりません")
    data = json.loads(matches[0].read_text(encoding="utf-8"))
    entry = (data.get("programs") or data.get("presets"))[0]
    return entry["name"], entry["patch"]


def _render(patch: dict, note: int, on: float = 2.0, release: float = 1.5) -> np.ndarray:
    freq = 440.0 * 2 ** ((note - 69) / 12.0)
    s = patchlab.render_patch(json.dumps(patch), freq, on, release, 100, SR)
    return np.asarray(s, dtype=np.float32)


def _write_wav(path: Path, x: np.ndarray) -> None:
    pcm = np.clip(x * 32767.0, -32768, 32767).astype("<i2")
    with _wave.open(str(path), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(int(SR))
        w.writeframes(pcm.tobytes())


def parse_range(s: str) -> list[int]:
    if "," in s:
        return [int(p) for p in s.split(",")]
    if "-" in s:
        a, b = s.split("-", 1)
        return list(range(int(a), int(b) + 1))
    return [int(s)]


def main() -> int:
    ap = argparse.ArgumentParser(description="TX81Z Factory Bank A-D から GM2 Bank 0 を組み立てる")
    ap.add_argument("--only", type=str, default=None, help="対象プログラム範囲 (例: 0-7 / 0,4,8)")
    ap.add_argument("--note", type=int, default=60, help="試聴ノート (既定 60=C4)")
    ap.add_argument("--out-dir", type=str, default=None, help="WAV 出力先")
    ap.add_argument("--bank", type=str, default=None, help=".op505 バンクファイル出力先")
    ap.add_argument("--report-only", action="store_true", help="マッピング表の表示のみ (生成しない)")
    args = ap.parse_args()

    progs = parse_range(args.only) if args.only else sorted(MAPPING.keys())

    base = Path(__file__).resolve().parent.parent / "private"
    out_dir = Path(args.out_dir) if args.out_dir else base / "gm2_bank0_opz_wav"

    print(f"GM2 Bank0 (OPZ由来) マッピング: {len(progs)} プログラム  note={args.note}")
    print(f"{'prog':>4} {'GM2名':<24} {'src':<20} {'conf':<6}")

    presets = []
    low_count = 0
    for prog in progs:
        if prog not in MAPPING:
            print(f"  WARNING: program {prog} はマッピング未定義")
            continue
        bank, idx, conf = MAPPING[prog]
        src_name, patch = _load_source_patch(bank, idx)
        gm2_name = GM2_NAMES[prog]
        flag = " !LOW" if conf == "low" else ""
        if conf == "low":
            low_count += 1
        print(f"{prog:4d} {gm2_name:<24} {bank}{idx:03d}_{src_name:<14} {conf:<6}{flag}")
        presets.append({"program": prog, "name": gm2_name, "patch": patch})

        if not args.report_only:
            out_dir.mkdir(parents=True, exist_ok=True)
            x = _render(patch, args.note)
            _write_wav(out_dir / f"{prog:03d}_{gm2_name}.wav", x)

    print(f"\nlow confidence: {low_count}/{len(progs)} 件 (優先的に試聴・差し替え検討)")

    if not args.report_only:
        print(f"WAV 出力: {out_dir}")
        if args.bank:
            bank_path = Path(args.bank)
            bank_path.parent.mkdir(parents=True, exist_ok=True)
            bank_path.write_text(
                json.dumps({"bank": 0, "presets": presets}, indent=2), encoding="utf-8")
            print(f"バンク書き出し: {bank_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
