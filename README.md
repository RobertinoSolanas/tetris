# pi-tetris

A neon, arcade-styled **Tetris** written in Rust. It is a native windowed application — its own
window, its own render loop, its own GPU command stream — not a terminal game.

Everything is generated at run time: no textures, no fonts, no sound files, no asset directory.
The binary is the game.

```
cargo run --release -p tetris-app        # or: ./scripts/run.sh
```

---

## Contents

- [Playing](#playing) — controls, options
- [Building and running](#building-and-running) — requirements, CLI, what you should see
- [Verified on this machine](#verified-on-this-machine)
- [Architecture](#architecture) — bounded contexts, crates, data flow, the decisions behind them
- [Testing](#testing) — 268 tests, how to run them all
- [Where things live](#where-things-live) — data files, source map
- [Troubleshooting](#troubleshooting)
- [Documentation index](#documentation-index)

---

## Playing

| Key | Action |
|---|---|
| ← / → (or H / L) | Move (with DAS/ARR auto-shift while held) |
| ↓ | Soft drop (×20 gravity, 1 pt per cell) |
| **Space** | Hard drop (2 pt per cell) |
| ↑ / X | Rotate clockwise |
| Z | Rotate counter-clockwise |
| A | Rotate 180° |
| C / Shift | Hold |
| P / Esc | Pause (Esc on the pause menu abandons to the title) |
| R | Restart |
| F2 | Toggle CRT post-processing (scanlines, curvature, bloom) |
| F3 | Debug overlay (fps, sim time, backend, scene size) |
| M | Mute |
| Q | Quit, from the title screen |

Rules are Guideline-flavoured: **SRS** rotation with wall kicks (plus a 180° rotation), **7-bag**
randomiser, **hold**, **ghost piece**, 500 ms **lock delay** with a 15-move reset budget, DAS 133 ms /
ARR 20 ms, back-to-back, combos, T-spins (full and mini), perfect clears, 10 lines per level.
Scoring and gravity numbers live in one file:
[`crates/core/src/scoring.rs`](crates/core/src/scoring.rs).

Visual effects: per-block emissive rims and bevels, two-octre **bloom**, CRT **scanlines**,
barrel **curvature**, **vignette**, **chromatic aberration**, animated background grid, line-clear
flash with per-cell **particle** bursts, hard-drop **trails**, screen shake, combo / T-spin / level
toasts, an attract-mode title screen with a honours board, and a name-entry game-over screen.

### Command line

```
tetris [OPTIONS]

  --seed <N>          Deterministic piece sequence (replays are exact, see ADR-0009)
  --window <WxH>      Window size in physical pixels (default 1280x800)
  --no-crt            Disable scanlines/curvature/bloom
  --no-sound          Do not spawn an audio player
  --snapshot <FILE>   Render a gameplay frame and a title frame to PNG, then exit
  --smoke-test        Open a window, let the scripted player play, exit 0
  --frames <N>        Frames for --smoke-test (default 120)
  --dump-audio <FILE> Write the synthesised cues to a WAV file, then exit
  -h, --help          Usage
```

`--snapshot` is how you look at the game without playing it — useful on a headless box, and used by
the verification script:

```
cargo run --release -p tetris-app -- --snapshot /tmp/tetris.png
# wrote /tmp/tetris.png
# wrote /tmp/tetris-title.png
```

---

## Building and running

### Requirements

- **Rust** 1.87+ (developed on 1.99). Any recent stable works.
- A **Vulkan** 1.1+ driver. On Linux this is the only backend compiled in — see
  [ADR-0003](docs/adr/0003-winit-wgpu-vulkan-only.md).
- A Wayland **or** X11 session. Both are tested.
- Optional: `pw-play` (PipeWire) or `aplay` (ALSA) on `PATH` for sound. If neither exists the game
  runs silent; nothing else changes.

Notably **not** required: C headers, `pkg-config`, `.pc` files, `-dev` packages, SDL, or a terminal.
The dependency tree resolves its native libraries at run time (`libvulkan.so.1`,
`libwayland-client.so.0`, `libX11.so.6`) through `dlopen`, which is what makes the build work on a
machine that has runtime libraries only and no package-management privileges. That constraint came
from the environment this game was written in and drove the whole stack choice
([ADR-0003](docs/adr/0003-winit-wgpu-vulkan-only.md)).

### Build, run, check

```bash
cargo run --release -p tetris-app     # play
./scripts/run.sh                      # same thing, debug build (FASTER=1 for release)

cargo test --workspace                # 268 tests
./scripts/check.sh                    # fmt + clippy -D warnings + tests
./scripts/check.sh --full             # + release build, smoke tests on Wayland and X11, snapshot

cargo run --release -p tetris-app -- --smoke-test --frames 240
cargo run --release -p tetris-app -- --help
```

The debug build is pleasant to play (the simulation is a fixed 120 Hz and rendering is GPU-bound),
but use `--release` if you care about frame pacing.

---

## Verified on this machine

Run from `/home/openme/Documents/development/pi/tetris` on Ubuntu 26.10, GNOME/Mutter, AMD Mendocino
[Radeon 610M] with the RADV driver.

```
$ ./scripts/check.sh
── format ──  clean
── clippy ──  cargo clippy --workspace --all-targets -- -D warnings: clean
── tests  ──  268 passed; 0 failed; 0 ignored
── all good ──

$ cargo run --release -p tetris-app -- --smoke-test --frames 1200 --seed 7     # Wayland
audio: pw-play
renderer: AMD Radeon 610M (RADV RAPHAEL_MENDOCINO) (Vulkan) · surface Bgra8UnormSrgb
smoke test: 1200 frames at 61 fps | 5 match(es) | 273 pieces locked | 44 lines cleared
            | best score 2778 | final score 992 | screen Playing | 40 live particles

$ env -u WAYLAND_DISPLAY DISPLAY=:0 cargo run --release -p tetris-app -- --smoke-test
renderer: AMD Radeon 610M (RADV RAPHAEL_MENDOCINO) (Vulkan) · surface Bgra8UnormSrgb
smoke test: 90 frames at 60 fps | 1 match(es) | 19 pieces locked | 4 lines cleared …

$ cargo run --release -p tetris-app -- --snapshot /tmp/pi-tetris-snapshot.png
wrote /tmp/pi-tetris-snapshot.png
wrote /tmp/pi-tetris-snapshot-title.png
```

`--smoke-test` is not a no-op loop: a one-ply placement search drives the game through real
rotations, kicks, walks, locks, line clears, scoring, particles and game-over → restart, in a real
window on the real frame loop. When it reported `0 pieces locked` on X11, that was a genuine bug
(spurious focus-out auto-pausing a fresh match) — the story is written up in
[ADR-0012](docs/adr/0012-platform-robustness.md).

---

## Architecture

### Crates

```
crates/
├── core     tetris-core     gameplay domain — ZERO dependencies
├── render   tetris-render   Scene → pixels over wgpu/Vulkan (+ PNG writer, pixel font)
├── audio    tetris-audio    synthesised cues → pw-play/aplay (or silence)
└── app      tetris-app      shell, view model, scene builder, persistence, the `tetris` binary
    src/
      app.rs             winit ApplicationHandler, fixed-timestep loop, snapshot mode, bot
      scene_builder.rs   domain + view model → Scene   (the anti-corruption layer)
      view_model.rs      animation state: particles, trails, shake, flash, toasts, collapse
      layout.rs rect.rs theme.rs   pure layout/colour helpers
      input.rs           keyboard → Command/Action
      cli.rs             argument parsing
      json.rs persistence.rs        hand-rolled JSON, settings + high scores
docs/
├── adr/     twelve architecture decision records
└── domain/  ubiquitous language, bounded contexts and the context map
```

~13.5 k lines of Rust plus 317 lines of WGSL.

### Bounded contexts

Strategic design came first; the code follows it
([`docs/domain/bounded-contexts.md`](docs/domain/bounded-contexts.md),
[`docs/domain/ubiquitous-language.md`](docs/domain/ubiquitous-language.md)).

| # | Context | Where | Responsibility |
|---|---------|-------|----------------|
| 1 | Tetromino Geometry | `core::piece`, `core::kicks` | the seven shapes, rotation states, SRS kick tables |
| 2 | Playfield | `core::board` | the well: collision, locking, line clear, ghost |
| 3 | Piece Supply | `core::supply`, `core::rng` | 7-bag fairness, seeded entropy, preview queue |
| 4 | Scoring & Progression | `core::scoring` | every number in the game, level curve, gravity |
| 5 | Handling & Timing | `core::handling` | DAS/ARR, soft drop, lock delay + reset budget, input buffer |
| 6 | **Match Lifecycle** | `core::session` | the aggregate root: `Command` in, `GameEvent` out |
| 7 | Records & Settings | `app::persistence` | top-10 scores, settings, atomic JSON writes |
| 8 | Presentation | `tetris-render` | `Scene` → GPU, post-processing, PNG snapshot |
| 9 | Sound | `tetris-audio` | `Cue` → PCM → player subprocess (or silence) |
| 10 | Shell | `app::app` | window, input, frame scheduling, glue |

### Data flow

```
  keyboard ──► input::translate ──► Command ──► Session (aggregate root)
                                                    │
                                     GameEvent stream│
                                                    ├──────────────► audio cue mapping ─► Mixer ─► pw-play
                                                    ▼
                        ViewModel (particles, shake, flash, toasts, collapse)
                                                    │
                          Session state ────────────┤
                                                    ▼
                              scene_builder::build ─► Scene ─► tetris-render ─► screen
```

Two rules keep this honest, and both are enforced by the crate graph rather than by convention:

- **`tetris-core` has no dependencies at all** — no clock, no files, no global RNG, no threads. Time
  enters only as a `Duration` argument to `Session::tick`; entropy only through a seeded `Rng`
  ([ADR-0004](docs/adr/0004-dependency-free-domain-core.md),
  [ADR-0009](docs/adr/0009-determinism-by-construction.md)).
- **`tetris-render` has no idea what Tetris is.** It draws layered rectangles, glyphs and particles
  and nothing else; translating rows-and-columns into pixels and glow is the scene builder's job
  ([ADR-0006](docs/adr/0006-scene-as-anti-corruption-layer.md)).

### Frame structure

Simulation and rendering are decoupled ([ADR-0005](docs/adr/0005-fixed-timestep-simulation.md)):
the sim advances in fixed 1/120 s steps from an accumulator (max 8 steps per frame, so an Alt+Tab
cannot spiral), while the renderer draws once per `RedrawRequested` at whatever the compositor
offers. Presentation animation runs on wall-clock `dt`, so overlays keep breathing while paused.

The GPU frame graph ([ADR-0011](docs/adr/0011-per-pass-uniform-slots.md)):

```
Scene ─(instanced quads)─► scene texture (full res, Rgba16Float when available)
                              ├─(bright pass, ½ res)─► bloom A
                              │      └─ blur H/V, twice (step 2 then 6) ─► bloom A
                              └──────────────► composite (scanlines · curvature ·
                                               vignette · aberration · flash) ─► swapchain
```

### Decisions

| ADR | Decision | Why, in one line |
|-----|----------|------------------|
| [0001](docs/adr/0001-record-architecture-decisions.md) | Record decisions | rationale outlives the commit |
| [0002](docs/adr/0002-native-windowed-application.md) | Native window, not a TUI | "stand-alone app, not in bash" |
| [0003](docs/adr/0003-winit-wgpu-vulkan-only.md) | `winit` + `wgpu` (Vulkan only) | the only stack that builds without dev packages or sudo |
| [0004](docs/adr/0004-dependency-free-domain-core.md) | Dependency-free domain core | rule tests in ~1 s, determinism by construction |
| [0005](docs/adr/0005-fixed-timestep-simulation.md) | Fixed 120 Hz simulation | game speed independent of refresh rate; testable timing |
| [0006](docs/adr/0006-scene-as-anti-corruption-layer.md) | `Scene` as ACL | renderer stays dumb, animation stays testable |
| [0007](docs/adr/0007-zero-binary-assets.md) | Zero binary assets | procedural 8×8 pixel font + hand-rolled PNG writer |
| [0008](docs/adr/0008-audio-synthesis-with-subprocess-sink.md) | Synthesised audio → subprocess sink | no C audio bindings; graceful silence |
| [0009](docs/adr/0009-determinism-by-construction.md) | Determinism by construction | same seed + same tick-stamped log ⇒ same match |
| [0010](docs/adr/0010-testing-strategy.md) | Four-layer testing | rules, headless GPU, snapshots, windowed smoke test |
| [0011](docs/adr/0011-per-pass-uniform-slots.md) | Per-pass uniform slots | one buffer written per pass silently breaks the blur |
| [0012](docs/adr/0012-platform-robustness.md) | Focus / surface-loss policy | minimise and X11 focus quirks must not end a match |

---

## Testing

```bash
cargo test --workspace          # 268 tests
```

| Layer | What it covers | Tests |
|-------|----------------|-------|
| `tetris-core` unit | kick tables, collisions, bag fairness, every scoring rule, gravity curve, DAS/ARR/lock-delay state machines, session lifecycle, replay determinism, 12 000-tick soak | 131 |
| `tetris-core::tests/public_api` | the domain as an external consumer sees it: event ordering, bot survival, replay through the public API | 7 |
| `tetris-render` unit | scene/layer semantics, pixel-font coverage + atlas packing, PNG encode/decode round-trip | 28 |
| `tetris-render::tests/gpu` | **headless Vulkan**: every WGSL compiles, quads land where asked, text renders ink, bloom spreads, flash lifts, instance buffer grows, resize rebuilds | 11 |
| `tetris-audio` | every cue finite/bounded/normalised, determinism, mixer layering and voice cap, volume/mute, WAV output | 11 |
| `tetris-app` unit | JSON parser, settings/score persistence (corrupt file → defaults, atomic writes), layout maths, view-model animation, rectangles, theme contrast | 44 |
| `tetris-app::tests/app_flow` | the app headlessly: title → play → pause → restart → game over → name entry → high-score table, plus the scripted player | 13 |
| `tetris-app::tests/layout_probe` | renders real frames on the GPU and asserts the well, score panel, hold/next, wordmark and overlays are where they belong; CRT on vs off differs; 360×320 and 3200×1800 | 5 |
| `tetris-app::tests/cli_and_input` | every flag, every error path, every key mapping per screen, auto-repeat handling | 18 |

The GPU-dependent tests **skip with a printed reason** rather than fail where no Vulkan device
exists, so the suite stays green on a CI box without a GPU — but on a machine with one, they are
really drawing and reading pixels back.

Things deliberately not tested: winit's internals, and the exact hue of a given pixel under CRT
distortion (assertions are on relative luminance and region occupancy instead, because drivers
dither).

---

## Where things live

| Path | Contents |
|---|---|
| `$XDG_DATA_HOME/pi-tetris/settings.json` | CRT toggle, bloom, volume, mute, debug overlay, lines/level, player name, last seed |
| `$XDG_DATA_HOME/pi-tetris/scores.json` | top-10 high scores (name, score, lines, level, pieces, seconds) |
| (fallback) `$HOME/.local/share/pi-tetris/` | used when `$XDG_DATA_HOME` is unset |

Both files are written by a hand-rolled JSON writer
([`crates/app/src/json.rs`](crates/app/src/json.rs)), atomically (`*.json.tmp` then rename), and are
read back defensively: unreadable or out-of-range values become defaults, never a crash — losing a
settings file must not stop you playing.

---

## Troubleshooting

**"no compatible GPU adapter"** — pi-tetris needs a Vulkan driver. On Debian/Ubuntu that is
`mesa-vulkan-drivers` (or the vendor driver). Check with `vulkaninfo --summary` if it is installed.
Software rasterisation (lavapipe, `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/virtio_icd.json` or
`lvp_icd.x86_64.json`) also works, slowly. There is no OpenGL or software renderer path; adding one
means enabling wgpu's `gles` backend, which needs EGL at link time
([ADR-0003](docs/adr/0003-winit-wgpu-vulkan-only.md)).

**No sound** — the game looks for `pw-play`, then `aplay`. Install either, or run with
`--no-sound` to skip the attempt. Volume is software-side; if the player exists but the stream
stalls, cues are dropped rather than stalling the frame loop (`SubprocessSink::dropped()` counts it).

**Window is tiny on a HiDPI display** — the layout scales with `window.scale_factor()`; if your
compositor reports the wrong scale, size the window explicitly with `--window`.

**Text looks soft** — that is bloom; press <kbd>F2</kbd> to turn post-processing off, or lower
`bloom` in `settings.json`.

---

## Not in this version

Gamepad input, replay files on disk (the log exists in memory and is exercised by tests), ghost
races, themes, online scores, macOS/Windows backends (`metal`/`dx12` are one feature-flag away, not
an architectural change). See the backlog at the end of [`todo.md`](todo.md).

---

## Documentation index

- [`todo.md`](todo.md) — the working plan, in order, with what was actually done
- [`docs/domain/bounded-contexts.md`](docs/domain/bounded-contexts.md) — contexts and the context map
- [`docs/domain/ubiquitous-language.md`](docs/domain/ubiquitous-language.md) — glossary
- [`docs/adr/`](docs/adr/) — the twelve decisions summarised above

## Licence

MIT.
