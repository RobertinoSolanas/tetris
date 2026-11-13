//! `tetris-render` — the presentation context.
//!
//! One input type, [`Scene`], and one output: pixels. The crate knows about rectangles, glyphs,
//! particles and post-processing effects, and nothing about tetrominoes, boards or scores; the
//! translation from the domain into a scene happens in `tetris-app` (docs/adr/0006).
//!
//! Everything is generated at run time — the font ([`font`]) is authored as pixel art in the
//! source, and [`png`] is a hand-rolled writer — so the built binary has no assets to find
//! (docs/adr/0007).

pub mod font;
pub mod png;
pub mod renderer;
pub mod scene;

pub use font::{Font, ADVANCE, CELL, GLYPH_H, GLYPH_W, LINE_HEIGHT};
pub use png::{encode_rgba, write_rgba};
pub use renderer::{
    build_instances, Instance, RenderError, Renderer, MAX_INSTANCES, SNAPSHOT_FORMAT,
};
pub use scene::{
    Align, Layer, Particle, PostFx, Quad, QuadKind, Scene, SceneCounts, Text, DEFAULT_CLEAR,
};
