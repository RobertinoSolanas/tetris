# Bounded Contexts & Context Map

Strategic design first, code second. Ten contexts, one upstream core, thin downstream adapters.

```
                        ┌──────────────────────────────────────────────┐
   keyboard/mouse ───►  │  SHELL (crates/app::shell)                   │  window, input, frame clock
                        └───────────────┬──────────────────────────────┘
                                        │ Command (intent)          ▲ RenderFrame / Present
                                        ▼                           │
                        ┌───────────────────────────────┐           │
                        │  MATCH LIFECYCLE  (aggregate) │           │
                        │  crates/core::session         │           │
                        └───┬───────────┬───────────┬───┘           │
        uses (upstream)     │           │           │               │
        ┌───────────────────┘           │           └────────────┐  │
        ▼                               ▼                        ▼  │
┌───────────────────┐   ┌────────────────────────┐   ┌──────────────────┐
│ TETROMINO GEOMETRY│   │  PLAYFIELD             │   │ SCORING &        │
│ core::tetromino   │   │  core::board           │   │ PROGRESSION      │
│ + SRS kicks       │   │  collision/clear/ghost │   │ core::scoring    │
└───────────────────┘   └────────────────────────┘   └──────────────────┘
        ▲                               ▲
        │            ┌──────────────────┴───────────┐   ┌──────────────────┐
        └────────────│  PIECE SUPPLY                │   │ HANDLING & TIMING│
                     │  core::supply (7-bag, RNG)   │   │ core::handling   │
                     └──────────────────────────────┘   └──────────────────┘

   MATCH LIFECYCLE ──emit──► GameEvent stream ──┬──► VIEW MODEL (app) ──► SCENE BUILDER ──► PRESENTATION (crates/render)
                                               ├──► SOUND (crates/audio)
                                               └──► RECORDS (app::persistence) ◄── settings/store
```

## 1. Tetromino Geometry (`tetris-core::tetromino`, `tetris-core::kicks`)

- **Responsibility:** the intrinsic shape language — the seven tetrominoes, their four rotation
  states, and the SRS wall-kick tables.
- **Model:** `Tetromino`, `Rotation`, `Cell`, `Kick`, pure functions (`cells`, `kick_attempts`).
- **No knowledge of** boards, scores, windows or time. Changes to a shape are impossible to hide:
  every dependent test fails loudly.
- **Upstream** (shared kernel with Playfield: both speak `Cell`).

## 2. Playfield (`tetris-core::board`)

- **Responsibility:** the well — 10 × 20 + 2 buffer rows; "does this piece fit here?", "lock it",
  "which rows are full?", "where would it land?"
- **Model:** `Board`, `fits`, `lock`, `full_rows`, `remove_rows`, `ghost_row`, `spawn_blocked`.
- **Explicitly not:** deciding *when* to lock (Handling), what a clear is *worth* (Scoring), or
  how a clear should *look* (Presentation).

## 3. Piece Supply (`tetris-core::supply`)

- **Responsibility:** fairness and reproducibility of piece sequence: 7-bag randomisation, seeded
  deterministic RNG, preview queue.
- **Model:** `Rng` (port), `XorShift64` (adapter), `SevenBag`, `PieceSupply`.
- **Contract:** same seed ⇒ byte-identical sequence. This is what makes replays and deterministic
  tests possible.

## 4. Scoring & Progression (`tetris-core::scoring`)

- **Responsibility:** value and difficulty — clear/T-spin/perfect-clear values, combo, back-to-back,
  level curve, gravity, goal.
- **Model:** `ClearKind`, `SpinKind`, `ScoreTable`, `ScoreState`, `Progression`.
- **Pure:** `ScoreTable::value_of(clear, spin, perfect, state, level)` — no state mutation inside;
  the session applies it. All numbers live here and nowhere else.

## 5. Handling & Timing (`tetris-core::handling`)

- **Responsibility:** game feel as data + state machines: DAS/ARR auto-shift, soft drop factor,
  lock delay with a move-reset budget, gravity accumulator.
- **Model:** `HandlingConfig`, `AutoShift`, `LockTimer`, `DropTimer`.
- **Rule:** never reads a clock. The shell passes `dt`; the domain advances. This is why the
  "does holding left repeat at 20 ms?" test needs no sleeping.

## 6. Match Lifecycle (`tetris-core::session`) — **core / aggregate root**

- **Responsibility:** the lifecycle of one match and the consistency boundary between all of the
  above: spawn → input → move/rotate → land → lock → clear → score → next piece → level up →
  top out.
- **Model:** `Session` (aggregate root), `Phase`, `Command` (in), `GameEvent` (out), `ActivePiece`,
  `Stats`, `InputLog`.
- **Only** this context emits events; downstream contexts never touch `Board` mutably.

## 7. Records & Settings (`tetris-app::persistence`)

- **Responsibility:** durable facts across runs — top-10 high scores, settings (CRT/bloom/volume).
- **Model:** `ScoreEntry`, `ScoreBoard`, `Settings`, `Store` (atomic write to
  `$XDG_DATA_HOME/pi-tetris/`).
- **Depends on:** nothing but `serde`-free hand-rolled JSON to keep the tree small. It stores
  primitives; it does not know what a T-spin is beyond an integer.

## 8. Presentation (`tetris-render`)

- **Responsibility:** turn a `Scene` into pixels: instanced quad pipeline, procedural pixel-font,
  bloom + CRT composite, swapchain management, PNG snapshot.
- **Model:** `Scene`, `Quad`, `TextRun`, `Particle`, `PostFx`, `Renderer`.
- **Ignorance is the point:** this crate has no `Tetromino`, no `Board`, no `Score`. Give it quads
  and it draws them; that is the whole contract (ADR-0006).

## 9. Sound (`tetris-audio`)

- **Responsibility:** `Cue` → PCM samples → playback through a spawned `pw-play`/`aplay`.
- **Model:** `Cue`, `Synth` (envelopes: square/sine/noise), `AudioOut` (sink port;
  `SubprocessSink`, `SilentSink`).
- **Failure mode:** no player binary ⇒ `SilentSink`, game unaffected.

## 10. Shell (`tetris-app::shell`)

- **Responsibility:** platform glue: winit `ApplicationHandler`, window + resize, keyboard
  translation into `Command`, fixed-timestep accumulator, pause-on-blur, CLI flags, smoke test.
- **Owns no rules of its own** — it translates and schedules.

## Context map summary

| Relationship | Type | Mechanism |
|---|---|---|
| Geometry → Playfield | Shared kernel | `Cell`, `Rotation` |
| Supply, Scoring, Handling → Lifecycle | Upstream (libraries of the core) | direct calls, all pure |
| Lifecycle → Presentation | Upstream/downstream + **ACL** | `GameEvent` stream → `ViewModel` → `SceneBuilder` → `Scene` |
| Lifecycle → Sound | Customer/supervisor | `GameEvent` → `Cue` mapping |
| Lifecycle → Records | Downstream read-only projection | `Stats` → `ScoreEntry` |
| Shell → Lifecycle | Command dispatcher | `Command` in, `dt` in, events out |

## Why the core has zero dependencies

`tetris-core` depends on nothing at all (no `serde`, no `rand`, no `log`). Consequences that we
accept and exploit:

1. `cargo test -p tetris-core` compiles in ~1 s — the feedback loop for rules stays instant.
2. No version-drift risk in the part of the system with the most invariants.
3. Determinism is structural, not a promise: there is no ambient clock, no global RNG, no I/O.
4. GPU/audio/window crates are swappable (or absent) without touching gameplay rules.
