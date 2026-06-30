"""ブラス族テンプレート: 今日設計したブラス骨格を固定し、ノブで族展開する叩き台。

骨格（固定）:
    Algorithm 6 / 4OP / saw キャリア(OP3) / FB / OP0→中間→キャリアの配線
    エンベロープの形（D1R/D1L/RR）

ノブ（族内で動かす高レベルパラメーター）:
    brightness   : モジュレーター TL（小=明るくバズる / 大=暗い）
    attack       : AR（小=ふわっと柔らかい / 大=鋭い）
    aggression   : feedback（大=エッジ・攻撃的）
    detune       : OP間デチューン幅（大=アンサンブル・太い）
    carrier_wf   : キャリア波形（8=saw明るい / 24=tri丸い / 16=sq細い）

使い方:
    python brass_template.py            # 全8音色を生成して聴き比べ
    python brass_template.py --only 56,60  # 指定programのみ
    python brass_template.py --bank private/hand_designed/brass_family.38x6  # バンク書き出し
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


def make_brass_patch(
    brightness: int = 165,   # モジュレーター TL（小=明るい）
    attack: int = 120,       # AR（小=柔らかい）
    aggression: int = 60,    # feedback
    detune: int = 0,         # モジュレーター DT1 幅（alg6用。alg4では使わない）
    carrier_wf: int = 8,     # キャリア波形（8=saw / 24=tri / 16=sq50）
    carrier_tl: int = 248,   # キャリア出力レベル（255=最大・大きいほど音量大）
    mul_op2: int = 1,        # OP2 の MUL（alg6用: 3にすると倍音成分が分散）
    algorithm: int = 6,      # FMアルゴリズム（6=直列スタック / 4=2ペア独立）
    car_dt_spread: int = 0,  # alg4専用: キャリア(OP1/OP3)のDT1幅（小=なめらか）
    # ── フィルター（シンセ系のスウィープ用。既定=全開・無効）──
    filter_cutoff: int = 255,    # ベースカットオフ（小=閉じる）
    filter_resonance: int = 0,   # レゾナンス（大=ピーク強調）
    filter_type: int = 0,        # 0=LP / 1=HP / 2=BP
    filter_eg_depth: int = 0,    # EGで開く量（cutoff + eg×depth）
    filter_eg_attack: int = 0,   # フィルターEG: A
    filter_eg_decay: int = 0,    # 〃 D
    filter_eg_sustain: int = 0,  # 〃 S
    filter_eg_release: int = 0,  # 〃 R
) -> dict:
    """高レベルノブからブラスパッチ dict を生成する。骨格は固定。"""

    def op(wf: int, tl: int, mul: int, dt1: int = 128) -> dict:
        return dict(tl=tl, ar=attack, d1r=20, d2r=0, d1l=200, rr=100,
                    mul=mul, dt1=dt1, ksr=0, am_enable=False,
                    velocity_sensitivity=0, waveform=wf, op_fine_tune=128)

    # detune: OP0/OP1 を逆方向に散らす（alg6 モジュレーター用、128 中心）
    dt_lo = 128 - detune
    dt_hi = 128 + detune
    # car_dt_spread: alg4 のキャリア(OP1/OP3)だけデチューン（セクション感）
    car_dt_lo = 128 - car_dt_spread
    car_dt_hi = 128 + car_dt_spread

    ch = dict(
        algorithm=algorithm, feedback=aggression,
        tone_lfo_freq=0, tone_lfo_pmd=0, tone_lfo_amd=0, tone_lfo_delay=0,
        pms=0, ams=0,
        filter_cutoff=filter_cutoff, filter_resonance=filter_resonance,
        filter_type=filter_type, filter_self_oscillation=False,
        filter_eg_attack=filter_eg_attack, filter_eg_decay=filter_eg_decay,
        filter_eg_sustain=filter_eg_sustain, filter_eg_release=filter_eg_release,
        filter_eg_depth=filter_eg_depth,
    )
    if algorithm == 4:
        # Alg4: (OP0→OP1) + (OP2→OP3) 2ペア独立（2キャリア合算のため carrier_tl から減算してクリップ回避）
        # モジュレーターはDT1固定、キャリアのみcar_dt_spreadでデチューン
        car_tl_lo = max(carrier_tl - 18, 0)
        car_tl_hi = max(carrier_tl - 16, 0)
        ops = [
            op(0,          brightness,     1, dt1=128),       # OP0: mod1
            op(carrier_wf, car_tl_lo,      1, dt1=car_dt_lo), # OP1: car1
            op(0,          brightness - 3, 2, dt1=128),       # OP2: mod2（MUL2）
            op(carrier_wf, car_tl_hi,      1, dt1=car_dt_hi), # OP3: car2
        ]
    else:
        # Alg6（既定）: 直列スタック
        ops = [
            op(0, brightness,     1, dt1=dt_lo),
            op(0, brightness - 5, 2, dt1=dt_hi),
            op(0, brightness - 8, mul_op2),
            op(carrier_wf, carrier_tl, 1),
        ]
    return {"operators": ops, "channel": ch}


# ── GMブラス族 ノブ表（program → ノブ設定）─────────────────────────────────
# プロトタイプ(056 Trumpet)を基準に、机上予測でノブを差分設定した叩き台。
BRASS_FAMILY: dict[int, tuple[str, dict]] = {
    56: ("Trumpet",        dict(brightness=170, attack=120, aggression=60,  detune=0,  carrier_wf=8)),
    57: ("Trombone",       dict(brightness=180, attack=105, aggression=55,  detune=0,  carrier_wf=8)),
    58: ("Tuba",           dict(brightness=190, attack=100, aggression=45,  detune=0,  carrier_wf=24)),
    59: ("Muted_Trumpet",  dict(brightness=160, attack=125, aggression=100, detune=0,  carrier_wf=8)),
    60: ("French_Horn",    dict(brightness=195, attack=90,  aggression=20,  detune=0,  carrier_wf=24)),
    61: ("Brass_Section",  dict(brightness=172, attack=110, aggression=55,  detune=0,  carrier_wf=8, algorithm=4, car_dt_spread=4)),
    62: ("SynthBrass_1",   dict(brightness=160, attack=140, aggression=90,  detune=0,  carrier_wf=8,
                                carrier_tl=212,
                                filter_cutoff=70, filter_resonance=180, filter_type=0, filter_eg_depth=185,
                                filter_eg_attack=70, filter_eg_decay=90, filter_eg_sustain=130, filter_eg_release=60)),
    63: ("SynthBrass_2",   dict(brightness=160, attack=85,  aggression=70,  detune=0,  carrier_wf=8,
                                carrier_tl=235,
                                filter_cutoff=160, filter_resonance=100, filter_type=0)),
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
    ap = argparse.ArgumentParser(description="ブラス族テンプレート展開")
    ap.add_argument("--only", type=str, default=None,
                    help="生成するprogram番号（例: 56,60）。省略時は全8音色")
    ap.add_argument("--note", type=int, default=60, help="試聴ノート（デフォルト60=C4）")
    ap.add_argument("--out-dir", type=str, default=None,
                    help="WAV出力先（デフォルト private/brass_family_wav）")
    ap.add_argument("--bank", type=str, default=None,
                    help="指定すると .38x6 バンクファイルも書き出す")
    args = ap.parse_args()

    progs = ([int(p) for p in args.only.split(",")] if args.only
             else list(BRASS_FAMILY.keys()))

    base = Path(__file__).resolve().parent.parent / "private"
    out_dir = Path(args.out_dir) if args.out_dir else base / "brass_family_wav"
    out_dir.mkdir(parents=True, exist_ok=True)

    presets = []
    print(f"ブラス族展開: {len(progs)} 音色  note={args.note}")
    for prog in progs:
        if prog not in BRASS_FAMILY:
            print(f"  WARNING: program {prog} はブラス族にありません")
            continue
        name, knobs = BRASS_FAMILY[prog]
        patch = make_brass_patch(**knobs)
        x = _render(patch, args.note)
        wav_path = out_dir / f"{prog:03d}_{name}.wav"
        _write_wav(wav_path, x)
        presets.append({"program": prog, "name": name, "patch": patch})
        knob_str = "  ".join(f"{k}={v}" for k, v in knobs.items())
        print(f"  [{prog:3d}] {name:<16} {knob_str}")

    print(f"\nWAV出力: {out_dir}")

    if args.bank:
        bank_path = Path(args.bank)
        bank_path.parent.mkdir(parents=True, exist_ok=True)
        bank_path.write_text(
            json.dumps({"bank": 0, "presets": presets}, indent=2), encoding="utf-8")
        print(f"バンク書き出し: {bank_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
