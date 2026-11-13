# 10. Testing strategy: pure rule tests, headless GPU tests, snapshot and smoke modes

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0003 … ADR-0009

## Context

A GUI game with a GPU is usually the point where a project's test suite dies: the interesting code
(what is on screen, does the window open) seems to need human eyes, while the testable code (rule
maths) risks being the only thing covered. This machine also has no CI runner, no display server for
a headless GL context, and no way to install test tooling.

## Decision

Four layers, each chosen so it can run unattended on this host:

1. **Rule tests — `tetris-core`** (the bulk): table-driven unit tests for kicks, collisions, bag
   fairness, scoring arithmetic (hand-computed expectations), gravity curve, DAS/ARR/lock-delay
   state machines, plus session lifecycle tests that drive `tick(dt)` + `Command`s. A soak test runs
   1 000 pieces with a seeded random policy, and a determinism test replays logs (ADR-0009).
2. **Headless GPU tests — `tetris-render`**: create instance/adapter/device with **no surface**,
   render synthetic `Scene`s into offscreen textures, read pixels back with
   `copy_texture_to_buffer` and assert on them (background colour, quad present at the right pixel,
   bloom brightening neighbouring pixels, text glyph coverage). Creating the shader modules in these
   tests also proves every WGSL shader compiles/validates.
3. **Visual snapshot mode** — `tetris --snapshot out.png [--seed N]` renders real gameplay scenes
   off-screen and writes a PNG (ADR-0007), so a person (or an agent reading the image) can verify
   layout, glow and post-FX without a window. Used during development to inspect the look.
4. **Smoke test** — `tetris --smoke-test [--frames N]` opens a real window, runs the full loop
   (simulate → build scene → render → present), and exits 0. Run on Wayland **and** with only
   `DISPLAY=:0` set to exercise both winit backends.
5. **`scripts/check.sh`** runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test --workspace` and the smoke test — the same gate a CI job would use, runnable here.

## Consequences

- (+) Rules are covered where they can actually break; nothing in the core is untested for
  convenience.
- (+) Shader regressions fail the build (WGSL validation errors surface at shader-module creation).
- (+) "Does the window still come up?" is a command with an exit code, not a hope.
- (−) The GPU tests need a Vulkan driver and a render node; they skip (with a printed reason) rather
  than fail where none exists — so on such a machine the suite is green but shallower.
- (−) Reading swapchain-sized textures back is slow-ish; the tests render small (256×256) targets.
- (−) Pixel assertions are tolerance-based (±2 per channel) because formats are `Bgra8UnormSrgb` and
  dithering/backend rounding can differ.

## Alternatives rejected

- **Only rule tests, no GPU tests** — leaves the largest, most driver-sensitive surface unverified.
- **Frame-limited screenshots diffed against golden PNGs** — brittle across drivers/refresh rates;
  we assert *properties* of pixels instead of exact whole-image hashes.
- **`wgpu` `noop` backend for tests** — fast, but validates nothing about the real path (no shader
  compilation on Vulkan, no readback semantics).
- **Manual play-testing only** — necessary for feel, useless as a regression net.
