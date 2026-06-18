# ym38x6

A fictional FM synthesizer and composition support app.

## Concept

**38x6** is an imaginary FM sound chip — what if FM synthesis had taken one more step forward before PCM took over? It is based on YAMAHA's YM3806 (OPQ) with waveform extensions inspired by the OPZ, implemented entirely in Rust.

The companion **composition app** lets anyone play musically coherent chords without music theory knowledge, using a calibration-based gesture UI with no grids or guides.

Inspired by Ryu Umemoto's YM-2609, which explored a similar "what if" premise using SynthEdit + VOPM.

## Architecture

```
ym38x6/
  sound-core/         # Core primitives — WaveTable, AdsrParams, SoundEngine trait
  ym38x6-core/        # 38x6 FM engine implementation (depends on sound-core)
  ym38x6-vst/         # 38x6 VST3/CLAP plugin (nice-plug)
  gesture-app/        # Composition app (Tauri v2, Windows desktop)
    src/              # Frontend: calibration + gesture UI (HTML/JS)
    src-tauri/        # Backend: cpal WASAPI output, Tauri commands
```

`sound-core` and `ym38x6-core` have zero dependencies on nice-plug, Tauri, or cpal. The audio engine is fully isolated.

## Sound Engine

### Waveform Memory Voice Bank (single-operator)

A dedicated voice bank — *not* an engine mode. Each voice is an ordinary 38x6 patch with only OP1 audible (Algorithm 7, OP2–4 muted at TL=0). Voices are provided under the reserved `WAVEFORM_MEMORY_BANK` Bank Select; `waveform_memory_patch` in `ym38x6-core` builds each patch from the Program number.

This bank succeeds **WMS-1**, the Phase-1 prototype: a standalone waveform-oscillator + ADSR crate (`wms1-core` / `wms1-vst`). It was removed once it became clear that its single-operator voice was equivalent to a one-operator 38x6 patch, so the same sounds are now produced by the 38x6 engine itself.

- Internal wave format: 1024 × u16, log encoding (ymfm-compatible)
  - `bit14~0`: −log₂|amplitude| in 4.8 fixed point
  - `bit15`: sign flag
- Built-in waveforms: 38x6's native 8 waveforms (slots 0–7)
- User waveforms: 32 × i8 linear input → auto-converted to internal format (slots 8–255)
- Unlimited polyphony via `HashMap`-based stable channel IDs

### 38x6 FM Engine (Phase 3+)

4-operator FM synthesis, OPQ-derived with OPZ waveform extensions.

- 4op / channel, 8 algorithms
- Per-operator frequency: Op0–3 each always have an independent octave (3-bit) + F-Number (13-bit, more precise than OPQ's 12-bit) — generalizes OPQ's "2 frequencies per channel" (Op0/2, Op1/3 pairs)
- Per-operator key-on (Op3 as master)
- All parameters 0–255 (8-bit unified); octave + F-Number (16-bit total) is the only exception
- **State Variable Filter** per voice: Cutoff (0–255, log scale), Resonance (0–255), Type (LP/HP/BP)

## Composition App

### Gesture UI

No grids, no guides. The coordinate space is defined by the player's own calibration.

**Calibration** (mouse or touch):
Click C major, F major, and G major at positions that feel natural. The I–IV–V triangle defines the entire coordinate system — both the root note axis and the chord type axis.

**Playing** (mouse version):
- Hold and drag to play
- Y direction (along root axis) → root note
- X direction (perpendicular) → chord type: `dim` ← `m` ← `maj` → `7` → `maj7`
- Release → note-off (ADSR release)
- `R` key → recalibrate

The gesture system requires no recognition algorithm — everything is continuous coordinate-to-pitch mapping. An ∞ motion naturally produces vibrato.

### Avoid Note Handling (Phase 7)

Selectable handling for notes outside the current scale:
- **Snap** — auto-correct to the nearest scale tone
- **Random shift** — move to an adjacent scale tone, up or down
- **Silence** — don't play
- **Warning playback** — play at reduced volume via OPQ-derived per-operator key-on (Op3 stays on as master, Op0–2 play quieter), giving musical feedback instead of a hard block

## Development Roadmap

| Phase | Scope |
|-------|-------|
| 1 | Waveform memory prototype + Tauri desktop app + gesture UI (done) |
| 2 | Performance LFO + master effects (Reverb/Chorus) |
| 3 | 38x6 FM engine, waveform selection, detune |
| 4 | Per-operator F-Number, per-operator key-on |
| 5 | Parameter UI, preset save/load, GM2 Bank 0 program set via ML-based tone generation (`ym38x6-ml`) |
| 6 | VST3/CLAP plugin via nice-plug (optional) |
| 7 | Scale detection / avoid note handling |
| 8 | Tablet support (Tauri v2 iOS/Android) |
| 9 | Algorithm routing extension (SY77-style, optional) |

## Building

```powershell
# Check workspace
cargo check --workspace --message-format=short

# Run tests
cargo test -p sound-core
cargo test -p ym38x6-core

# Run app (first run compiles all dependencies, ~5 min)
cd gesture-app
npm install
npm run tauri dev
```

Requires: Rust (rustup), Node.js, WebView2 runtime (pre-installed on Windows 11).

## References

- [ymfm](https://github.com/aaronsgiles/ymfm) — OPQ/OPZ/OPN reference implementation (Aaron Giles, BSD 3-Clause)
- [PSR70-reverse](https://github.com/JKN0/PSR70-reverse) — OPQ programmer's guide and PSR-70 ROM2 voice/sound data (Jari Kangas)
- [MDSound / fmvgen](https://github.com/kuma4649/MDSound) — YM2609 emulator (kuma4649, C#) — the original implementation of Ryu Umemoto's fictional chip concept
- [YM2609](https://github.com/LTVA1/YM2609) — C++ port of the above (LTVA1, GPL-3.0)
