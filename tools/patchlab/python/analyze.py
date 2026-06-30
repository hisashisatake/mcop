"""Phase 0: GM2 WAV 群への知覚記述子分析ツール。

10 軸の記述子を計算してターミナルテーブルおよび CSV に出力する。
これは A-by-S の損失項（Phase A/B）で使う特徴抽出器の妥当性検証も兼ねる。

使い方:
    # GM2 全音色を分析
    python analyze.py --wav-dir private/gm2_wav --note 60

    # 範囲を絞って分析
    python analyze.py --wav-dir private/gm2_wav --prog-range 0-15,56-63 --note 60

    # 単一 WAV を分析
    python analyze.py --wav private/gm2_wav/056_Trumpet.wav --note 60

    # 金属度でソートして CSV に書き出し
    python analyze.py --wav-dir private/gm2_wav --sort metallic --csv private/gm2_descriptors.csv
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np

sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # PowerShell で日本語を表示
sys.path.insert(0, str(Path(__file__).resolve().parent))

import features as ft   # noqa: E402

# ── 出力テーブルの列定義 ──────────────────────────────────────────────────
# key: perceptual_descriptors の dict キー
# short: テーブルヘッダー（半角4文字以内）
# label: CSV ヘッダー / --sort 用名称（= key）
DESCRIPTORS: list[tuple[str, str]] = [
    ("brightness",       "Br"),
    ("warmth",           "Wa"),
    ("metallic",         "Me"),
    ("roughness",        "Ro"),
    ("distortion",       "Di"),
    ("odd_even",         "OE"),
    ("noise",            "No"),
    ("attack",           "Atk"),
    ("decay",            "Dcy"),
    ("brightness_decay", "ΔBr"),
]
KEYS   = [k for k, _ in DESCRIPTORS]
SHORTS = [s for _, s in DESCRIPTORS]


def _midi_to_freq(note: int) -> float:
    return 440.0 * 2 ** ((note - 69) / 12.0)


def _prog_from_filename(fname: str) -> int | None:
    stem = Path(fname).stem
    if len(stem) >= 3 and stem[:3].isdigit():
        return int(stem[:3])
    return None


def _parse_prog_set(s: str) -> set[int]:
    result: set[int] = set()
    for part in s.split(","):
        part = part.strip()
        if "-" in part:
            a, b = part.split("-", 1)
            result.update(range(int(a), int(b) + 1))
        else:
            result.add(int(part))
    return result


def _load_wav_mono(path: Path) -> tuple[np.ndarray, int]:
    import wave as _wave
    with _wave.open(str(path)) as w:
        sr = w.getframerate()
        raw = w.readframes(w.getnframes())
        ch = w.getnchannels()
        sw = w.getsampwidth()
    if sw == 2:
        x = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif sw == 4:
        x = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648.0
    else:
        raise ValueError(f"未対応サンプル幅: {sw}")
    if ch == 2:
        x = x.reshape(-1, 2).mean(axis=1)
    return x, sr


def main() -> int:
    ap = argparse.ArgumentParser(
        description="38x6 知覚記述子分析 (Phase 0) — 7+3 軸でGM2/WAVを可視化")
    src = ap.add_mutually_exclusive_group()
    src.add_argument("--wav-dir", type=str, default=None,
                     help="WAV ディレクトリ（000_Name.wav 形式）")
    src.add_argument("--wav", type=str, default=None,
                     help="単一 WAV ファイル")
    ap.add_argument("--note", type=int, default=60,
                    help="基音 MIDI ノート番号（デフォルト 60 = C4）")
    ap.add_argument("--freq", type=float, default=None,
                    help="基音周波数 Hz（--note を上書き）")
    ap.add_argument("--n-harmonics", type=int, default=16,
                    help="倍音解析の最大倍音次数（デフォルト 16）")
    ap.add_argument("--prog-range", type=str, default=None,
                    help="処理するプログラム番号（例: 0-15,56-63）")
    ap.add_argument("--sort", type=str, default=None,
                    choices=KEYS,
                    help="ソートする記述子キー（降順）")
    ap.add_argument("--csv", type=str, default=None,
                    help="CSV 出力先パス（省略時は標準出力のみ）")
    ap.add_argument("--pre-delay", type=float, default=0.3,
                    help="WAV 先頭のスキップ秒数（録音プリディレイ除去、デフォルト 0.3s）")
    args = ap.parse_args()

    freq = args.freq if args.freq else _midi_to_freq(args.note)
    print(f"基音: note={args.note}  freq={freq:.2f} Hz  n_harmonics={args.n_harmonics}")

    # ── WAV 収集 ────────────────────────────────────────────────────────
    wavs: list[Path] = []
    if args.wav:
        wavs.append(Path(args.wav))
    elif args.wav_dir:
        wav_dir = Path(args.wav_dir)
        if not wav_dir.is_dir():
            print(f"ERROR: {wav_dir} が見つかりません", file=sys.stderr)
            return 1
        prog_set = _parse_prog_set(args.prog_range) if args.prog_range else None
        for p in sorted(wav_dir.glob("*.wav")):
            if prog_set is not None:
                prog = _prog_from_filename(p.name)
                if prog is None or prog not in prog_set:
                    continue
            wavs.append(p)
    else:
        print("ERROR: --wav か --wav-dir を指定してください", file=sys.stderr)
        return 1

    if not wavs:
        print("ERROR: WAV ファイルが見つかりません", file=sys.stderr)
        return 1

    # ── 分析 ───────────────────────────────────────────────────────────
    rows: list[dict] = []
    skip_sec = args.pre_delay

    for wav_path in wavs:
        try:
            samples, sr = _load_wav_mono(wav_path)
            trim = int(skip_sec * sr)
            if trim > 0 and trim < len(samples):
                samples = samples[trim:]
            if ft.is_silent(samples):
                print(f"  スキップ（無音）: {wav_path.name}")
                continue
            desc = ft.perceptual_descriptors(
                samples, freq, sr=float(sr), n_harmonics=args.n_harmonics)
            desc["name"] = wav_path.stem
            rows.append(desc)
        except Exception as e:
            print(f"  FAIL {wav_path.name}: {e}", file=sys.stderr)

    if not rows:
        print("ERROR: 有効な WAV が 0 件", file=sys.stderr)
        return 1

    # ── ソート ──────────────────────────────────────────────────────────
    if args.sort:
        rows.sort(key=lambda r: r.get(args.sort, 0.0), reverse=True)

    # ── テーブル出力 ────────────────────────────────────────────────────
    name_w = max(max(len(r["name"]) for r in rows), 8)
    col_w = 5

    header = f"{'音色':<{name_w}}" + "  " + "  ".join(f"{s:>{col_w}}" for s in SHORTS)
    print()
    print(header)
    print("-" * len(header))
    for r in rows:
        vals = "  ".join(f"{r.get(k, 0.0):>{col_w}.3f}" for k in KEYS)
        print(f"{r['name']:<{name_w}}  {vals}")

    # ── CSV 出力 ────────────────────────────────────────────────────────
    if args.csv:
        import csv
        out_path = Path(args.csv)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        with open(out_path, "w", newline="", encoding="cp932") as f:
            writer = csv.DictWriter(f, fieldnames=["name"] + KEYS)
            writer.writeheader()
            for r in rows:
                writer.writerow({k: (f"{r[k]:.4f}" if k in KEYS else r.get(k, "")) for k in ["name"] + KEYS})
        print(f"\nCSV 出力: {out_path}")

    # ── 凡例 ────────────────────────────────────────────────────────────
    print()
    print("凡例:")
    for k, s in DESCRIPTORS:
        descs = {
            "brightness":       "明るさ（スペクトル重心）",
            "warmth":           "暖かみ（1-3倍音エネルギー比）",
            "metallic":         "金属度（非整数倍音エネルギー比）",
            "roughness":        "ラフネス（低域帯の倍音間エネルギー比）",
            "distortion":       "歪み（8倍音以降エネルギー比）",
            "odd_even":         "奇偶比（奇数倍音エネルギー比; 1=矩形波）",
            "noise":            "ノイズ性（スペクトル平坦度）",
            "attack":           "アタック速度（0=遅い, 1=瞬時）",
            "decay":            "減衰速度（0=持続, 1=急減衰）",
            "brightness_decay": "明暗変化（前半重心 - 後半重心）",
        }
        print(f"  {s:>4} ({k:<17}): {descs.get(k, '')}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
