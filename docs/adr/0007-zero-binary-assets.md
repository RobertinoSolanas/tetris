# 7. Zero binary assets: procedural pixel font and hand-rolled PNG writer

Date: 2026-11-13 · Status: **accepted** · Depends on: ADR-0003

## Context

"Ship a single binary that runs on a machine where I cannot install anything" implies: no runtime
asset lookup that can fail. Text is the usual problem — a real font needs a `.ttf`, a rasteriser and
an atlas builder (`ab_glyph`, `cosmic-text`, `fontdue`), and an arcade game does not need TrueType
hinting anyway. The PNG snapshot mode (`--snapshot out.png`, used to verify visuals head-lessly)
would otherwise drag in an image crate.

## Decision

- **Font:** a hand-authored 8 × 8 bitmap font for printable ASCII (uppercase + digits +
  punctuation + a few box glyphs), stored as `[[u8; 8]; 95]` bitmaps in
  `crates/render/src/font5x7.rs`-style source data. At startup we rasterise it into one
  `R8`-style RGBA atlas texture; text is drawn with the same instanced quad pipeline by pointing
  each glyph's UV at its atlas cell. Crisp at integer scales, on-theme for an arcade game, and it
  adds a whole extra knob of juice: per-glyph glow feeding the bloom pass.
- **PNG:** `crates/render/src/png.rs` writes 8-bit RGBA PNGs directly — zlib stream of
  *uncompressed* DEFLATE stored blocks, `Adler32`, `CRC32`, filter byte 0 per scanline.
  ~120 lines, no dependencies; snapshot files are a few MB, which is irrelevant for their purpose.
- **Everything else visual** (blocks, panels, ghost, particles, background, glow, CRT) is generated
  by shaders. No textures, no model files, no audio files: SFX are synthesised (ADR-0008).

## Consequences

- (+) The binary is self-contained: copy it anywhere with a Vulkan driver and it runs, no `assets/`
  directory, no `include_bytes!` blobs, no working-directory assumptions.
- (+) `cargo test -p tetris-render` can assert font coverage (all printable ASCII present) and PNG
  round-trip properties without any fixture files.
- (+) Deterministic pixel output → snapshot tests are reproducible.
- (−) Uppercase-flavoured 8×8 glyphs only; no font fallback, no emoji, no text measurement beyond
  advance = 8 × scale. Fine for HUD numbers and short labels.
- (−) A hand-rolled encoder must be correct; it is covered by tests (signature, dimensions, CRC,
  and the fact that `file`/image viewers open the output — verified during the build of this game).

## Alternatives rejected

- **`ab_glyph` / `fontdue` + a `.ttf`** — nicer glyphs, but a binary asset in the repo, a rasteriser
  in the frame path, and a look that fights the pixel aesthetic.
- **`image`/`png` crate for snapshots** — pulls a large dependency graph into the workspace purely
  for a debug tool.
- **BMP/PPM instead of PNG** — trivially simple, but not viewable/clickable in most toolchains; PNG
  with stored blocks is barely harder and universally openable.
