# 2. Native windowed application, not a terminal UI

Date: 2026-11-13 · Status: **accepted**

## Context

The brief: a *fancy* Tetris, written in Rust, that "runs as a stand-alone app, not in bash".
Rust has excellent terminal options (`crossterm`, `ratatui`, `termion`) which would be the cheapest
path, but a TUI is bound to a terminal emulator: no real frame pacing, no shaders, no glow, no
sub-pixel motion, no window identity, and it is not a stand-alone app in the sense asked for.

## Decision

The deliverable is a native GUI binary (`tetris`) that opens its own window with its own render
loop, event loop and lifecycle. No terminal required at run time. A terminal is only used to
*launch* it, exactly like any other desktop game.

Consequences for packaging:
- The binary must start with nothing but the system's runtime libraries available.
- CLI flags are still supported for automation: `--smoke-test`, `--snapshot <path>`, `--seed`,
  `--no-crt`, `--window <WxH>`.

## Alternatives rejected

- **TUI (`ratatui`/`crossterm`)** — cheap, but "in a terminal" is exactly what was ruled out; the
  visual ceiling (ANSI blocks) cannot deliver glow, particles or post-processing.
- **Web/webassembly** — needs a browser or a 200 MB webview runtime; not stand-alone.
- **GLES/`miniquad`/`macroquad`** — small and pleasant, but its Linux backends link X11/EGL
  symbols at build time, which fails in this environment (see ADR-0003).
