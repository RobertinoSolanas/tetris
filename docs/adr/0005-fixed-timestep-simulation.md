# 5. Fixed timestep simulation decoupled from rendering

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0004

## Context

Tetris is a timing game: DAS at 133 ms, ARR at 20 ms, lock delay 500 ms, gravity 1 s → 0.02 s.
If gameplay advances by whatever `Instant::now()` says, then (a) rules depend on frame rate and
monitor refresh, (b) tests have to sleep, (c) a hitch on the GPU skips game time, (d) replays are
impossible. Rendering at the monitor's refresh rate is still desirable for smooth animation.

## Decision

- The simulation runs on a **fixed 120 Hz tick** (`TICK = 1/120 s`, i.e. ~8.33 ms granularity).
- The shell accumulates wall-clock delta and calls `Session::tick(Duration)` a bounded number of
  times per frame (max 8, to survive an Alt+Tab without a spiral of death); the remainder carries
  over.
- The domain only ever sees `Duration`s it is handed. Presentation interpolates with the leftover
  fraction (`alpha`) so motion stays smooth at 60/120/144 Hz.
- Input arrives as `Command`s tagged with the tick they were received on, so a run is a pure
  function of `(seed, [(tick, Command)])`.
- Pause stops ticking the simulation but keeps rendering (so overlays animate).

## Consequences

- (+) Rule tests assert exact behaviour without sleeping: `session.tick(ms(134))` either repeated
  the move or it did not.
- (+) Frame rate and game speed are independent; a 40 fps hiccup cannot change the outcome.
- (+) Replay/determinism test is meaningful (see ADR-0009).
- (−) Two clocks to reason about (sim tick vs render alpha); input latency gains at most one tick
  (8.3 ms), which is well under a frame at 60 Hz, so the trade is cheap.
- (−) `TICK` must stay small enough that 20 G gravity (20 ms/row) is still quantised sanely; 120 Hz
  gives 2.4 ticks per row at the top speed, which is acceptable and testable.

## Alternatives rejected

- **Variable timestep (`dt` straight from the clock)** — simplest, but makes every timing rule
  frame-rate dependent and untestable.
- **Render-lock (tick once per frame)** — ties game speed to refresh rate; on a 144 Hz panel the
  game would run 2.4× faster than on 60 Hz.
- **Fixed timestep with unbounded catch-up** — after a long stall this locks the machine; hence the
  iteration cap.
