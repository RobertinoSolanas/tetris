# 8. Audio cues synthesised in Rust, played through a subprocess sink

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0002

## Context

Sound is part of "fancy": a chunky lock, a rising rotate blip, the four-line payoff. The obvious
Rust routes are `cpal`/`rodio`/`alsa`/`libpulse-simple`, but every one of them binds a C library
(`alsa-sys`, `pulse-sys`) through `pkg-config` — unavailable here (no headers, no `.pc` files, no
`sudo`), see ADR-0003. We also want *no* sound files in the repo (ADR-0007).

## Decision

- **Synthesis:** `tetris-audio` generates PCM itself — envelopes over square/sine/triangle voices
  plus a white-noise burst, mixed to interleaved stereo `i16` at 44.1 kHz. Each `Cue` is a pure
  function `Cue → Vec<i16>`, peak-scaled and fade-limited so nothing clips.
- **Playback (port/adapter):** an `AudioSink` trait with three implementations:
  - `SubprocessSink` — spawns `pw-play --rate 44100 --channels 2 --format s16-le -` once (probed
    present on this host), falling back to `aplay -q -r 44100 -f S16_LE -c 2 -t raw -`, and writes
    frames to its stdin from a writer thread with a bounded ring buffer (drop oldest on overflow, so
    a blocked sink can never stall the game loop).
  - `WavFileSink` — test/diagnostics: dumps the mixed stream to a `.wav`.
  - `SilentSink` — used when no player binary exists, when `--no-sound` is passed, or when the pipe
    breaks.
- **Mapping:** domain events → cues (`Rotate`, `Move`, `Lock`, `Clear(1..4)`, `TSpin`, `LevelUp`,
  `Hold`, `GameOver`, `UiConfirm`, …). Mapping lives in the app layer, so the audio crate stays
  game-agnostic.

## Consequences

- (+) Zero C bindings, zero audio dev packages, zero asset files; the game is silent-but-correct on
  a machine without a player binary.
- (+) Cue generation is pure → unit-testable (length, peak, no NaN, determinism) without a device.
- (−) `pw-play`/`aplay` startup latency is tens of ms, and pipewire's own buffering adds a little
  latency; acceptable for a puzzle game, and the process is kept warm for the whole session.
- (−) Process-based audio is not a shipping-quality solution on desktops with strict sandboxing; it
  is a pragmatic adapter behind a trait — replacing it with `cpal` later touches one file.
- (−) Volume is applied in software (pre-scaling the stream) rather than per-application mixer gain.

## Alternatives rejected

- **`cpal`/`rodio`** — needs `alsa-sys` (pkg-config + headers) on Linux ⇒ build fails here.
- **Linking ALSA by hand with `extern "C"` + `dlopen`** — doable, but ~10× the code of the sink
  trait for the same audible result, and far more ways to get underruns.
- **No audio at all** — the cheapest option, but it costs part of the "fancy" the brief asks for;
  we keep it as the graceful fallback instead of the design.
