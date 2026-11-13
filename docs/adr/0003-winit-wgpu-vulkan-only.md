# 3. `winit` + `wgpu` with the Vulkan backend only

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0002

## Context

Constraints measured on the target machine before writing any code:

- Ubuntu 26.10, Wayland session (GNOME/Mutter) with XWayland also reachable (`DISPLAY=:0`).
- AMD Radeon 610M with `/dev/dri/renderD128` granted to the user via ACL; Mesa Vulkan ICD present.
- **No `-dev` packages, no headers, no `pkg-config`, no `.pc` files, no `libEGL.so` / `libGLESv2.so`
  / `libvulkan.so` development symlinks.**
- **`sudo` requires interactive authentication** ⇒ we cannot install anything. The build must work
  with what is on disk.

That eliminates any dependency whose `build.rs` needs a header, a `.pc` file, or `-l<lib>` against a
dev symlink. Runtime `dlopen()` of an existing `libfoo.so.N` is still perfectly fine.

## Decision

Rendering + windowing stack:

- `winit = "0.30"` — window and input. Its Wayland backend (`wayland-backend` with
  `client_system`) and X11 backend (`x11-dl`) plus `xkbcommon-dl` all resolve their native
  libraries at **runtime**, so no headers or link-time symlinks are needed.
- `wgpu = "30"` with **`default-features = false, features = ["std", "vulkan", "wgsl"]`**.
  `ash` loads `libvulkan.so.1` through `libloading`, and `naga` compiles our WGSL in-process — no
  C toolchain involvement.
  Dropping the default `gles` feature is not cosmetic: the GLES backend wants EGL/GLX at link time
  and would break the build in this environment.
- WGSL shaders, one instanced quad pipeline plus a small post-FX chain, no mesh/format libraries,
  no asset files.

Verified by spike before committing: instance → adapter `Vulkan` → device → surface on Wayland,
31 frames presented, clean exit; same binary works with `WAYLAND_DISPLAY` unset against `DISPLAY=:0`.

## Consequences

- (+) Pure-Rust build graph; `cargo build` works on a machine with zero dev packages.
- (+) Vulkan-only means one shader dialect, one set of quirks, and hardware bloom/compute headroom.
- (+) Works on both of the host's display servers with no feature flags.
- (−) No macOS/Windows/iOS support out of the box (backends compiled out). Enabling `metal`/`dx12`
  and `gles` is a feature-list change, not an architectural one.
- (−) Needs a Vulkan-capable GPU/driver (lavapipe fallback would also do). A `--headless` offscreen
  mode exists for tests and snapshots; a software path is documented in the README rather than
  implemented.
- (−) `wgpu` 30 has a young API (`Surface::get_current_texture` returning `CurrentSurfaceTexture`,
  `Queue::present`, `multiview_mask` in pass descriptors). We pin `"30"` and adapt on upgrade.

## Alternatives rejected

- **`wgpu` with default features** — pulls `gles` → link-time EGL → fails here.
- **`glutin` + `glow`/OpenGL** — needs EGL/GLX at link time (or `libGL.so` symlink) and gives us a
  compatibility profile with fewer modern conveniences than WGSL.
- **`macroquad`/`miniquad`, `piston`, `sdl2`, `bevy`** — link C libraries (X11/EGL/SDL) or are far
  heavier than a Tetris needs.
- **Wayland + shared-memory software rendering written from scratch** — no C deps, but weeks of
  protocol work and no GPU effects; the shader work is the "fancy" part of the brief.
- **`softbuffer`/`minifb` + CPU drawing** — feasible, but loses bloom/CRT/post FX and burns CPU.
