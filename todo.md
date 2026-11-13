# TODO — `pi-tetris`: a fancy native Tetris in Rust

> Living checklist. Legend: `[ ]` todo · `[~]` in progress · `[x]` done.
> Work order is deliberate: **recon → strategic design (bounded contexts) → scaffolding →
> pure domain + tests → rendering → shell → juice → hardening → docs**.

---

## 0. Recon & feasibility

- [x] Inventory toolchain: `cargo 1.99.1`, `rustc 1.99.1`, gcc present.
- [x] Inventory display: Wayland session (`wayland-0`) **and** XWayland (`DISPLAY=:0`),
      GNOME/Mutter, AMD Mendocino [Radeon 610M], `/dev/dri/renderD128` accessible via ACL.
- [x] Inventory system libraries: **runtime only — no `-dev` headers, no `pkg-config`, no `.pc`
      files, no `libEGL.so`-style dev symlinks** — and `sudo` needs interactive auth ⇒
      **cannot apt-install anything**.
      Hard constraint for the stack: no build-time C linking, no headers/`.pc` files. Runtime
      `dlopen` of `libvulkan.so.1` / `libwayland-client.so.0` / `libX11.so.6` is fine.
- [x] Feasibility spike (`/tmp/spike`): `winit 0.30` + `wgpu 30` (default features off,
      `std,vulkan,wgsl` on) → window created, adapter = **Vulkan**, 31 frames presented, exit 0.
- [x] Spike re-run with `WAYLAND_DISPLAY` unset, `DISPLAY=:0` → also works (X11 backend).
- [x] Audio probe: `pw-play` + `aplay` present and accept raw S16_LE on stdin ⇒ optional SFX
      via subprocess, no ALSA/Pipewire Rust bindings.
- [x] Decisions recorded: ADR-0003 (winit + wgpu/Vulkan), ADR-0008 (audio via subprocess).

## 1. Strategic design — bounded contexts first

- [x] Write the ubiquitous language (`docs/domain/ubiquitous-language.md`).
- [x] Map bounded contexts + context map (`docs/domain/bounded-contexts.md`):
      1. **Tetromino Geometry** — shapes, rotation states, SRS kick tables.
      2. **Playfield** — grid, collision, locking, line clearing, ghost projection.
      3. **Piece Supply** — 7-bag, seeded deterministic RNG, preview queue.
      4. **Scoring & Progression** — clear values, T-spin, combo, back-to-back, perfect clear,
         level curve, gravity.
      5. **Match Lifecycle** — session aggregate: Ready → Playing → Paused → GameOver + events.
      6. **Handling & Timing** — DAS/ARR, soft-drop factor, lock delay + reset cap, input buffer.
      7. **Records & Settings** — high scores, settings file (infrastructure context).
      8. **Presentation** — scene description, GPU pipelines, post FX.
      9. **Sound** — domain event → synthesised cue.
      10. **Shell** — window, input events, frame scheduling.
- [x] Define relationships: domain core = **upstream**; Presentation/Sound/Shell = **downstream**
      consumers of the domain event stream; **Scene Builder** = anti-corruption layer between the
      domain's units (row/col/level) and the renderer's units (pixels/glow).
- [x] Record ADRs 0001–0012 (`docs/adr/`) — two more (uniform slots, platform robustness) were
      added once the implementation surfaced them.

## 2. Scaffolding

- [x] Cargo workspace: `crates/core` (`tetris-core`, **zero dependencies**), `crates/render`
      (`tetris-render`), `crates/audio` (`tetris-audio`), `crates/app` (`tetris-app`, bin `tetris`).
- [x] `wgpu = "30", default-features = false, features = ["std", "vulkan", "wgsl"]`
      (keeps `gles`/EGL out of the link step).
- [x] `.gitignore`, `rustfmt.toml`, `clippy.toml`, `scripts/run.sh`, `scripts/check.sh`.
- [x] Verify an empty workspace builds and `cargo test` runs.

## 3. Domain: Tetromino Geometry

- [x] `Cell`, `Rotation` (Spawn/CW/180/CCW), `Tetromino` enum with SRS spawn shapes.
- [x] Per-rotation cell offsets (single source of truth: spawn shape + rotate rule), cross-checked
      against the SRS tables.
- [x] SRS wall-kick tables: JLSTZ + separate I table, plus 180 kicks.
- [x] Tests: 7 shapes × 4 rotations cell counts & occupancy; kick table shape/order (test 1 is the
      no-op); O never needs a kick; 180 table symmetric.

## 4. Domain: Playfield

- [x] `Board` 10 wide × (20 visible + 2 buffer above), occupancy grid with colour index.
- [x] `fits()` collision, `lock()`, `full_rows()`, `remove_rows()`, `cell()`, `is_empty()`.
- [x] `ghost_row()` (hard-drop projection), `spawn_blocked()` (top-out rule).
- [x] Tests: walls/floor/existing-blocks collisions; single & multi row detection; removal keeps
      stack intact; ghost == lowest legal row; blocked spawn ⇒ top-out.

## 5. Domain: Piece Supply

- [x] `Rng` trait + `XorShift64` (deterministic, seedable).
- [x] `SevenBag`: shuffle a bag of seven, expose `next()` and `preview(n)`.
- [x] Tests: every bag holds exactly one of each piece; same seed ⇒ same sequence; different
      seeds differ; bounded distance between repeats (≤ 13); `preview` does not consume.

## 6. Domain: Scoring & Progression

- [x] `ClearKind` (None/Single/Double/Triple/Tetris), `SpinKind` (None/Mini/Full), perfect clear.
- [x] Guideline-flavoured `ScoreTable`: clear values, T-spin values, back-to-back ×1.5, combo
      cascade, soft/hard drop points, perfect-clear bonus, level multiplier.
- [x] Gravity curve (Guideline formula), lines-per-level goal, level-up detection.
- [x] `ScoreState`: combo counter, b2b chain, what counts as a "hard clear".
- [x] Tests: hand-computed values (Single@L1 = 100, Tetris@L1 = 800, B2B Tetris = 1200,
      T-Spin Double = 1200, combo = 50·combo·level, perfect clear = 800→2000·level), gravity
      monotonicity, level up every 10 lines.

## 7. Domain: Handling & Timing (game feel)

- [x] `HandlingConfig`: DAS 133 ms, ARR 20 ms, soft-drop factor 2×, lock delay 500 ms,
      15 move-resets, 100 ms input buffer — pure data, no wall clock inside the domain.
- [x] `AutoShift` state machine (tap → delay → repeat) driven by `tick(dt)`.
- [x] `LockTimer` with move-reset cap; `DropTimer` for gravity + soft drop.
- [x] Tests: tap moves once; hold past DAS repeats at ARR; a step resets the counter and re-triggers
      DAS; lock delay expiry locks; reset cap reached ⇒ instant lock; soft drop multiplies gravity.

## 8. Domain: Match Lifecycle (aggregate root)

- [x] `Session` aggregate root: board, bag, active piece, hold, score, level, timers, stats.
- [x] `Command` enum (move/rotate/drop/hold/pause/restart/…) + `GameEvent` stream.
- [x] T-spin detection (3-corner rule, kick override, mini vs full).
- [x] Lock → clear → spawn pipeline, top-out detection, hold swap once per piece, queue refill.
- [x] Deterministic `tick(dt)`; guarded state transitions; `Stats` (pieces, lines, time, tetrises,
      max combo, score).
- [x] Tests: happy path lifecycle; hold twice rejected; rotate resets lock delay; top-out emits
      `GameOver`; same seed + same scripted commands ⇒ identical state hash (replay determinism);
      1 000-piece soak run with no panic/overflow.

## 9. Presentation: scene description + renderer

- [x] `Scene` (layered quads, text runs, particles, screen shake, flash) = renderer's only input;
      the renderer knows **nothing** about Tetris.
- [x] wgpu setup: Vulkan instance, offscreen target, swapchain, resize/reconfigure.
- [x] One instanced quad pipeline + WGSL: solid / rounded / bevelled block / outline / vertical
      gradient / ring, per-instance glow.
- [x] Procedural pixel font: a hand-authored 5×7 table packed into 8×8 atlas cells, rasterised at
      startup (no asset files, no font crates).
- [x] Post FX: bright-pass → two rounds of separable blur at ½ res → composite with CRT scanlines,
      barrel distortion, vignette, chromatic aberration and flash; togglable at runtime (F2).
      Each pass reads its own slot of one uniform buffer (ADR-0011).
- [x] PNG snapshot mode (`tetris --snapshot out.png`) for headless visual verification.
- [x] Tests: all WGSL compiles; offscreen frames land where asked (quad position, layer order,
      glyph ink, flash, buffer growth, resize); font coverage and gutter packing; bloom raises
      luminance around a bright quad; PNG round-trips and rejects corruption.

## 10. Shell: winit event loop

- [x] `ApplicationHandler`; window title/size; resize ⇒ reconfigure surface + relayout.
- [x] Fixed timestep simulation (120 Hz) + accumulator; render each frame; `ControlFlow::Poll`.
- [x] Keyboard map (screen-sensitive, auto-repeat filtered); pause on focus loss *after* the
      window has held focus (ADR-0012); quit on close; name entry on the high-score screen.
- [x] `--smoke-test`: opens a window and lets a scripted one-ply player actually play — locks,
      clears, scores, particles, game over, restart — then exits 0, asserting pieces were locked.

## 11. Sound

- [x] Pure-Rust SFX synthesiser (square/sine/noise envelopes) → interleaved S16LE frames.
- [x] Sink: spawn `pw-play` (fallback `aplay`) once and stream frames; silent no-op otherwise.
- [x] Event → cue mapping (move, rotate, lock, clear, tetris, level up, hold, game over, UI).
- [x] Tests: cue length bounded, peak-limited (no clipping), no NaN, deterministic per cue.

## 12. Records & settings

- [x] `Store` under `$XDG_DATA_HOME/pi-tetris/` (`scores.json`, `settings.json`), atomic writes.
- [x] Top-10 high score table + name entry; settings: CRT toggle, bloom, volume.
- [x] Tests: round-trip, corrupt file ⇒ defaults, ordering & truncation to 10.

## 13. Game feel / juice pass

- [x] Line-clear animation: white flash → row shatter into particles → eased stack collapse.
- [x] Hard-drop after-image trail + screen shake scaled by drop distance.
- [x] Ghost piece outline, lock-delay pulse, level-up banner, combo/B2B/T-spin toasts.
- [x] Animated background (drifting grid + gradient + vignette), glowing HUD panels.
- [x] Title screen with animated falling tetrominoes, a controls panel and the honours board.

## 14. Hardening & verification

- [x] `cargo fmt --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean.
- [x] `cargo test --workspace` green (unit + headless GPU + integration).
- [x] `--smoke-test` exits 0 on Wayland **and** with `DISPLAY` only (X11).
- [x] Headless app-flow check of the same paths (title → play → pause → restart → game over →
      name entry → high-score table) plus the live windowed smoke test. A human with a keyboard has
      *not* played it — noted honestly.
- [x] `--snapshot` PNGs written and verified numerically (region luminance/occupancy probes in
      `layout_probe.rs`); the model driving this build cannot view images, so assertions replaced
      eyeballing.
- [x] Release build (`--release`) runs at sane frame pacing.
- [x] Backlog, explicitly out of scope for v1: gamepad input, replay files on disk, ghost
      races, themes, macOS/Windows backends (a feature flag away, not an architecture change).

## 15. Documentation

- [x] `README.md`: what/why, build & run, controls, architecture + ADR summary, testing, and the
      "no dev headers / no sudo" constraint and how the stack dodges it.
- [x] `docs/domain/*`, `docs/adr/*` linked from the README.
- [x] Final pass: README matches the code as shipped (flags, paths, module names, 268 tests,
      measured smoke-test output on both display servers).

---

## 16. Findings during implementation (added after the fact; the plan stays honest)

- **The X11 focus-out bug.** `check.sh --full` failed on the X11 smoke run only: X11 delivers
  `Focused(false)` before any `Focused(true)`, so "pause on blur" froze a match nobody had seen.
  Fixed by tracking `had_focus` (ADR-0012). Running the same smoke test on both display servers is
  worth the ten seconds it costs.
- **Per-pass writes to one uniform buffer are a trap.** Writing the post uniform before each of the
  six passes collapsed the bloom blur into a copy — every pass saw the *last* value written. Fixed
  with one buffer, six 256-byte slots and dynamic offsets (ADR-0011). The probe that caught it: a
  bright quad produced 0 luminance 14 px away, where a halo belonged.
- **`Limits::downlevel_defaults()` caps textures at 2048 px**, which turned a 3200×1800 layout probe
  into a validation error. Now `adapter.limits()` is requested and offscreen targets are clamped.
- **`wgpu` 30 API churn**: `create_surface` and `get_current_texture` are no longer `async`, the
  latter returns `CurrentSurfaceTexture` rather than a `Result`, pipeline layouts take
  `bind_group_layouts: &[Option<&…>]` plus `immediate_size`, and passes need `multiview_mask`.
  Pin the version and read the vendored crate sources rather than writing from memory.
- **WGSL details that cost time**: no `float` alias (use `f32`), `mod` is reserved (use `%`), no
  assignment through a swizzle (`colour.rgb *= x` → rebuild the `vec4`), and a raw string opened with
  `r#"` must be closed with `"#`.
- **Test-count drift.** The plan predicted a "1 000-piece soak"; what exists is a 12 000-tick
  deterministic soak that restarts on game over (the live smoke run locks 273 pieces), which covers
  more paths than the original phrasing.
