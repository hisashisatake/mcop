"""GM2リズムチャンネル用ドラムキット試作: Standard Kit（kit 0 = bank 15360）。

Step 9（GM2リズムチャンネル実装計画）の実機検証用プロトタイプ。全ノート:
- fixed_note_enable=True（GM2の「ノート番号=楽器選択キー、各楽器は固定ピッチ」を体現）
- 全4オペレーターのEGへ auto_release=1 / retrigger_mode=1(RESET) を設定
  （🔴A: 外部note_offを無視して自動リリースへ入る、🟠E: 同音連打をResetで再アタック）
- 音色本体はOP1（キャリア）のみが鳴らす。OP2-4はtl=0（無音）だが、is_idle()が
  「全4オペレーターがidle」条件のためOP1と同じar/d1r/d1l/rrを与え、同じタイミングで
  一緒にリリース完了させる（op_c()と同型、そうしないとOP2-4がホールド段に永久停滞し
  ボイスが解放されない = 🔴B）。

ノイズ系（waveform 32〜63、`sound_fm::waveform::is_noise_waveform`）はピッチ非依存
（`Operator::next_noise_sample`が周波数を一切参照しない）。Snare/HH/Crash/Ride/Clap/
Rimはこれを使い、fixed_noteの値自体は「GM2のノート番号=楽器選択キー」という意味だけを
持ち音には影響しない。Kick/Tomはsine(waveform=0)キャリアで、fixed_noteが実際に音高を決める。

使い方:
    python gm2_drum_kit.py                                    # 既定 private/rhythm_kits/standard_kit.op505
    python gm2_drum_kit.py --bank private/rhythm_kits/standard_kit.op505
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.stdout.reconfigure(encoding="utf-8", errors="replace")

import op505_patch  # noqa: E402

RHYTHM_BANK_BASE = 15360  # op505-midi::rhythm::RHYTHM_BANK_BASE と一致させる

# ノイズ色（waveform - 32、0=速い/白色寄り 〜 31=遅い/低域寄り）
NOISE_WAVEFORM_BASE = 32


def noise_waveform(color: int) -> int:
    return NOISE_WAVEFORM_BASE + max(0, min(31, color))


def make_drum_patch(*, waveform: int, fixed_note: int, tl: int, ar: int, rr: int,
                     d1r: int = 0, d1l: int = 255, mul: int = 1, feedback: int = 0) -> dict:
    """1オペレーター（OP1）だけが鳴るAR(+D1)+自動リリース型ドラムパッチ。

    既定(d1r=0/d1l=255)は`convert_eg_shape()`がstage_count=2・release_point=0の
    「段0=アタック、段1=リリース(rr)」という最短形を作る（`eg_convert.rs`参照）。
    d1r>0を指定すると3段（段0=アタック、段1=d1r decay→d1l、段2=リリース(rr)、
    release_point=1）になり、「ホールド区間に実時間の長さがある」ドラム
    （シンバルの本体シェイク等）を表現できる。どちらの形でもauto_release=1・
    loop_enabled=0なので、ホールド区間の完了と同時に外部note_offを待たずリリースへ
    入る＝ワンショット（3段形式だと、ごく短いnote-offでd1r decayの途中で切られる
    のをauto_releaseが防ぐ、という🔴Aの効果がはっきり出る）。
    """
    def op(tl_: int, wf: int) -> dict:
        o = op505_patch.make_op(
            tl=tl_, ar=ar, d1r=d1r, d2r=0, d1l=d1l, rr=rr,
            mul=mul, dt1=128, ksr=0, am_enable=False,
            velocity_sensitivity=0, waveform=wf, op_fine_tune=128,
        )
        o["eg"]["auto_release"] = 1
        o["eg"]["retrigger_mode"] = 1  # RETRIGGER_MODE_RESET
        return o

    ops = [op(tl, waveform), op(0, waveform), op(0, waveform), op(0, waveform)]

    ch = op505_patch.make_channel(algorithm=7, feedback=feedback)
    ch["fixed_note_enable"] = True
    ch["fixed_note"] = fixed_note
    ch["fixed_note_fine"] = 128

    return {"operators": ops, "channel": ch}


def make_tonal_drum(*, fixed_note: int, tl: int, ar: int, rr: int, mul: int = 1,
                     d1r: int = 0, d1l: int = 255) -> dict:
    return make_drum_patch(waveform=0, fixed_note=fixed_note, tl=tl, ar=ar, rr=rr, mul=mul, d1r=d1r, d1l=d1l)


def make_noise_drum(*, fixed_note: int, tl: int, ar: int, rr: int, color: int,
                     d1r: int = 0, d1l: int = 255) -> dict:
    return make_drum_patch(waveform=noise_waveform(color), fixed_note=fixed_note, tl=tl, ar=ar, rr=rr, d1r=d1r, d1l=d1l)


# ── Standard Kit（GM2ノート番号 → (名前, 生成関数, kwargs)）────────────────────
# ar=255（瞬時アタック、クリック回避のため0ではなくフルレート）。rrがそのままテール長。
STANDARD_KIT: dict[int, tuple[str, str, dict]] = {
    36: ("Bass_Drum_1", "tonal", dict(fixed_note=36, tl=250, ar=255, rr=90)),   # ~0.25s
    38: ("Snare_Drum_1", "noise", dict(fixed_note=60, tl=220, ar=255, rr=110, color=6)),  # ~0.2s
    40: ("Electric_Snare", "noise", dict(fixed_note=60, tl=220, ar=255, rr=120, color=3)),
    39: ("Hand_Clap", "noise", dict(fixed_note=63, tl=200, ar=255, rr=100, color=2)),
    37: ("Side_Stick", "noise", dict(fixed_note=75, tl=180, ar=255, rr=200, color=10)),  # 短い
    42: ("Closed_Hi-Hat", "noise", dict(fixed_note=80, tl=180, ar=255, rr=210, color=1)),  # 最短
    46: ("Open_Hi-Hat", "noise", dict(fixed_note=80, tl=180, ar=255, rr=130, color=1)),   # 開いた分長め
    # d1r/d1l付き3段EG: ホールド区間(段0attack+段1 d1r decay)に実時間の長さがあるため、
    # ごく短いnote-off(1/32)でも段1が完了するまで自動リリースへ入らない=auto_releaseの
    # 効果がはっきり出る（🔴A検証用。既定のAR2段構成だとホールド区間が一瞬なので差が出ない）。
    49: ("Crash_Cymbal_1", "noise", dict(fixed_note=84, tl=210, ar=255, d1r=25, d1l=140, rr=60, color=4)),
    51: ("Ride_Cymbal_1", "noise", dict(fixed_note=81, tl=190, ar=255, rr=60, color=8)),
    41: ("Low_Tom", "tonal", dict(fixed_note=41, tl=230, ar=255, rr=100)),
    45: ("Mid_Tom", "tonal", dict(fixed_note=45, tl=225, ar=255, rr=105)),
    48: ("High_Tom", "tonal", dict(fixed_note=48, tl=220, ar=255, rr=110)),
}


def build_presets() -> list[dict]:
    presets = []
    for note, (name, kind, kwargs) in sorted(STANDARD_KIT.items()):
        patch = make_tonal_drum(**kwargs) if kind == "tonal" else make_noise_drum(**kwargs)
        presets.append({"program": note, "name": name, "patch": patch})
    return presets


def main() -> int:
    ap = argparse.ArgumentParser(description="GM2 Standard Kit（kit 0）試作")
    ap.add_argument("--bank", type=str, default=None,
                     help="出力先 .op505 パス（既定 private/rhythm_kits/standard_kit.op505）")
    args = ap.parse_args()

    base = Path(__file__).resolve().parent.parent
    bank_path = Path(args.bank) if args.bank else base / "private" / "rhythm_kits" / "standard_kit.op505"
    bank_path.parent.mkdir(parents=True, exist_ok=True)

    presets = build_presets()
    bank_path.write_text(
        json.dumps({"bank": RHYTHM_BANK_BASE, "presets": presets}, indent=2), encoding="utf-8")

    print(f"Standard Kit: {len(presets)} 音色  bank=0x{RHYTHM_BANK_BASE:04X}({RHYTHM_BANK_BASE})")
    for note, (name, kind, _kwargs) in sorted(STANDARD_KIT.items()):
        print(f"  [{note:3d}] {name:<18} ({kind})")
    print(f"\nバンク書き出し: {bank_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
