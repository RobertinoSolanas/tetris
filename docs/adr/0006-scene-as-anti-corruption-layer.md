# 6. A `Scene` anti-corruption layer between domain and renderer

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0004

## Context

The temptation in a small game is to let the renderer `use tetris_core::*` and draw `Board` cells
directly: `for cell in board.occupied() { draw(cell, COLORS[piece]) }`. That works until the first
animation: a clearing row needs a 200 ms flash, the stack has to collapse with easing, particles
must spawn from the cleared cells, and the HUD wants interpolated values. Suddenly the renderer
owns animation state, timing, layout *and* GPU resources, and every domain refactor breaks draw code.

## Decision

`tetris-render` exposes exactly one input type: `Scene`, a flat, immediate-mode description of the
frame.

```rust
pub struct Scene {
    pub layers: Vec<Layer>,      // ordered: Background, Playfield, Pieces, Effects, Hud, Overlay
    pub shake: f32,              // screen shake amplitude in px
    pub flash: f32,              // full-screen flash intensity
    pub post_fx: PostFx,         // bloom/crt/vignette switches + strengths
}
pub struct Quad { pos, size, color_rgba, uv_min, uv_max, radius, glow, kind, pattern }
pub struct TextRun { text, pos, scale, color_rgba, glow, letter_spacing }
pub struct Particle { pos, size, color_rgba, glow }
```

Between the two sits the **Scene Builder** (`tetris-app::scene`), an anti-corruption layer:

- domain + view-model state in, `Scene` out;
- translates grid units → pixels (cell size computed from the window size, layout, scale factor);
- resolves game colours, glow, animation progress into concrete quads;
- owns *layout* (playfield centred, HUD panels left/right, hold/next placement) and *animation*.

The renderer never learns that Tetris exists: no `Tetromino`, no `Board`, no `Score`, no `Level`.
It has one quad pipeline, a text pipeline over a generated atlas, and a post-FX chain.

## Consequences

- (+) `SceneBuilder` is pure CPU code → fully unit-testable: "a board with a 4-wide gap at row 19
  produces N quads at these pixel rects". No GPU, no window.
- (+) The renderer is reusable for anything quad-shaped and independently testable with synthetic
  scenes (the headless GPU tests and PNG snapshots draw synthetic scenes).
- (+) Screens (title, pause, game over) are just different scenes.
- (−) One extra data structure and a per-frame quad rebuild (~a few hundred quads — trivial; we
  cap the instance buffer and rebuild in < 100 µs).
- (−) Per-frame allocation risk; mitigated by reusing scratch `Vec`s inside `SceneBuilder`.

## Alternatives rejected

- **Renderer draws domain objects directly** — see Context; couples the GPU layer to the aggregate
  and pushes animation into draw calls.
- **Retained scene graph with dirty flags** — the state we would track is bigger than the rebuild,
  and it makes "what does frame 4123 look like?" harder to test.
- **A GUI toolkit (`egui`/`iced`)** — fast for HUDs, but its immediate-mode widgets fight the
  full-screen shader look we want, and it would own the frame.
