# 9. Determinism by construction: seeded RNG, tick-stamped input log

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0004, ADR-0005

## Context

"Same game, same inputs, same result" is what lets us test timing rules and scoring end-to-end, and
it is what makes replays (and later, ghost races) possible at all. Determinism bolted on afterwards
never holds: one stray `Instant::now()` or `HashMap` iteration order and it is gone.

## Decision

- `tetris-core` defines `trait Rng { fn next_u64(&mut self) -> u64 }` with one implementation,
  `XorShift64` (seed 0 mapped to a non-zero constant). All randomness in the game flows through a
  seeded `Rng` that is owned by the `Session`.
- `Session::new(seed, config)` produces a fully reproducible run. There is no other source of
  entropy: no `SystemTime`, no `HashMap` iteration affecting gameplay, no global state.
- Input is recorded as `InputLog: Vec<(u64 tick, Command)>` inside the session, in `tick` order,
  and can be exported. `Session::replay(seed, &log)` rebuilds the run.
- A state fingerprint (`Session::fingerprint() -> u64`, FNV-1a over board, score, level, stats,
  phase, piece positions) makes equality cheap to assert.
- The app accepts `--seed <u64>` so a run can be re-created from the command line.

## Consequences

- (+) The replay test is a one-liner with real teeth:
      play run A, replay its log, assert fingerprints equal — catches accidental nondeterminism in
      any rule change.
- (+) Bug reports become reproducible: "seed 12345, tick 40213, RotateCw".
- (+) Property-style soak tests (1 000 pieces, random commands) are stable and debuggable: a failure
  prints the seed and is reproducible exactly.
- (−) The domain must resist convenience: "just measure how long the player thought" is not allowed
  inside the core; time enters only as `dt` at `tick`.
- (−) `XorShift64` is not cryptographically strong and has visible structure in some low-bit tests —
  irrelevant for a 7-bag shuffle, and the bag fairness test bounds the consequences.

## Alternatives rejected

- **`rand` with a seeded `StdRng`** — fine algorithm, but it puts a dependency in the dependency-free
  core (ADR-0004) and makes replay stability hostage to a transitive version bump.
- **Recording wall-clock timestamps instead of ticks** — replays would drift with frame rate and
  break the fixed-timestep guarantee (ADR-0005).
- **Determinism "tested later"** — the cost is paid per rule if deferred; we assert it from the first
  commit that has a session.
