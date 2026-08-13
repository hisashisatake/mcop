"""ピアノ族テンプレート: GM prog 0-7 (Piano Family) の叩き台。

骨格（固定）:
    Algorithm 4 / 2ペア独立 (OP0→OP1) + (OP2→OP3)
    ref.38x6 #13 (mucom_acoustic_piano) をアコピの基準として設計

FM ピアノの設計原則（ref から読み取ったもの）:
    - モジュレーター d1r=0: D1R フェーズをスキップ → D2R (mod_decay) で緩やかに倍音フェード
    - feedback=252: アコピの倍音感・明るさを出す。アタックの位相不連続もマスク
    - mod ksr=255: 高音域ほどモジュレーターが速く消える（倍音が鍵盤上位で自然に薄れる）
    - car ksr=170: キャリアも高音域ほど短く（弦の長さ・テンション感）
    - DT1 デチューン: ペア間を微妙にズラしてコーラス・奥行きを出す

ノブ（族内で動かす高レベルパラメーター）:
    brightness   : モジュレーター TL（小=明るい変調深め / 大=暗い）
    mod_d1r      : モジュレーター D1R（0=D1Rスキップ・アコピ向け / >0=EP・打鍵系）
    mod_decay    : モジュレーター D2R（倍音がフェードする速さ）
    mod_d1l      : モジュレーター D1L（変調レベルの遷移点）
    body_d1r     : キャリア D1R（ボディの第1減衰）
    body_d2r     : キャリア D2R（ボディの第2・長期フェード）
    body_d1l     : キャリア D1L（第1→第2減衰の切替レベル）
    feedback     : チャンネルフィードバック（倍音・明るさ）
    rr           : リリース
    mod_ksr      : モジュレーター KSR（高音短縮率）
    car_ksr      : キャリア KSR
    dt1_spread   : DT1 デチューン幅（コーラス感）
    mul_mod1     : OP0 MUL（ペア1モジュレーター）
    mul_mod2     : OP2 MUL（ペア2モジュレーター）
    carrier_wf   : キャリア波形（0=sine / 8=saw / 16=sq50）

使い方:
    python piano_template.py            # 全8音色を WAV 書き出し
    python piano_template.py --only 0,4 # 指定 program のみ
    python piano_template.py --bank private/hand_designed/piano_family.op505
    python piano_template.py --note 64  # 別の試聴ノート
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

import op505_patch  # noqa: E402
import patchlab  # noqa: E402

SR = 44100.0


def make_piano_patch(
    brightness: int = 190,   # モジュレーター TL（小=明るい）
    mod_d1r: int = 0,        # モジュレーター D1R（0=D1Rスキップ / >0=打鍵感）
    mod_decay: int = 140,    # モジュレーター D2R（倍音フェード速度）
    mod_d1l: int = 187,      # モジュレーター D1L
    body_d1r: int = 89,      # キャリア D1R
    body_d2r: int = 89,      # キャリア D2R
    body_d1l: int = 204,     # キャリア D1L
    feedback: int = 252,     # フィードバック
    rr: int = 81,            # リリース
    mod_ksr: int = 255,      # モジュレーター KSR
    car_ksr: int = 170,      # キャリア KSR
    dt1_spread: int = 6,     # DT1 デチューン幅（コーラス感）
    mul_mod1: int = 1,       # OP0 MUL（ペア1モジュレーター）
    mul_mod2: int = 3,       # OP2 MUL（ペア2モジュレーター）
    mod_wf: int = 0,         # モジュレーター波形（0=sine / 1=sin² / 8=saw / 16=sq50）
    carrier_wf: int = 0,     # キャリア波形（0=sine / 24=tri / 8=saw）
    algorithm: int = 4,      # 4=2ペア独立（ピアノ系）/ 0=直列チェーン（ギター系）
) -> dict:
    """高レベルノブからピアノパッチ dict を生成する。"""

    ch = op505_patch.make_channel(algorithm=algorithm, feedback=feedback)

    if algorithm == 0:
        # 直列チェーン OP0→OP1→OP2→OP3（ギター系 ref 準拠 TL）
        # body_d1r/d2r/d1l/rr で全 OP の減衰を共通制御
        def chain_op(tl: int, mul: int, wf: int) -> dict:
            return op505_patch.make_op(tl=tl, ar=255,
                        d1r=body_d1r, d2r=body_d2r, d1l=body_d1l, rr=rr,
                        mul=mul, dt1=128, ksr=car_ksr, am_enable=False,
                        velocity_sensitivity=0, waveform=wf, op_fine_tune=128)
        ops = [
            chain_op(222, 1, mod_wf),     # OP0: entry（feedback対象）
            chain_op(202, 0, mod_wf),     # OP1
            chain_op(198, 2, mod_wf),     # OP2
            chain_op(250, 0, carrier_wf), # OP3: carrier（出力。単一キャリアのためフル出力寄り）
        ]
        return {"operators": ops, "channel": ch}

    # algorithm 4: 2ペア独立（既存コード）
    def mod_op(mul: int, tl: int, dt1: int) -> dict:
        return op505_patch.make_op(tl=tl, ar=255,
                    d1r=mod_d1r, d2r=mod_decay, d1l=mod_d1l, rr=rr,
                    mul=mul, dt1=dt1, ksr=mod_ksr, am_enable=False,
                    velocity_sensitivity=0, waveform=mod_wf, op_fine_tune=128)

    def car_op(tl: int, dt1: int) -> dict:
        return op505_patch.make_op(tl=tl, ar=255,
                    d1r=body_d1r, d2r=body_d2r, d1l=body_d1l, rr=rr,
                    mul=1, dt1=dt1, ksr=car_ksr, am_enable=False,
                    velocity_sensitivity=0, waveform=carrier_wf, op_fine_tune=128)

    dt = dt1_spread
    # car_op TLは2キャリア合算（クリップ回避のため234/254から-14: 220/240）
    ops = [
        mod_op(mul_mod1, brightness,     128 + dt),
        car_op(220,                      128),
        mod_op(mul_mod2, brightness + 8, 128 - dt // 2),
        car_op(240,                      128 + dt // 2),
    ]
    return {"operators": ops, "channel": ch}


# ── GM ピアノ族 ノブ表 ─────────────────────────────────────────────────────────
# prog 0: ref.38x6 #13 (mucom_acoustic_piano) をほぼそのまま使用
# prog 4: ref.38x6 #27 (mucom_epiano) を参考に mul_mod1=15 + mod_d1r>0 でベルトーン
PIANO_FAMILY: dict[int, tuple[str, dict]] = {
    0: ("Acoustic_Grand_Piano", dict(
        brightness=190, mod_d1r=0,   mod_decay=140, mod_d1l=187,
        body_d1r=89,  body_d2r=89,  body_d1l=204,
        feedback=180, rr=81,  mod_ksr=255, car_ksr=100, dt1_spread=6,
        mul_mod1=1,  mul_mod2=3)),
    1: ("Bright_Acoustic_Piano", dict(
        brightness=190, mod_d1r=0,   mod_decay=140, mod_d1l=187,
        body_d1r=89,  body_d2r=89,  body_d1l=204,
        feedback=180, rr=81,  mod_ksr=255, car_ksr=100, dt1_spread=6,
        mul_mod1=1,  mul_mod2=3,  mod_wf=1, carrier_wf=24)),
    2: ("Electric_Grand_Piano", dict(
        brightness=164, mod_d1r=60,  mod_decay=134, mod_d1l=196,
        body_d1r=121, body_d2r=70,  body_d1l=225,
        feedback=180, rr=112, mod_ksr=128, car_ksr=92, dt1_spread=7,
        mul_mod1=5,  mul_mod2=2)),
    3: ("Honky_tonk_Piano", dict(
        brightness=190, mod_d1r=0,   mod_decay=140, mod_d1l=187,
        body_d1r=89,  body_d2r=89,  body_d1l=204,
        feedback=180, rr=81,  mod_ksr=255, car_ksr=100, dt1_spread=14,
        mul_mod1=1,  mul_mod2=3,  mod_wf=16)),
    4: ("Electric_Piano_1", dict(
        brightness=155, mod_d1r=119, mod_decay=128, mod_d1l=206,
        body_d1r=153, body_d2r=51,  body_d1l=247,
        feedback=180, rr=144, mod_ksr=0,  car_ksr=85, dt1_spread=8,
        mul_mod1=15, mul_mod2=1)),
    5: ("Electric_Piano_2", dict(
        brightness=178, mod_d1r=130, mod_decay=110, mod_d1l=206,
        body_d1r=153, body_d2r=51,  body_d1l=247,
        feedback=180, rr=144, mod_ksr=0,  car_ksr=85, dt1_spread=8,
        mul_mod1=7,  mul_mod2=1,  mod_wf=19)),
    6: ("Harpsichord", dict(
        algorithm=0,
        body_d1r=70,  body_d2r=40,  body_d1l=150,
        feedback=40,  rr=130, car_ksr=100, mod_wf=24)),
    7: ("Clavinet", dict(
        algorithm=0,
        body_d1r=70,  body_d2r=40,  body_d1l=150,
        feedback=40,  rr=230, car_ksr=100, mod_wf=24)),
}


def _render(patch: dict, note: int, on: float = 2.0, release: float = 3.0) -> np.ndarray:
    freq = 440.0 * 2 ** ((note - 69) / 12.0)
    s = patchlab.render_patch(json.dumps(patch), freq, on, release, 100, SR)
    return np.asarray(s, dtype=np.float32)


def _write_wav(path: Path, x: np.ndarray) -> None:
    pcm = np.clip(x * 32767.0, -32768, 32767).astype("<i2")
    with _wave.open(str(path), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(int(SR))
        w.writeframes(pcm.tobytes())


def main() -> int:
    ap = argparse.ArgumentParser(description="ピアノ族テンプレート展開")
    ap.add_argument("--only", type=str, default=None,
                    help="生成する program 番号（例: 0,4）。省略時は全8音色")
    ap.add_argument("--note", type=int, default=60, help="試聴ノート（デフォルト 60=C4）")
    ap.add_argument("--out-dir", type=str, default=None,
                    help="WAV 出力先（デフォルト private/piano_family_wav）")
    ap.add_argument("--bank", type=str, default=None,
                    help="指定すると .op505 バンクファイルも書き出す")
    args = ap.parse_args()

    progs = ([int(p) for p in args.only.split(",")] if args.only
             else list(PIANO_FAMILY.keys()))

    base = Path(__file__).resolve().parent.parent / "private"
    out_dir = Path(args.out_dir) if args.out_dir else base / "piano_family_wav"
    out_dir.mkdir(parents=True, exist_ok=True)

    presets = []
    print(f"ピアノ族展開: {len(progs)} 音色  note={args.note}")
    for prog in progs:
        if prog not in PIANO_FAMILY:
            print(f"  WARNING: program {prog} はピアノ族にありません")
            continue
        name, knobs = PIANO_FAMILY[prog]
        patch = make_piano_patch(**knobs)
        x = _render(patch, args.note)
        wav_path = out_dir / f"{prog:03d}_{name}.wav"
        _write_wav(wav_path, x)
        presets.append({"program": prog, "name": name, "patch": patch})
        alg = knobs.get('algorithm', 4)
        if alg == 0:
            print(f"  [{prog:3d}] {name:<22}  alg=0  "
                  f"fb={knobs['feedback']:3d}  "
                  f"body({knobs['body_d1r']},{knobs['body_d2r']})  rr={knobs['rr']}")
        else:
            print(f"  [{prog:3d}] {name:<22}  "
                  f"fb={knobs['feedback']:3d}  br={knobs['brightness']:3d}  "
                  f"mod_d1r={knobs.get('mod_d1r',0):3d}  mod_decay={knobs.get('mod_decay',0):3d}  "
                  f"body({knobs['body_d1r']},{knobs['body_d2r']})  "
                  f"mul={knobs['mul_mod1']}/{knobs['mul_mod2']}")

    print(f"\nWAV 出力: {out_dir}")

    if args.bank:
        bank_path = Path(args.bank)
        bank_path.parent.mkdir(parents=True, exist_ok=True)
        bank_path.write_text(
            json.dumps({"bank": 0, "presets": presets}, indent=2), encoding="utf-8")
        print(f"バンク書き出し: {bank_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
