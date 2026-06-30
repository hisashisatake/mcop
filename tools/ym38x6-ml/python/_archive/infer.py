"""infer.py: .wav ファイルから 38x6 FM パラメーターを推論する。

単音 .wav → log-mel 特徴量 → 逆算モデル → FMパラメーターベクトル → パッチ JSON。
バッチモード (--wav-dir) では複数ファイルを処理し .38x6 バンクを生成する。

使い方（単一ファイル）:
    python infer.py <wav_file> [--model <pt>] [--out <json>]

使い方（バッチ: GM2 Bank0 生成）:
    python infer.py --wav-dir <dir> [--model <pt>] [--out <bank.38x6>]
    python infer.py --wav-dir ../private/gm2_wav --out ../private/bank0.38x6

バッチモードでは <wav_dir> 内の 000_*.wav 〜 127_*.wav を番号順に読み込み、
program 番号はファイル名先頭の3桁数字から決定する。
"""

from __future__ import annotations

import argparse
import json
import sys
import wave
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import features as ft  # noqa: E402
import param_space as ps  # noqa: E402
from model import InverseNet  # noqa: E402

_DEFAULT_MODEL = Path(__file__).resolve().parent.parent / "private" / "model_f64.pt"


# ─── モデル読み込み ──────────────────────────────────────────────────────

def load_model(model_path: Path):
    ckpt = torch.load(str(model_path), map_location="cpu", weights_only=False)
    model = InverseNet(out_dim=ckpt["out_dim"], n_mels=ckpt["n_mels"], n_frames=ckpt["n_frames"])
    model.load_state_dict(ckpt["model_state"])
    model.eval()
    return model, ckpt


# ─── WAV 読み込み ────────────────────────────────────────────────────────

def load_wav_mono(path: Path) -> tuple[np.ndarray, int]:
    """WAV をモノラル float32 に変換して返す。"""
    with wave.open(str(path)) as w:
        sr = w.getframerate()
        n = w.getnframes()
        raw = w.readframes(n)
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


# ─── 推論コア ────────────────────────────────────────────────────────────

def infer_patch(samples: np.ndarray, sr: int, model, ckpt: dict) -> dict:
    """モノラルサンプル列 → Ym38x6Patch 相当の dict。"""
    n_mels = ckpt["n_mels"]
    n_frames = ckpt["n_frames"]
    mean = ckpt["mean"]  # [n_mels, 1] ndarray
    std = ckpt["std"]

    mel = ft.log_mel(samples, sr=float(sr), n_mels=n_mels, n_frames=n_frames)
    x = ((mel - mean) / std)[None, None, :, :]   # [1, 1, n_mels, n_frames]
    with torch.no_grad():
        vec = model(torch.from_numpy(x.astype(np.float32)))[0].numpy()

    return ps.vector_to_patch(vec)


# ─── .38x6 バンクJSON 生成 ───────────────────────────────────────────────

def make_bank_json(entries: list[dict], bank: int = 0) -> str:
    """(program, name, patch_dict) のリスト → .38x6 presets 形式 JSON 文字列。

    spec-sound.md の形式:
      { "bank": N, "presets": [ { "program": N, "name": "...", "patch": {...} }, ... ] }
    """
    presets = [
        {"program": e["program"], "name": e["name"], "patch": e["patch"]}
        for e in entries
    ]
    return json.dumps({"bank": bank, "presets": presets}, indent=2)


# ─── バッチ処理（wav_dir → .38x6）───────────────────────────────────────

def _prog_from_filename(fname: str) -> int | None:
    """'000_Acoustic_Grand_Piano.wav' → 0 のようにファイル名先頭3桁をプログラム番号に変換。"""
    stem = Path(fname).stem
    if len(stem) >= 3 and stem[:3].isdigit():
        return int(stem[:3])
    return None


def run_batch(wav_dir: Path, model, ckpt: dict, out_path: Path, bank: int) -> int:
    wavs = sorted(wav_dir.glob("*.wav"))
    if not wavs:
        print(f"ERROR: {wav_dir} に .wav が見つかりません", file=sys.stderr)
        return 1

    entries: list[dict] = []
    failed: list[str] = []
    for wav_path in wavs:
        prog = _prog_from_filename(wav_path.name)
        if prog is None:
            print(f"  SKIP {wav_path.name} (ファイル名から program 番号を取得できません)")
            continue
        name = Path(wav_path.stem[4:]).stem if len(wav_path.stem) > 4 else wav_path.stem

        print(f"  [{prog:3d}] {name:<30} ... ", end="", flush=True)
        try:
            samples, sr = load_wav_mono(wav_path)
            patch = infer_patch(samples, sr, model, ckpt)
            entries.append({"program": prog, "name": name, "patch": patch})
            print("OK")
        except Exception as e:
            print(f"FAIL: {e}")
            failed.append(wav_path.name)

    if not entries:
        print("ERROR: 有効なエントリーが0件です", file=sys.stderr)
        return 1

    bank_json = make_bank_json(entries, bank=bank)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(bank_json, encoding="utf-8")
    print(f"\n{len(entries)} パッチ → {out_path}")
    if failed:
        print(f"失敗: {failed}")
    return 0 if not failed else 1


# ─── 単一ファイル処理 ────────────────────────────────────────────────────

def run_single(wav_path: Path, model, ckpt: dict, out_path: Path | None) -> int:
    samples, sr = load_wav_mono(wav_path)
    patch = infer_patch(samples, sr, model, ckpt)
    patch_json = json.dumps(patch, indent=2)
    if out_path:
        out_path.write_text(patch_json, encoding="utf-8")
        print(f"→ {out_path}")
    else:
        print(patch_json)
    return 0


# ─── CLI ─────────────────────────────────────────────────────────────────

def main() -> int:
    ap = argparse.ArgumentParser(description="38x6 FM パラメーター逆算推論")
    ap.add_argument("wav", nargs="?", help="単一 .wav ファイル")
    ap.add_argument("--wav-dir", help="バッチモード: .wav ディレクトリ")
    ap.add_argument("--model", default=None, help=f"モデルチェックポイント (既定: {_DEFAULT_MODEL.name})")
    ap.add_argument("--out", default=None, help="出力先 (単一: .json / バッチ: .38x6)")
    ap.add_argument("--bank", type=int, default=0, help="バッチ出力の bank 番号 (既定: 0)")
    args = ap.parse_args()

    if not args.wav and not args.wav_dir:
        ap.print_help()
        return 1

    model_path = Path(args.model) if args.model else _DEFAULT_MODEL
    if not model_path.exists():
        print(f"ERROR: モデルが見つかりません: {model_path}", file=sys.stderr)
        return 1

    print(f"モデル: {model_path.name}")
    model, ckpt = load_model(model_path)

    if args.wav_dir:
        wav_dir = Path(args.wav_dir)
        default_out = wav_dir.parent / (wav_dir.name + ".38x6")
        out_path = Path(args.out) if args.out else default_out
        print(f"バッチモード: {wav_dir}  →  {out_path}")
        return run_batch(wav_dir, model, ckpt, out_path, args.bank)
    else:
        wav_path = Path(args.wav)
        out_path = Path(args.out) if args.out else None
        return run_single(wav_path, model, ckpt, out_path)


if __name__ == "__main__":
    raise SystemExit(main())
