# 12. Surviving the platform: focus, lost surfaces, and oversized windows

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0003

## Context

Three platform behaviours bit during the first end-to-end run of `scripts/check.sh --full`, which
runs the windowed smoke test on Wayland and then on X11:

1. **X11 delivers a `Focused(false)` before any `Focused(true)`.** The "pause when the window loses
   focus" rule therefore paused a match that had just started and had never been looked at. The
   smoke test caught it because the scripted player locked zero pieces on X11 and the assertion
   `pieces > 0` fired.
2. **`get_current_texture` fails while a window is minimised or occluded** (`Lost`, `Occluded`,
   `Validation`). Treating the first failure as fatal would close the game whenever somebody
   minimises it — GNOME does this when you switch workspaces.
3. **Offscreen textures are capped by `Limits::max_texture_dimension_2d`.** Requesting
   `Limits::downlevel_defaults()` capped the scene target at 2048 px, so a 3200 px wide window
   produced a hard validation error instead of a picture.

## Decision

- **Focus:** the app tracks whether the window has *ever* held focus (`had_focus`). Auto-pause
  happens only after that, and never in `--smoke-test` mode. Pausing is safe — resume with `P` or
  Enter — so the rule stays, it just no longer fires on a spurious event.
- **Surface loss:** a failed `get_current_texture` increments a counter, reconfigures the surface
  from the window's current inner size, and skips the frame. Only after 240 consecutive failures
  (about four seconds of a window that refuses to come back) does the app close, and it says so on
  stderr. Success resets the counter.
- **Limits:** request `adapter.limits()` (what the device can really do) and clamp offscreen target
  dimensions to `max_texture_dimension_2d`. The composite samples the scene through normalised UVs,
  so a clamped target is a slightly softer picture rather than an error — verified by a test that
  renders at 3200×1800.
- **Resizing:** a resize is applied on `WindowEvent::Resized`, on `CurrentSurfaceTexture::Outdated`,
  and whenever the obtained image size disagrees with the configured size, whichever happens first.

## Consequences

- (+) Minimise, restore, drag between monitors and resize are all non-events.
- (+) The X11 and Wayland smoke tests now behave identically, which is what makes running both
  worthwhile — the divergence was a bug in *our* code, not in the backends.
- (−) A genuinely dead GPU (driver reset) means up to four seconds of black frames before exiting.
  Acceptable for a game, and the message on stderr says what happened.
- (−) `had_focus` is a heuristic: a compositor that hands focus out in an unusual order could still
  auto-pause once, at a moment when the player is not looking anyway.

## Alternatives rejected

- **Never auto-pause** — simpler, but a Tetris match left running in the background keeps dropping
  pieces onto a stack nobody is watching.
- **Exit on the first surface failure** — what we had; it turned workspace switches into crashes.
- **Fixed 2048 px targets with letterboxing** — avoids the clamp question by capping the whole
  renderer, which is a real quality loss on large displays.
- **Recreating the whole renderer on every resize** — correct but heavy: pipelines, the font atlas
  and bind groups would be rebuilt for a drag-resize gesture.
