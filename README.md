# op505

A fictional FM synthesizer and composition support app.

## Concept

**op505** is an imaginary FM sound chip with an N-point Time/Level envelope generator (rather than the classic rate-based ADSR found on real FM chips), implemented entirely in Rust. It succeeds an earlier design, **ym38x6** (based on YAMAHA's YM3806/OPQ with OPZ-inspired waveform extensions), which was retired in favor of op505's more flexible envelope model; op505's FM synthesis core (algorithms, waveforms, chip LFO) still traces back to that lineage.

The companion **composition app** lets anyone play musically coherent chords without music theory knowledge, using a calibration-based gesture UI with no grids or guides.

Inspired by Ryu Umemoto's YM-2609, which explored a similar "what if" premise using SynthEdit + VOPM.

## Architecture

Directories are groups; crate names follow `<group>-<part>`. Each group's main crate is named `core`.

```
(repo root)
  sound/              # Shared audio-layer foundation (product-agnostic)
    core/             # crate: sound-core — WaveTable, AdsrParams, PerformanceLfo, MasterEffects, Vco trait, TimeEg
    fm/               # crate: sound-fm — EG-agnostic parts shared across FM chips (algorithm, mapping, chip LFO)
  ui/                 # Shared UI-layer foundation (product-agnostic, egui)
    core/             # crate: ui-core — knobs, EG preview, TimeEg editor, algorithm diagram, param handles
    layout/           # crate: ui-layout — taffy-based panel layout solver (no egui dependency)
    codegen/          # crate: ui-codegen — panel XML DSL parser / IR / Rust code generator
  op505/              # The OP505 product (N-point Time/Level EG)
    core/             # OP505 FM engine implementation (crate: op505-core, depends on sound-core/sound-fm)
    ui/               # Editor panel definition (crate: op505-ui; src/panel.xml is the source of truth)
    vst/              # OP505 VST3/CLAP plugin (crate: op505-vst, nice-plug)
    midi/             # CC/NRPN interpretation shared by op505-vst and smf2op505 (crate: op505-midi)
    tools/            # Legacy chip converters, patch design & perf tools
  gesture-app/        # Composition app (Tauri v2, Windows desktop). Owns no engine or audio
                      # output; translates gestures into MIDI sent to op505-standalone.
    src/              # Frontend: calibration + gesture UI (HTML/JS)
    src-tauri/        # Backend: sends MIDI over a named pipe (midi_out.rs), Tauri commands
```

`sound-core`, `sound-fm`, and `op505-core` have zero dependencies on nice-plug, Tauri, or cpal. The audio engine is fully isolated.

## Sound Engine

### Waveform Memory Voice Bank (single-operator)

A dedicated voice bank — *not* an engine mode — where each voice uses only OP1 (Algorithm 7, OP2–4 muted at TL=0). This was implemented on the retired ym38x6 engine (`waveform_memory_patch`, reserved `WAVEFORM_MEMORY_BANK` Bank Select) but has not yet been ported to op505.

- Internal wave format: 1024 × u16, log encoding (ymfm-compatible)
  - `bit14~0`: −log₂|amplitude| in 4.8 fixed point
  - `bit15`: sign flag
- Built-in waveforms: 32 waveforms (slots 0–31) — the OPZ-derived sine set (0–7) plus saw/square/triangle extensions (8–31, not present on OPN/OPM/OPZ)
- User waveforms: 32 × i8 linear input → auto-converted to internal format (slots 32–255)

### OP505 FM Engine

4-operator FM synthesis with an N-point Time/Level envelope generator (`TimeEg`, up to 8 stages with optional loop) in place of the classic rate-based ADSR.

- 4op / channel, 8 algorithms
- Per-operator TimeEg (up to 8 stages, loop with drift) plus three channel-level function generators (Pitch/Cutoff/Gain FG) for modulation
- Per-operator frequency: each operator has an independent octave (3-bit) + F-Number (13-bit)
- All parameters 0–255 (8-bit unified); octave + F-Number (16-bit total) and MUL (4-bit) are the only exceptions
- **State Variable Filter** per voice: Cutoff (0–255, log scale), Resonance (0–255), Type (LP/HP/BP)
- Voice stealing above 64 simultaneous voices (score-based, oldest/quietest released first)

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
- **Warning playback** — play at reduced volume via per-operator key-on, giving musical feedback instead of a hard block

## Development Roadmap

See `spec-roadmap.md` for the full phase-by-phase plan. op505 has completed its VST3/CLAP plugin (DAW parameters + persisted TimeEg, NRPN/expression CC support), the direct legacy-chip converters (`op505/tools/*`), and the gesture-app integration (single-engine, OP505-only as of 2026-08-20).

## Building

```powershell
# Check workspace
cargo check --workspace --message-format=short

# Run tests
cargo test -p sound-core
cargo test -p op505-core

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
