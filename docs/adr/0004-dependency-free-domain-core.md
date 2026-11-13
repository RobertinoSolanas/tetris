# 4. Dependency-free domain core in its own crate

Date: 2026-11-13 · Status: **accepted** · Depends on: bounded-contexts map

## Context

Almost every Tetris bug lives in rules: wall kicks, lock-delay reset budgets, scoring chains,
bag fairness. Those rules are testable only if the code that holds them can be exercised without
a window, a GPU, an audio device or a wall clock. In a single-crate binary that "just runs"
everything, tests quietly become integration tests and the fast feedback loop is lost.

## Decision

A Cargo workspace with four crates and a strict dependency direction:

```
tetris-core    (no dependencies at all — not even serde/rand/log)
   ▲
tetris-audio   tetris-render        (independent leaves; render deps on wgpu, audio on std)
   ▲            ▲
   └── tetris-app (binary `tetris`; deps on winit + the three above)
```

- `tetris-core` contains the six gameplay contexts: geometry+kicks, playfield, supply,
  scoring/progression, handling/timing, session lifecycle.
- No `Instant`, no file I/O, no environment access, no thread-locals, no global mutable state, no
  RNG other than the injected `Rng` — enforced by the fact that the crate has no dependencies.
- The domain never calls into rendering or audio. It returns `GameEvent`s; the app fans them out.

## Consequences

- (+) `cargo test -p tetris-core` links a tiny binary — the rule tests run in well under a second,
  so they get run.
- (+) Determinism is a structural property: same seed + same `(tick, Command)` log ⇒ same state.
  There is a test asserting it, which also protects replays.
- (+) GPU/audio/window concerns can be replaced or broken without touching gameplay rules.
- (−) More `Cargo.toml` files and one `use` line per boundary crossing.
- (−) Shared visual constants (per-tetromino colour) must live where both sides can see them:
  `tetris-core` exposes a `palette` of RGB tuples, the render crate stays colour-agnostic.

## Alternatives rejected

- **Single crate with modules only** — cheaper to set up, but nothing stops domain code from
  reaching for `wgpu` or `Instant`; the boundary is a convention instead of a compile error.
- **`serde` in the core for save states** — persistence maps `Stats` primitives instead; keeping
  the core dependency-free is worth more than deriving 8 traits.
- **`rand` in the core** — `rand`'s algorithm is not part of its API contract, so replays could
  break on a minor version bump. We own a tiny `XorShift64` behind an `Rng` trait instead.
