"""オルガン族テンプレート: GM prog 16-23 (Organ Family)

骨格（固定）:
    Algorithm 7 / 全4OP独立キャリア（加算合成・FM変調なし）
    各OPが独立した倍音を担当（Hammond ドローバー方式）

ノブ（族内で動かす高レベルパラメーター）:
    tl0..3     : 各倍音の音量（255=最大 / 0=消音）
    mul0..3    : 倍音ポジション（MUL: 1=基音/2=1oct/3=5度+1oct/4=2oct）
    feedback   : 歪み量（0=クリーン / 大=ロック系の歪み）
    pms        : ピッチLFO感度（0=ビブラートOFF / 大=深いビブラート）
    vib_depth  : ビブラート深さ（tone_lfo_pmd）
    vib_rate   : ビブラート速度（tone_lfo_freq: 0≈3Hz / 255≈80Hz、指数）
    d1r        : 全OP共通の減衰（0=無限サステイン）
    perc_d1r   : OP1のみ速減衰（Hammond Percussion: 4'倍音の打鍵感）
    perc_d1l   : OP1のD1L（perc_d1r>0時。0=消音まで減衰）
    rr         : リリース
    waveform   : 全OP共通波形（0=sine / 16=sq50 / 24=tri）
    dt1_spread : ペアデチューン幅（musette用）
                 OP0,2→128-spread、OP1,3→128+spread で逆方向デチューン

使い方:
    python organ_template.py              # 全8音色
    python organ_template.py --only 16,18 # 指定programのみ
    python organ_template.py --bank private/hand_designed/organ_family.38x6
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

import ym38x6_ml  # noqa: E402

SR = 44100.0


def make_organ_patch(
    tl0: int = 230,     # OP0 音量（255=最大・0=消音）
    tl1: int = 220,     # OP1 音量
    tl2: int = 200,     # OP2 音量
    tl3: int = 185,     # OP3 音量
    mul0: int = 1,      # OP0 MUL（1=基音）
    mul1: int = 2,      # OP1 MUL（2=1オクターブ上）
    mul2: int = 3,      # OP2 MUL（3=5度+1oct）
    mul3: int = 4,      # OP3 MUL（4=2オクターブ上）
    feedback: int = 30,    # 歪み量
    pms: int = 80,         # ピッチLFO感度（0=ビブラートOFF）
    vib_depth: int = 60,   # ビブラート深さ（tone_lfo_pmd）
    vib_rate: int = 50,    # ビブラート速度（tone_lfo_freq: ≈50→5Hz、Leslie風）
    ar: int = 255,         # アタック（255=瞬時 / 小=ふわっとスウェル）
    d1r: int = 0,          # 全OP共通減衰（0=無限サステイン）
    d1l: int = 255,        # 全OP共通サステインレベル（255=完全サステイン / 0=消音まで減衰）
    perc_d1r: int = 0,     # OP1のみ速減衰（Hammond Percussion エミュ）
    perc_d1l: int = 0,     # OP1のD1L（perc_d1r>0時）
    rr: int = 80,          # リリース
    waveform: int = 0,     # 全OP共通波形（0=sine / 16=sq50 / 24=tri）
    dt1_spread: int = 0,   # ペアデチューン幅（musette用）
) -> dict:
    """高レベルノブからオルガンパッチ dict を生成する。Algorithm 7 固定。"""

    # OP0,2 → 128-spread、OP1,3 → 128+spread でペア内逆方向デチューン
    dt = [128 - dt1_spread, 128 + dt1_spread, 128 - dt1_spread, 128 + dt1_spread]

    def op_c(tl: int, mul: int, dt1: int, d1r_: int, d1l_: int) -> dict:
        return dict(
            tl=tl, ar=ar, d1r=d1r_, d2r=0, d1l=d1l_, rr=rr,
            mul=mul, dt1=dt1, ksr=0, am_enable=False,
            velocity_sensitivity=0, waveform=waveform, op_fine_tune=128,
        )

    op1_d1r = perc_d1r if perc_d1r else d1r
    op1_d1l = perc_d1l if perc_d1r else 255

    ops = [
        op_c(tl0, mul0, dt[0], d1r,     d1l),
        op_c(tl1, mul1, dt[1], op1_d1r, op1_d1l if perc_d1r else d1l),
        op_c(tl2, mul2, dt[2], d1r,     d1l),
        op_c(tl3, mul3, dt[3], d1r,     d1l),
    ]

    ch = dict(
        algorithm=7, feedback=feedback,
        tone_lfo_freq=vib_rate, tone_lfo_pmd=vib_depth, tone_lfo_amd=0, tone_lfo_delay=0,
        pms=pms, ams=0,
        filter_cutoff=255, filter_resonance=0, filter_type=0,
        filter_self_oscillation=False,
        filter_eg_attack=0, filter_eg_decay=0, filter_eg_sustain=0,
        filter_eg_release=0, filter_eg_depth=0,
    )
    return {"operators": ops, "channel": ch}


def make_reed_patch(
    mod_mul: int = 10,      # モジュレーターMUL（ref.50 OP0/OP1 の MUL=10）
    mod_tl0: int = 174,     # OP0 TL（ref.50 準拠）
    mod_tl1: int = 152,     # OP1 TL（ref.50 準拠）
    feedback: int = 252,    # FB（大=buzz/リード感）
    dt1_spread: int = 7,    # モジュレーターペアのデチューン幅（大=musette感）
    ar: int = 200,          # アタック（ref.50 準拠）
    d1r: int = 43,          # OP0 減衰速度（OP1 は +25 で固定）
    d1l0: int = 102,        # OP0 サステインレベル（小=打鍵感強い）
    rr: int = 161,          # リリース
    mod_wf: int = 0,        # モジュレーター波形（0=sine / 24=tri / 16=sq50）
    car_wf: int = 0,        # キャリア波形（OP2/OP3。0=sine / 24=tri / 8=saw）
) -> dict:
    """ref.38x6 #50 (mucom_harmonica) ベース。
    Algorithm 0 直列チェーン + 高 FB + 高 MUL モジュレーターペアでリード系を合成。
    OP0/OP1: 逆方向デチューンしたモジュレーターペア（musette / buzz の源）
    OP2: MUL=1 中間変調段（ref 準拠固定）
    OP3: MUL=2 出力キャリア（ref 準拠固定）
    """
    dt_lo = 128 - dt1_spread
    dt_hi = 128 + dt1_spread

    ops = [
        dict(tl=mod_tl0, ar=ar,  d1r=d1r,      d2r=0, d1l=d1l0, rr=rr,
             mul=mod_mul, dt1=dt_lo, ksr=0, am_enable=False,
             velocity_sensitivity=0, waveform=mod_wf, op_fine_tune=128),
        dict(tl=mod_tl1, ar=ar,  d1r=d1r + 25, d2r=0, d1l=255,  rr=rr,
             mul=mod_mul, dt1=dt_hi, ksr=0, am_enable=False,
             velocity_sensitivity=0, waveform=mod_wf, op_fine_tune=128),
        dict(tl=196,     ar=166, d1r=17,        d2r=0, d1l=255,  rr=rr,
             mul=1,       dt1=128, ksr=0, am_enable=False,
             velocity_sensitivity=0, waveform=car_wf, op_fine_tune=128),
        dict(tl=254,     ar=149, d1r=68,        d2r=0, d1l=238,  rr=rr,
             mul=2,       dt1=dt_hi, ksr=0, am_enable=False,
             velocity_sensitivity=0, waveform=car_wf, op_fine_tune=128),
    ]

    ch = dict(
        algorithm=0, feedback=feedback,
        tone_lfo_freq=0, tone_lfo_pmd=0, tone_lfo_amd=0, tone_lfo_delay=0,
        pms=0, ams=0,
        filter_cutoff=255, filter_resonance=0, filter_type=0,
        filter_self_oscillation=True,
        filter_eg_attack=0, filter_eg_decay=0, filter_eg_sustain=0,
        filter_eg_release=0, filter_eg_depth=0,
    )
    return {"operators": ops, "channel": ch}


# ── GMオルガン族 ノブ表（program → ノブ設定）──────────────────────────────────
# 16-18: make_organ_patch（Algorithm 7 / 全OP独立キャリア）
# 16 Drawbar:    MUL 1/2/3/4、軽いFB、Leslie風ビブラート
# 17 Percussive: 20ベース + d1r/d1l でアタック後に落ちて鳴り続ける
# 18 Rock:       全倍音フルボリューム、FB=130 で歪み感
# 20: make_organ_patch（Algorithm 7）
# 20 Reed:       MUL 1/2/4/8、クリーン、即切り
#
# 19/21-23: make_reed_patch（ref.38x6 #50 ベース / Algorithm 0 + 高FB）
# 19 Church:     ref ベース + 低FB + 低mod_mul でソフトなコーラス感（rr=120）
# 21 Accordion:  ref ベース + 広いデチューン（musette）
# 22 Harmonica:  ref.50 のデフォルト値そのまま
# 23 Tango Acc:  ref ベース + 高FB + 速い減衰でアタック感強め
ORGAN_FAMILY: dict[int, tuple[str, dict]] = {
    16: ("Drawbar_Organ", dict(
        tl0=230, tl1=220, tl2=200, tl3=185,
        mul0=1, mul1=2, mul2=3, mul3=4,
        feedback=30, pms=80, vib_depth=60, vib_rate=50, rr=255)),
    17: ("Percussive_Organ", dict(
        tl0=235, tl1=215, tl2=195, tl3=175,
        mul0=1, mul1=2, mul2=4, mul3=8,
        feedback=0, pms=0, vib_depth=0, vib_rate=0,
        ar=255, d1r=200, d1l=240, rr=255)),
    18: ("Rock_Organ", dict(
        tl0=235, tl1=230, tl2=225, tl3=215,
        mul0=1, mul1=2, mul2=3, mul3=4,
        feedback=130, pms=100, vib_depth=80, vib_rate=60, rr=255)),
    19: ("Church_Organ", dict(
        maker="reed",
        feedback=80, mod_mul=6, dt1_spread=14,
        ar=150, d1r=17, d1l0=220, rr=120, mod_wf=24)),
    20: ("Reed_Organ", dict(
        tl0=235, tl1=215, tl2=195, tl3=175,
        mul0=1, mul1=2, mul2=4, mul3=8,
        feedback=0, pms=0, vib_depth=0, vib_rate=0, rr=255)),
    21: ("Accordion", dict(
        maker="reed",
        feedback=220, mod_mul=10, dt1_spread=14,
        ar=200, d1r=30, d1l0=150, rr=140)),
    22: ("Harmonica", dict(
        maker="reed", ar=110, rr=130, mod_mul=2, mod_wf=25)),
    23: ("Tango_Accordion", dict(
        maker="reed",
        feedback=135, mod_mul=10, dt1_spread=10,
        ar=220, d1r=60, d1l0=80, rr=120, car_wf=24)),
}


def _render(patch: dict, note: int, on: float = 2.0, release: float = 3.0) -> np.ndarray:
    freq = 440.0 * 2 ** ((note - 69) / 12.0)
    s = ym38x6_ml.render_patch(json.dumps(patch), freq, on, release, 100, SR)
    x = np.asarray(s, dtype=np.float32)
    peak = float(np.max(np.abs(x)))
    if peak > 1e-6:
        x = x / peak * 0.9
    return x


def _write_wav(path: Path, x: np.ndarray) -> None:
    pcm = np.clip(x * 32767.0, -32768, 32767).astype("<i2")
    with _wave.open(str(path), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(int(SR))
        w.writeframes(pcm.tobytes())


def main() -> int:
    ap = argparse.ArgumentParser(description="オルガン族テンプレート展開")
    ap.add_argument("--only", type=str, default=None,
                    help="生成する program 番号（例: 16,18）。省略時は全8音色")
    ap.add_argument("--note", type=int, default=60, help="試聴ノート（デフォルト 60=C4）")
    ap.add_argument("--out-dir", type=str, default=None,
                    help="WAV 出力先（デフォルト private/organ_family_wav）")
    ap.add_argument("--bank", type=str, default=None,
                    help="指定すると .38x6 バンクファイルも書き出す")
    args = ap.parse_args()

    progs = ([int(p) for p in args.only.split(",")] if args.only
             else list(ORGAN_FAMILY.keys()))

    base = Path(__file__).resolve().parent.parent / "private"
    out_dir = Path(args.out_dir) if args.out_dir else base / "organ_family_wav"
    out_dir.mkdir(parents=True, exist_ok=True)

    presets = []
    print(f"オルガン族展開: {len(progs)} 音色  note={args.note}")
    for prog in progs:
        if prog not in ORGAN_FAMILY:
            print(f"  WARNING: program {prog} はオルガン族にありません")
            continue
        name, knobs = ORGAN_FAMILY[prog]
        k = dict(knobs)
        maker = k.pop("maker", "organ")
        if maker == "reed":
            patch = make_reed_patch(**k)
            info = (f"fb={k.get('feedback', 252)}  "
                    f"mul={k.get('mod_mul', 10)}  "
                    f"dt1_spread={k.get('dt1_spread', 7)}  "
                    f"ar={k.get('ar', 200)}  rr={k.get('rr', 161)}")
        else:
            patch = make_organ_patch(**k)
            tl_str = f"tl={k['tl0']}/{k['tl1']}/{k['tl2']}/{k['tl3']}"
            mul_str = f"mul={k['mul0']}/{k['mul1']}/{k['mul2']}/{k['mul3']}"
            extras = []
            if k.get("feedback", 30) != 30:
                extras.append(f"fb={k['feedback']}")
            if k.get("dt1_spread", 0):
                extras.append(f"dt1_spread={k['dt1_spread']}")
            if k.get("waveform", 0):
                extras.append(f"wf={k['waveform']}")
            if k.get("perc_d1r", 0):
                extras.append(f"perc_d1r={k['perc_d1r']}")
            info = tl_str + "  " + mul_str + ("  " + " ".join(extras) if extras else "")
        x = _render(patch, args.note)
        wav_path = out_dir / f"{prog:03d}_{name}.wav"
        _write_wav(wav_path, x)
        presets.append({"program": prog, "name": name, "patch": patch})
        print(f"  [{prog:3d}] {name:<18}  {info}")

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
