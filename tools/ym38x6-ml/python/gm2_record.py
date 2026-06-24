"""gm2_record.py: Microsoft GS Wavetable Synth の GM2 全プログラムを録音する。

ステレオ ミキサー（システム出力ループバック）で録音するため Windows 専用。
事前に pip install sounddevice が必要。

使い方:
    python gm2_record.py [options]

オプション:
    --out-dir DIR        出力先ディレクトリ (既定: ../private/gm2_wav)
    --note N             録音する MIDI ノート番号 (既定: 60=C4)
    --vel N              ベロシティ (既定: 100)
    --on SECS            キーオン保持秒数 (既定: 2.5)
    --release SECS       キーオフ後の余韻秒数 (既定: 1.5)
    --pre-delay SECS     ノートオン前の待機秒数 (既定: 0.3)
    --programs START-END 録音するプログラム範囲 (既定: 0-127)
    --sr HZ              サンプルレート (既定: 44100)
    --midi-device N      MIDI 出力デバイスインデックス (既定: 0)
    --rec-device N       録音デバイスインデックス (既定: 自動検出)
    --list-devices       デバイス一覧を表示して終了
    --channel CH         MIDI チャンネル 0-15 (既定: 0)
"""

from __future__ import annotations

import argparse
import ctypes
import queue
import sys
import threading
import time
import warnings
import wave
from pathlib import Path

import numpy as np

try:
    import soundcard as sc
    from soundcard import SoundcardRuntimeWarning
    warnings.filterwarnings("ignore", category=SoundcardRuntimeWarning)
except ImportError:
    print("ERROR: soundcard が必要です。 python -m pip install soundcard", file=sys.stderr)
    sys.exit(1)

# ─── GM2 プログラム名 (0-127) ────────────────────────────────────────────
GM2_NAMES = [
    # Piano (0-7)
    "Acoustic_Grand_Piano", "Bright_Acoustic_Piano", "Electric_Grand_Piano",
    "Honky-tonk_Piano", "Electric_Piano_1", "Electric_Piano_2",
    "Harpsichord", "Clavi",
    # Chromatic Perc (8-15)
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
    "Lead_5_charang", "Lead_6_voice", "Lead_7_fifths", "Lead_8_bass+lead",
    # Synth Pad (88-95)
    "Pad_1_new_age", "Pad_2_warm", "Pad_3_polysynth", "Pad_4_choir",
    "Pad_5_bowed", "Pad_6_metallic", "Pad_7_halo", "Pad_8_sweep",
    # Synth Effects (96-103)
    "FX_1_rain", "FX_2_soundtrack", "FX_3_crystal", "FX_4_atmosphere",
    "FX_5_brightness", "FX_6_goblins", "FX_7_echoes", "FX_8_sci-fi",
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

# ─── GM2 ドラムキット (ch10 プログラム番号 → 名前) ─────────────────────
GM2_DRUM_KITS: dict[int, str] = {
    0:  "Standard_Kit",
    8:  "Room_Kit",
    16: "Power_Kit",
    24: "Electronic_Kit",
    25: "Analog_Kit",
    32: "Jazz_Kit",
    40: "Brush_Kit",
    48: "Orchestra_Kit",
    56: "SFX_Kit",
}

# GM2 標準ドラムノートマップ (note → 名前)
GM2_DRUM_NOTES: dict[int, str] = {
    35: "Acoustic_Bass_Drum",
    36: "Bass_Drum_1",
    37: "Side_Stick",
    38: "Acoustic_Snare",
    39: "Hand_Clap",
    40: "Electric_Snare",
    41: "Low_Floor_Tom",
    42: "Closed_Hi-Hat",
    43: "High_Floor_Tom",
    44: "Pedal_Hi-Hat",
    45: "Low_Tom",
    46: "Open_Hi-Hat",
    47: "Low-Mid_Tom",
    48: "Hi-Mid_Tom",
    49: "Crash_Cymbal_1",
    50: "High_Tom",
    51: "Ride_Cymbal_1",
    52: "Chinese_Cymbal",
    53: "Ride_Bell",
    54: "Tambourine",
    55: "Splash_Cymbal",
    56: "Cowbell",
    57: "Crash_Cymbal_2",
    58: "Vibraslap",
    59: "Ride_Cymbal_2",
    60: "Hi_Bongo",
    61: "Low_Bongo",
    62: "Mute_Hi_Conga",
    63: "Open_Hi_Conga",
    64: "Low_Conga",
    65: "High_Timbale",
    66: "Low_Timbale",
    67: "High_Agogo",
    68: "Low_Agogo",
    69: "Cabasa",
    70: "Maracas",
    71: "Short_Whistle",
    72: "Long_Whistle",
    73: "Short_Guiro",
    74: "Long_Guiro",
    75: "Claves",
    76: "Hi_Wood_Block",
    77: "Low_Wood_Block",
    78: "Mute_Cuica",
    79: "Open_Cuica",
    80: "Mute_Triangle",
    81: "Open_Triangle",
}

# ─── Windows MIDI (winmm) ────────────────────────────────────────────────
_winmm = ctypes.windll.winmm

# 64bit Windows では HMIDIOUT は pointer-size の handle 値。
# argtypes を明示して midiOutShortMsg に正しく渡す。
_HMIDIOUT = ctypes.c_size_t
_winmm.midiOutOpen.argtypes = [
    ctypes.POINTER(_HMIDIOUT), ctypes.c_uint, ctypes.c_size_t,
    ctypes.c_size_t, ctypes.c_uint,
]
_winmm.midiOutOpen.restype = ctypes.c_uint
_winmm.midiOutShortMsg.argtypes = [_HMIDIOUT, ctypes.c_uint]
_winmm.midiOutShortMsg.restype = ctypes.c_uint
_winmm.midiOutClose.argtypes = [_HMIDIOUT]
_winmm.midiOutClose.restype = ctypes.c_uint


class MidiOut:
    """winmm ベースの MIDI 出力ハンドル。"""

    def __init__(self, device_id: int = 0):
        self._handle = _HMIDIOUT(0)
        ret = _winmm.midiOutOpen(ctypes.byref(self._handle), device_id, 0, 0, 0)
        if ret != 0:
            raise RuntimeError(f"midiOutOpen 失敗: code={ret}")

    def _send(self, status: int, d1: int = 0, d2: int = 0) -> None:
        msg = (status & 0xFF) | ((d1 & 0xFF) << 8) | ((d2 & 0xFF) << 16)
        _winmm.midiOutShortMsg(self._handle, msg)

    def program_change(self, ch: int, prog: int) -> None:
        self._send(0xC0 | ch, prog)

    def note_on(self, ch: int, note: int, vel: int) -> None:
        self._send(0x90 | ch, note, vel)

    def note_off(self, ch: int, note: int) -> None:
        self._send(0x80 | ch, note, 0)

    def all_notes_off(self, ch: int) -> None:
        self._send(0xB0 | ch, 123, 0)

    def control_change(self, ch: int, cc: int, val: int) -> None:
        self._send(0xB0 | ch, cc, val)

    def close(self) -> None:
        _winmm.midiOutClose(self._handle)

    def __enter__(self) -> "MidiOut":
        return self

    def __exit__(self, *_) -> None:
        self.close()


def list_midi_devices() -> list[str]:
    """利用可能な MIDI 出力デバイス名を返す。"""

    class MIDIOUTCAPS(ctypes.Structure):
        _fields_ = [
            ("wMid", ctypes.c_ushort), ("wPid", ctypes.c_ushort),
            ("vDriverVersion", ctypes.c_uint), ("szPname", ctypes.c_char * 32),
            ("wTechnology", ctypes.c_ushort), ("wVoices", ctypes.c_ushort),
            ("wNotes", ctypes.c_ushort), ("wChannelMask", ctypes.c_ushort),
            ("dwSupport", ctypes.c_ulong),
        ]

    n = _winmm.midiOutGetNumDevs()
    names = []
    for i in range(n):
        caps = MIDIOUTCAPS()
        _winmm.midiOutGetDevCapsA(i, ctypes.byref(caps), ctypes.sizeof(caps))
        names.append(caps.szPname.decode("cp932", errors="replace"))
    return names


# ─── セッション録音（WASAPI ループバック）───────────────────────────────

class SessionRecorder:
    """soundcard の WASAPI ループバックを使い、デジタルで直接キャプチャする。

    ステレオ ミキサー（DAC→ADC 経由）と違い、アナログ段を通らないため
    ノイズが乗らない。record() は1回の大きなブロックで呼び出すことで
    discontinuity を防ぐ。
    """

    def __init__(self, sr: int, device_name: str | None = None) -> None:
        self._sr = sr
        if device_name:
            speaker = next(s for s in sc.all_speakers() if device_name in s.name)
        else:
            speaker = sc.default_speaker()
        self._mic = sc.get_microphone(speaker.id, include_loopback=True)
        self._stream: object = None

    def __enter__(self) -> "SessionRecorder":
        self._ctx = self._mic.recorder(samplerate=self._sr, channels=2)
        self._stream = self._ctx.__enter__()
        return self

    def __exit__(self, *args) -> None:
        self._ctx.__exit__(*args)

    def record(self, secs: float) -> np.ndarray:
        """指定秒数分をブロッキングで録音してステレオ配列 [n_frames, 2] を返す。"""
        n = int(secs * self._sr)
        return self._stream.record(numframes=n).astype(np.float32)


def _fade(x: np.ndarray, sr: int, ms: float = 10.0) -> np.ndarray:
    """頭と尾に線形フェードをかける（境界クリック隠蔽）。"""
    n = int(sr * ms / 1000)
    if n == 0 or len(x) < n * 2:
        return x
    ramp = np.linspace(0.0, 1.0, n, dtype=np.float32)
    x = x.copy()
    x[:n] *= ramp
    x[-n:] *= ramp[::-1]
    return x


def record_note(
    recorder: SessionRecorder,
    midi: MidiOut,
    prog: int,
    note: int,
    vel: int,
    on_secs: float,
    release_secs: float,
    pre_delay: float,
    sr: int,
    ch: int,
    setup_program: bool = True,
) -> np.ndarray:
    """1プログラムまたは1ドラムノートを録音してモノラル float32 配列を返す。

    録音は1回の record() で行い discontinuity を防ぐ。
    MIDI タイミングは別スレッドで制御する。
    setup_program=False のとき PC/CC 初期化をスキップ（ドラムのノートごとの録音で使用）。
    """
    midi.all_notes_off(ch)
    if setup_program:
        midi.control_change(ch, 7, 127)
        midi.control_change(ch, 11, 127)
        midi.program_change(ch, prog)
        time.sleep(0.05)

    total_secs = pre_delay + on_secs + release_secs

    def midi_timer() -> None:
        time.sleep(pre_delay)
        midi.note_on(ch, note, vel)
        time.sleep(on_secs)
        midi.note_off(ch, note)

    t = threading.Thread(target=midi_timer, daemon=True)
    t.start()
    audio = recorder.record(total_secs)   # 1回のブロッキング録音
    t.join()

    mono = audio.mean(axis=1).astype(np.float32)
    mono = _fade(mono, sr, ms=10.0)
    return mono


def save_wav(path: Path, samples: np.ndarray, sr: int, normalize: bool = True) -> None:
    x = samples.copy()
    if normalize:
        peak = float(np.max(np.abs(x)))
        if peak > 1e-6:
            x = x / peak * 0.5  # -6dBFS
    x = np.clip(x * 32767.0, -32768, 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(x.tobytes())


# ─── CLI ─────────────────────────────────────────────────────────────────

def parse_program_range(s: str) -> tuple[int, int]:
    if "-" in s:
        a, b = s.split("-", 1)
        return int(a), int(b)
    v = int(s)
    return v, v


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Microsoft GS Wavetable Synth の GM2 音色を録音する",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--out-dir", default=None, help="出力先 (既定: ../private/gm2_wav or gm2_drums)")
    ap.add_argument("--note", type=int, default=60, help="MIDI ノート番号 (既定: 60=C4、メロディモード)")
    ap.add_argument("--vel", type=int, default=100, help="ベロシティ (既定: 100)")
    ap.add_argument("--on", type=float, default=None, help="キーオン秒数 (既定: メロディ=2.5 / ドラム=0.05)")
    ap.add_argument("--release", type=float, default=None, help="キーオフ後余韻秒 (既定: メロディ=1.5 / ドラム=2.0)")
    ap.add_argument("--pre-delay", type=float, default=0.3, help="ノートオン前待機秒 (既定: 0.3)")
    ap.add_argument("--programs", default="0-127", help="メロディ録音範囲 START-END (既定: 0-127)")
    ap.add_argument("--sr", type=int, default=44100, help="サンプルレート (既定: 44100)")
    ap.add_argument("--midi-device", type=int, default=0, help="MIDI 出力デバイスインデックス (既定: 0)")
    ap.add_argument("--rec-device", default=None, help="録音スピーカー名の部分一致 (既定: システムデフォルト出力)")
    ap.add_argument("--channel", type=int, default=0, help="MIDI チャンネル 0-15 (既定: 0、--drums時は9固定)")
    ap.add_argument("--no-normalize", action="store_true", help="ピーク正規化を無効化")
    ap.add_argument("--list-devices", action="store_true", help="デバイス一覧を表示して終了")
    # ドラムモード
    ap.add_argument("--drums", action="store_true", help="ドラムキット録音モード (ch9、ノート35-81)")
    ap.add_argument("--drum-kits", default="0", help="録音するドラムキットのプログラム番号 カンマ区切り (既定: 0=Standard_Kit)")
    ap.add_argument("--drum-notes", default="35-81", help="録音するノート範囲 START-END (既定: 35-81)")
    args = ap.parse_args()

    if args.list_devices:
        print("=== MIDI 出力デバイス ===")
        for i, name in enumerate(list_midi_devices()):
            print(f"  [{i}] {name}")
        print()
        print("=== 録音可能スピーカー (WASAPI ループバック) ===")
        for s in sc.all_speakers():
            print(f"  {s.name}")
        return 0

    # MIDI デバイス確認
    midi_devs = list_midi_devices()
    if args.midi_device >= len(midi_devs):
        print(f"ERROR: MIDI デバイス {args.midi_device} が見つかりません", file=sys.stderr)
        return 1
    print(f"MIDI 出力: [{args.midi_device}] {midi_devs[args.midi_device]}")

    # 録音デバイス確認
    if args.rec_device:
        matches = [s for s in sc.all_speakers() if args.rec_device in s.name]
        if not matches:
            print(f"ERROR: スピーカー '{args.rec_device}' が見つかりません", file=sys.stderr)
            return 1
        rec_name: str | None = matches[0].name
    else:
        rec_name = None  # デフォルト出力を使用
    speaker_name = rec_name or sc.default_speaker().name
    print(f"録音デバイス: {speaker_name} (WASAPI ループバック)")

    # on/release デフォルト（モードで変わる）
    on_secs = args.on if args.on is not None else (0.05 if args.drums else 2.5)
    release_secs = args.release if args.release is not None else (2.0 if args.drums else 1.5)

    private = Path(__file__).resolve().parent.parent / "private"
    failed: list[int] = []

    with MidiOut(args.midi_device) as midi, \
         SessionRecorder(args.sr, device_name=rec_name) as recorder:

        if args.drums:
            # ── ドラムモード ─────────────────────────────────────────────
            drum_ch = 9
            kit_progs = [int(k.strip()) for k in args.drum_kits.split(",")]
            note_start, note_end = parse_program_range(args.drum_notes)

            base_dir = Path(args.out_dir) if args.out_dir else private / "gm2_drums"
            print(f"出力先: {base_dir}")
            print(
                f"ドラムキット: {kit_progs}  "
                f"ノート: {note_start}-{note_end}  "
                f"vel={args.vel}  on={on_secs}s  release={release_secs}s"
            )
            print()

            for kit_prog in kit_progs:
                kit_name = GM2_DRUM_KITS.get(kit_prog, f"Kit_{kit_prog}")
                kit_dir = base_dir / f"{kit_prog:03d}_{kit_name}"
                kit_dir.mkdir(parents=True, exist_ok=True)
                print(f"=== {kit_name} (prog {kit_prog}) ===")

                # ドラムキット切り替え（ch9 へのプログラムチェンジ）
                midi.control_change(drum_ch, 0, 0)    # Bank Select MSB
                midi.control_change(drum_ch, 32, 0)   # Bank Select LSB
                midi.program_change(drum_ch, kit_prog)
                time.sleep(0.1)

                for note in range(note_start, note_end + 1):
                    note_name = GM2_DRUM_NOTES.get(note, f"Note_{note}")
                    fname = kit_dir / f"{note:03d}_{note_name}.wav"
                    print(f"  [{note:3d}] {note_name:<25} ... ", end="", flush=True)
                    try:
                        samples = record_note(
                            recorder=recorder,
                            midi=midi,
                            prog=kit_prog,
                            note=note,
                            vel=args.vel,
                            on_secs=on_secs,
                            release_secs=release_secs,
                            pre_delay=args.pre_delay,
                            sr=args.sr,
                            ch=drum_ch,
                            setup_program=False,
                        )
                        save_wav(fname, samples, args.sr, normalize=not args.no_normalize)
                        rms = float(np.sqrt(np.mean(samples ** 2)))
                        print(f"OK  RMS={rms:.4f}")
                    except Exception as e:
                        print(f"FAIL: {e}")
                        failed.append(note)

                midi.all_notes_off(drum_ch)
                print()

            total = len(kit_progs) * (note_end - note_start + 1)

        else:
            # ── メロディモード ───────────────────────────────────────────
            out_dir = Path(args.out_dir) if args.out_dir else private / "gm2_wav"
            out_dir.mkdir(parents=True, exist_ok=True)
            print(f"出力先: {out_dir}")

            prog_start, prog_end = parse_program_range(args.programs)
            print(
                f"プログラム: {prog_start}-{prog_end}  "
                f"note={args.note}  vel={args.vel}  "
                f"on={on_secs}s  release={release_secs}s"
            )
            print()

            for prog in range(prog_start, prog_end + 1):
                name = GM2_NAMES[prog] if prog < len(GM2_NAMES) else f"prog{prog}"
                fname = out_dir / f"{prog:03d}_{name}.wav"
                print(f"[{prog:3d}] {name:<30} ... ", end="", flush=True)
                try:
                    samples = record_note(
                        recorder=recorder,
                        midi=midi,
                        prog=prog,
                        note=args.note,
                        vel=args.vel,
                        on_secs=on_secs,
                        release_secs=release_secs,
                        pre_delay=args.pre_delay,
                        sr=args.sr,
                        ch=args.channel,
                    )
                    save_wav(fname, samples, args.sr, normalize=not args.no_normalize)
                    rms = float(np.sqrt(np.mean(samples ** 2)))
                    print(f"OK  RMS={rms:.4f}  → {fname.name}")
                except Exception as e:
                    print(f"FAIL: {e}")
                    failed.append(prog)

            midi.all_notes_off(args.channel)
            total = prog_end - prog_start + 1

    print()
    print(f"完了: {total - len(failed)}/{total} 件成功")
    if failed:
        print(f"失敗: {failed}")
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())
