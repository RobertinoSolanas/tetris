//! The scene: the renderer's entire vocabulary.
//!
//! A `Scene` says *"these translucent rectangles, these glowing blocks, these strings, in this
//! order, with this much shake"*. It contains no tetrominoes, no boards and no scores — translating
//! the domain into a `Scene` is the app's job (docs/adr/0006). Keeping the type free of GPU handles
//! means it can be built, inspected and unit-tested without a device.

use std::fmt;

/// The colour behind everything: a deep blue-black that reads as "arcade at night".
pub const DEFAULT_CLEAR: [f32; 4] = [0.015, 0.017, 0.035, 1.0];

/// Draw order, low to high. Quads are drawn layer by layer, then text on top.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    /// Gradient, grid, drifting background shapes.
    Background,
    /// The well: frame, floor lines, ghost piece.
    Playfield,
    /// Locked stack and the active piece.
    Pieces,
    /// Particles, flashes, drop trails, clear animation.
    Effects,
    /// Score, level, hold, next queue.
    Hud,
    /// Menu / pause / game-over panels and their text backdrops.
    Overlay,
}

impl Layer {
    pub const ALL: [Layer; 6] = [
        Layer::Background,
        Layer::Playfield,
        Layer::Pieces,
        Layer::Effects,
        Layer::Hud,
        Layer::Overlay,
    ];
}

/// How a quad's fragment shader shades it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QuadKind {
    /// Flat colour.
    Solid,
    /// Flat colour with rounded corners.
    Rounded,
    /// An arcade block: bevelled top-left highlight, darker body, emissive rim.
    Block,
    /// Hollow rounded rectangle.
    Outline,
    /// Vertical gradient from `color` to a darker version of it.
    Gradient,
    /// Disc / ring.
    Ring,
    /// Text glyph: samples the font atlas between `uv_min` and `uv_max`.
    Glyph,
}

/// A single rectangle in pixel space (origin top-left, y down).
#[derive(Copy, Clone, Debug)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Linear-ish RGBA, 0.0..=1.0.
    pub color: [f32; 4],
    /// Corner radius in pixels (used by `Rounded`, `Outline`, `Block`).
    pub radius: f32,
    /// Emissive strength: feeds the bloom bright-pass. 0.0 = no bloom.
    pub glow: f32,
    pub kind: QuadKind,
    /// Texture coordinates for `Glyph` (min xy, max xy); ignored otherwise.
    pub uv: [f32; 4],
    /// Procedural overlay: 0 none, 1 grid, 2 diagonal stripes, 3 dots, 4 scanlines.
    pub pattern: f32,
}

impl Quad {
    fn with_kind(x: f32, y: f32, w: f32, h: f32, color: [f32; 4], kind: QuadKind) -> Self {
        Self {
            x,
            y,
            w,
            h,
            color,
            radius: 0.0,
            glow: 0.0,
            kind,
            uv: [0.0; 4],
            pattern: 0.0,
        }
    }

    pub fn solid(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        Self::with_kind(x, y, w, h, color, QuadKind::Solid)
    }

    pub fn rounded(x: f32, y: f32, w: f32, h: f32, radius: f32, color: [f32; 4]) -> Self {
        let mut q = Self::with_kind(x, y, w, h, color, QuadKind::Rounded);
        q.radius = radius;
        q
    }

    /// A tetromino-shaped block (see [`QuadKind::Block`]).
    pub fn block(x: f32, y: f32, size: f32, color: [f32; 4]) -> Self {
        let mut q = Self::with_kind(x, y, size, size, color, QuadKind::Block);
        q.radius = size * 0.18;
        q.glow = 0.6;
        q
    }

    pub fn outline(x: f32, y: f32, w: f32, h: f32, radius: f32, color: [f32; 4]) -> Self {
        let mut q = Self::with_kind(x, y, w, h, color, QuadKind::Outline);
        q.radius = radius;
        q
    }

    pub fn gradient(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        Self::with_kind(x, y, w, h, color, QuadKind::Gradient)
    }

    pub fn ring(x: f32, y: f32, size: f32, color: [f32; 4]) -> Self {
        let mut q = Self::with_kind(x, y, size, size, color, QuadKind::Ring);
        q.glow = 0.8;
        q
    }

    /// A glyph sampled from the font atlas.
    pub fn glyph(x: f32, y: f32, w: f32, h: f32, color: [f32; 4], uv: [f32; 4]) -> Self {
        let mut q = Self::with_kind(x, y, w, h, color, QuadKind::Glyph);
        q.uv = uv;
        q
    }

    pub fn with_glow(mut self, glow: f32) -> Self {
        self.glow = glow;
        self
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn with_pattern(mut self, pattern: f32) -> Self {
        self.pattern = pattern;
        self
    }

    /// Multiply the alpha (useful for fades).
    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.color[3] *= alpha;
        self
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn centre_x(&self) -> f32 {
        self.x + self.w * 0.5
    }

    pub fn centre_y(&self) -> f32 {
        self.y + self.h * 0.5
    }
}

/// Horizontal alignment of a text run.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// A string to draw with the procedural pixel font.
///
/// The renderer lays it out (advance = 6 px per glyph at scale 1), so callers only choose the
/// anchor. Unknown characters are rendered as a hollow box rather than dropped, so a typo in a
/// string is visible instead of silently swallowing a space.
#[derive(Clone, Debug)]
pub struct Text {
    pub content: String,
    pub x: f32,
    pub y: f32,
    /// Integer-ish scale of the 8×8 glyphs.
    pub scale: f32,
    pub color: [f32; 4],
    pub glow: f32,
    pub align: Align,
    /// Extra pixels between glyphs.
    pub tracking: f32,
}

impl Text {
    pub fn new(content: impl Into<String>, x: f32, y: f32, scale: f32, color: [f32; 4]) -> Self {
        Self {
            content: content.into(),
            x,
            y,
            scale,
            color,
            glow: 0.0,
            align: Align::Left,
            tracking: 0.0,
        }
    }

    pub fn with_glow(mut self, glow: f32) -> Self {
        self.glow = glow;
        self
    }

    pub fn centred(mut self) -> Self {
        self.align = Align::Center;
        self
    }

    pub fn right_aligned(mut self) -> Self {
        self.align = Align::Right;
        self
    }

    pub fn with_tracking(mut self, tracking: f32) -> Self {
        self.tracking = tracking;
        self
    }
}

/// A particle: a tiny glowing square, drawn additively-ish on the effects layer.
#[derive(Copy, Clone, Debug)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub color: [f32; 4],
    pub glow: f32,
}

impl Particle {
    pub fn new(x: f32, y: f32, size: f32, color: [f32; 4]) -> Self {
        Self {
            x,
            y,
            size,
            color,
            glow: 1.0,
        }
    }
}

/// Post-processing switches and strengths, decided per frame by the app.
#[derive(Copy, Clone, Debug)]
pub struct PostFx {
    /// Bloom strength (0 disables the bloom chain entirely).
    pub bloom: f32,
    /// Luminance above which pixels contribute to bloom.
    pub bloom_threshold: f32,
    /// CRT scanlines strength.
    pub scanlines: f32,
    /// Barrel distortion amount (0 = flat).
    pub curvature: f32,
    /// Vignette strength.
    pub vignette: f32,
    /// Chromatic aberration in pixels.
    pub aberration: f32,
}

impl Default for PostFx {
    fn default() -> Self {
        Self::arcade()
    }
}

impl PostFx {
    /// The look the game ships with.
    pub const fn arcade() -> Self {
        Self {
            bloom: 0.85,
            bloom_threshold: 0.55,
            scanlines: 0.22,
            curvature: 0.035,
            vignette: 0.35,
            aberration: 0.7,
        }
    }

    /// Everything off — useful for screenshots and for reading small text.
    pub const fn off() -> Self {
        Self {
            bloom: 0.0,
            bloom_threshold: 1.0,
            scanlines: 0.0,
            curvature: 0.0,
            vignette: 0.0,
            aberration: 0.0,
        }
    }
}

/// One frame's worth of drawing instructions.
#[derive(Clone, Debug)]
pub struct Scene {
    /// Logical size in pixels (usually the window's physical size).
    pub width: f32,
    pub height: f32,
    /// Quads per layer, indexed by `Layer::ALL`.
    layers: [Vec<Quad>; 6],
    pub text: Vec<Text>,
    pub particles: Vec<Particle>,
    /// Screen shake amplitude in pixels; the renderer applies it to everything but the overlay.
    pub shake: f32,
    /// Full-screen additive flash, 0.0..=1.0.
    pub flash: f32,
    pub post: PostFx,
    /// Colour the scene target is cleared to before any quad is drawn (RGBA 0..1).
    pub clear_color: [f32; 4],
}

impl Default for Scene {
    fn default() -> Self {
        Self::new(1280.0, 800.0)
    }
}

impl Scene {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            layers: std::array::from_fn(|_| Vec::new()),
            text: Vec::new(),
            particles: Vec::with_capacity(512),
            shake: 0.0,
            flash: 0.0,
            post: PostFx::default(),
            clear_color: crate::scene::DEFAULT_CLEAR,
        }
    }

    /// Empty the scene but keep the allocations for the next frame.
    pub fn clear(&mut self) {
        for layer in self.layers.iter_mut() {
            layer.clear();
        }
        self.text.clear();
        self.particles.clear();
        self.shake = 0.0;
        self.flash = 0.0;
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
    }

    pub fn push(&mut self, layer: Layer, quad: Quad) {
        self.layers[layer as usize].push(quad);
    }

    pub fn extend(&mut self, layer: Layer, quads: impl IntoIterator<Item = Quad>) {
        self.layers[layer as usize].extend(quads);
    }

    pub fn text(&mut self, text: Text) {
        self.text.push(text);
    }

    pub fn particle(&mut self, p: Particle) {
        self.particles.push(p);
    }

    /// Quads in a given layer.
    pub fn layer(&self, layer: Layer) -> &[Quad] {
        &self.layers[layer as usize]
    }

    /// Total quads, particles and text runs — handy for HUD debug lines and for tests.
    pub fn counts(&self) -> SceneCounts {
        SceneCounts {
            quads: self.layers.iter().map(|l| l.len()).sum(),
            particles: self.particles.len(),
            text: self.text.len(),
        }
    }

    /// Every quad in strict layer order (what the renderer submits).
    pub fn in_order(&self) -> Vec<&Quad> {
        let mut out = Vec::with_capacity(self.counts().quads);
        for layer in self.layers.iter() {
            out.extend(layer.iter());
        }
        out
    }
}

/// Number of primitives in a scene.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneCounts {
    pub quads: usize,
    pub particles: usize,
    pub text: usize,
}

impl fmt::Display for SceneCounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} quads, {} particles, {} text runs",
            self.quads, self.particles, self.text
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(x: f32) -> Quad {
        Quad::solid(x, 0.0, 1.0, 1.0, [1.0, 0.0, 0.0, 1.0])
    }

    #[test]
    fn layers_draw_in_declaration_order() {
        for pair in Layer::ALL.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} should draw before {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(Layer::ALL.first(), Some(&Layer::Background));
        assert_eq!(Layer::ALL.last(), Some(&Layer::Overlay));
    }

    #[test]
    fn in_order_flattens_layers_and_respects_their_order() {
        let mut s = Scene::new(800.0, 600.0);
        s.push(Layer::Overlay, q(9.0));
        s.push(Layer::Background, q(1.0));
        s.push(Layer::Hud, q(7.0));
        s.push(Layer::Playfield, q(3.0));

        let flat: Vec<f32> = s.in_order().iter().map(|quad| quad.x).collect();
        assert_eq!(flat, vec![1.0, 3.0, 7.0, 9.0]);
    }

    #[test]
    fn clear_empties_everything_but_keeps_the_size_fields() {
        let mut s = Scene::new(1024.0, 768.0);
        s.push(Layer::Pieces, q(1.0));
        s.text(Text::new("SCORE", 0.0, 0.0, 2.0, [1.0; 4]));
        s.particle(Particle::new(1.0, 2.0, 3.0, [1.0; 4]));
        s.shake = 4.0;
        s.flash = 0.5;
        assert_eq!(s.counts().quads, 1);

        s.clear();
        assert_eq!(
            s.counts(),
            SceneCounts {
                quads: 0,
                particles: 0,
                text: 0
            }
        );
        assert_eq!(s.shake, 0.0);
        assert_eq!(s.flash, 0.0);
        assert_eq!(s.width, 1024.0);
        assert_eq!(s.height, 768.0);
        assert_eq!(s.post.bloom, PostFx::default().bloom);
        assert_eq!(s.clear_color, DEFAULT_CLEAR);
    }

    #[test]
    fn builders_set_the_fields_they_claim_to() {
        let b = Quad::block(10.0, 20.0, 32.0, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(b.kind, QuadKind::Block);
        assert!(b.radius > 0.0 && b.radius < 32.0);
        assert!(b.glow > 0.0, "blocks should bloom");

        let r = Quad::solid(0.0, 0.0, 4.0, 4.0, [1.0; 4])
            .with_glow(0.5)
            .with_radius(2.0)
            .with_pattern(3.0)
            .with_alpha(0.5);
        assert_eq!(r.glow, 0.5);
        assert_eq!(r.radius, 2.0);
        assert_eq!(r.pattern, 3.0);
        assert_eq!(r.color[3], 0.5);

        let g = Quad::glyph(0.0, 0.0, 8.0, 8.0, [1.0; 4], [0.0, 0.0, 0.1, 0.1]);
        assert_eq!(g.kind, QuadKind::Glyph);
        assert_eq!(g.uv, [0.0, 0.0, 0.1, 0.1]);
    }

    #[test]
    fn geometry_helpers_are_correct() {
        let r = Quad::solid(10.0, 20.0, 100.0, 50.0, [1.0; 4]);
        assert_eq!(r.right(), 110.0);
        assert_eq!(r.bottom(), 70.0);
        assert_eq!(r.centre_x(), 60.0);
        assert_eq!(r.centre_y(), 45.0);
    }

    #[test]
    fn alpha_never_goes_negative_from_a_fade() {
        let q = Quad::solid(0.0, 0.0, 1.0, 1.0, [1.0, 1.0, 1.0, 1.0]).with_alpha(0.0);
        assert_eq!(q.color[3], 0.0);
    }

    #[test]
    fn text_builders_are_chainable() {
        let t = Text::new("TETRIS", 100.0, 20.0, 3.0, [1.0; 4])
            .centred()
            .with_glow(0.7)
            .with_tracking(1.0);
        assert_eq!(t.align, Align::Center);
        assert_eq!(t.glow, 0.7);
        assert_eq!(t.tracking, 1.0);
        assert_eq!(t.scale, 3.0);

        let r = Text::new("999999", 500.0, 0.0, 2.0, [1.0; 4]).right_aligned();
        assert_eq!(r.align, Align::Right);
    }

    #[test]
    fn post_fx_presets_are_sane() {
        let a = PostFx::arcade();
        let o = PostFx::off();
        assert!(a.bloom > 0.0 && a.bloom_threshold > 0.0 && a.bloom_threshold <= 1.0);
        assert!(a.scanlines > 0.0 && a.scanlines < 1.0);
        assert!(a.curvature >= 0.0 && a.curvature < 0.2);
        assert!(a.vignette >= 0.0 && a.vignette <= 1.0);
        assert_eq!(o.bloom, 0.0);
        assert_eq!(o.scanlines, 0.0);
        assert_eq!(o.curvature, 0.0);
        assert_eq!(o.aberration, 0.0);
    }

    #[test]
    fn resize_clamps_to_something_the_gpu_accepts() {
        let mut s = Scene::new(100.0, 100.0);
        s.resize(0.0, -5.0);
        assert_eq!(s.width, 1.0);
        assert_eq!(s.height, 1.0);
    }

    #[test]
    fn counts_report_every_primitive() {
        let mut s = Scene::new(640.0, 480.0);
        s.extend(Layer::Effects, [q(1.0), q(2.0), q(3.0)]);
        s.particle(Particle::new(0.0, 0.0, 1.0, [1.0; 4]));
        s.text(Text::new("x", 0.0, 0.0, 1.0, [1.0; 4]));
        let c = s.counts();
        assert_eq!((c.quads, c.particles, c.text), (3, 1, 1));
        assert!(c.to_string().contains("3 quads"));
    }
}
