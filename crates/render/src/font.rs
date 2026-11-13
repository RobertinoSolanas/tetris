//! A hand-authored 5×7 dot-matrix font, rasterised into an atlas at startup.
//!
//! There is no `.ttf` anywhere in this project (docs/adr/0007): the glyphs below *are* the font, in
//! a form you can read with your eyes. Each line is `<char>|<7 rows of 5 pixels>`, `#` = ink.
//!
//! The atlas packs glyphs into 8×8 cells (5×7 ink plus a one-pixel right and bottom gutter, so
//! nearest-neighbour sampling can never bleed a neighbour in). Text is drawn with the same quad
//! pipeline as everything else, which is what lets a glyph carry `glow` into the bloom pass.

use crate::scene::{Align, Quad};

/// Atlas cell size in pixels (ink is 5×7, the rest is gutter).
pub const CELL: u32 = 8;
/// Ink width of a glyph.
pub const GLYPH_W: u32 = 5;
/// Ink height of a glyph.
pub const GLYPH_H: u32 = 7;
/// Cells per atlas row.
pub const COLUMNS: u32 = 16;
/// Advance between glyph origins at scale 1.0 (5 ink + 1 gap).
pub const ADVANCE: f32 = 6.0;
/// Height of a line of text at scale 1.0.
pub const LINE_HEIGHT: f32 = 8.0;

/// The glyph table. Order defines the atlas layout, and `Font::build` asserts it is well formed.
const ART: &str = r#"
 |.....|.....|.....|.....|.....|.....|.....
!|..#..|..#..|..#..|..#..|..#..|.....|..#..
" |.#.#.|.#.#.|.#.#.|.....|.....|.....|.....
#|.#.#.|.#.#.|#####|.#.#.|#####|.#.#.|.#.#.
%|##..#|##.#.|..#..|.#...|#.##.|..###|.....
&|.##..|#..#.|#.#..|.#...|#.#.#|#..#.|.##.#
'|..#..|..#..|.#...|.....|.....|.....|.....
(|...#. |..#..|.#...|.#...|.#...|..#..|...#.
)|.#...|..#..|...#. |...#. |...#. |..#..|.#...
*|.....|#.#.#|.###.|#####|.###.|#.#.#|.....
+|.....|..#..|..#..|#####|..#..|..#..|.....
,|.....|.....|.....|.....|..#..|..#..|.#...
-|.....|.....|.....|#####|.....|.....|.....
.|.....|.....|.....|.....|.....|.##..|.##..
/|....#|....#|...#. |..#..|.#...|#....|#....
0|.###.|#...#|#..##|#.#.#|##..#|#...#|.###.
1|..#..|.##..|..#..|..#..|..#..|..#..|.###.
2|.###.|#...#|....#|..##.|.#...|.#...|#####
3|#####|...#. |..#..|...#. |....#|#...#|.###.
4|...#. |..##.|.#.#.|#..#. |#####|...#. |...#.
5|#####|#....|####.|....#|....#|#...#|.###.
6|..##.|.#...|#....|####.|#...#|#...#|.###.
7|#####|....#|...#. |..#..|.#...|.#...|.#...
8|.###.|#...#|#...#|.###.|#...#|#...#|.###.
9|.###.|#...#|#...#|.####|....#|...#. |.##..
:|.....|.##..|.##..|.....|.##..|.##..|.....
;|.....|.##..|.##..|.....|.##..|..#..|.#...
<|...#. |..#..|.#...|#....|.#...|..#..|...#.
=|.....|.....|#####|.....|#####|.....|.....
>|.#...|..#..|...#. |....#|...#. |..#..|.#...
?|.###.|#...#|....#|...#. |..#..|.....|..#..
@|.###.|#...#|#.###|#.#.#|#.###|#....|.###.
A|.###.|#...#|#...#|#####|#...#|#...#|#...#
B|####.|#...#|#...#|####.|#...#|#...#|####.
C|.###.|#...#|#....|#....|#....|#...#|.###.
D|####.|#...#|#...#|#...#|#...#|#...#|####.
E|#####|#....|#....|####.|#....|#....|#####
F|#####|#....|#....|####.|#....|#....|#....
G|.###.|#...#|#....|#.###|#...#|#...#|.###.
H|#...#|#...#|#...#|#####|#...#|#...#|#...#
I|.###.|..#..|..#..|..#..|..#..|..#..|.###.
J|..###|...#. |...#. |...#. |...#. |#..#. |.##..
K|#...#|#..#. |#.#..|##...|#.#..|#..#. |#...#
L|#....|#....|#....|#....|#....|#....|#####
M|#...#|##.##|#.#.#|#.#.#|#...#|#...#|#...#
N|#...#|##..#|##..#|#.#.#|#..##|#..##|#...#
O|.###.|#...#|#...#|#...#|#...#|#...#|.###.
P|####.|#...#|#...#|####.|#....|#....|#....
Q|.###.|#...#|#...#|#...#|#.#.#|#..#. |.##.#
R|####.|#...#|#...#|####.|#.#..|#..#. |#...#
S|.####|#....|#....|.###.|....#|....#|####.
T|#####|..#..|..#..|..#..|..#..|..#..|..#..
U|#...#|#...#|#...#|#...#|#...#|#...#|.###.
V|#...#|#...#|#...#|#...#|#...#|.#.#.|..#..
W|#...#|#...#|#...#|#.#.#|#.#.#|##.##|#...#
X|#...#|#...#|.#.#.|..#..|.#.#.|#...#|#...#
Y|#...#|#...#|.#.#.|..#..|..#..|..#..|..#..
Z|#####|....#|...#. |..#..|.#...|#....|#####
[|..###|..#..|..#..|..#..|..#..|..#..|..###
\|#....|#....|.#...|..#..|...#. |....#|....#
]|###..|..#..|..#..|..#..|..#..|..#..|###..
^|..#..|.#.#.|#...#|.....|.....|.....|.....
_|.....|.....|.....|.....|.....|.....|#####
°|.##..|.#..#|.##..|.....|.....|.....|.....
→|.....|..#..|...#. |#####|...#. |..#..|.....
"#;

/// Which cell a character maps to in the atlas.
#[derive(Clone, Debug)]
pub struct Font {
    /// ASCII code → cell index (`u32::MAX` when absent).
    cells: Vec<u32>,
    /// Atlas pixels, RGBA8.
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    glyphs: usize,
}

impl Default for Font {
    fn default() -> Self {
        Self::build()
    }
}

impl Font {
    /// Rasterise the glyph table. Panics only if the table above is malformed (a unit test guards
    /// that, so a panic here means somebody edited `ART` and broke it).
    pub fn build() -> Self {
        let mut cells = vec![u32::MAX; 128];
        let mut order: Vec<char> = Vec::new();

        for (line_no, raw) in ART.lines().enumerate() {
            let line = raw.trim_end();
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.split('|');
            let head = parts.next().unwrap_or("").trim_end();
            // The space glyph's row starts with a single space, which trim removes; treat an empty
            // head as exactly that.
            let ch = head.chars().next().unwrap_or(' ');
            assert!(
                head.chars().count() <= 1,
                "ART line {}: head {:?} must be at most one character",
                line_no + 2,
                head
            );
            let rows: Vec<&str> = parts.collect();
            assert_eq!(
                rows.len(),
                GLYPH_H as usize,
                "ART line {}: '{}' needs {} rows, found {}",
                line_no + 2,
                ch,
                GLYPH_H,
                rows.len()
            );

            let index = order.len() as u32;
            if (ch as u32) < 128 {
                cells[ch as usize] = index;
            }
            order.push(ch);
        }

        let glyphs = order.len();
        let cols = COLUMNS;
        let rows = glyphs.div_ceil(cols as usize) as u32;
        let width = cols * CELL;
        let height = rows * CELL;
        let mut pixels = vec![0u8; (width * height * 4) as usize];

        // Re-walk the table and paint each glyph into its cell.
        for (glyph_index, raw) in ART
            .lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.trim().is_empty())
            .enumerate()
        {
            let mut parts = raw.split('|');
            let ch = parts.next().and_then(|h| h.chars().next()).unwrap_or(' ');
            let cell_x = (glyph_index as u32 % cols) * CELL;
            let cell_y = (glyph_index as u32 / cols) * CELL;

            for (row, bits) in parts.enumerate() {
                // Trailing spaces are only there so the alignment of the table stays readable.
                let bits = bits.trim_end();
                assert!(
                    bits.len() <= GLYPH_W as usize,
                    "ART: glyph '{ch}' row {row} is {} px wide, max is {}",
                    bits.len(),
                    GLYPH_W
                );
                for (col, px) in bits.chars().enumerate() {
                    let ink = match px {
                        '#' => 255,
                        '.' => 0,
                        other => {
                            panic!("ART: glyph '{ch}' row {row} has '{other}', use '#' or '.'")
                        }
                    };
                    let x = cell_x + col as u32;
                    let y = cell_y + row as u32;
                    let i = ((y * width + x) * 4) as usize;
                    // White ink; the shader tints it with the quad colour.
                    pixels[i] = 255;
                    pixels[i + 1] = 255;
                    pixels[i + 2] = 255;
                    pixels[i + 3] = ink;
                }
            }
        }

        Self {
            cells,
            pixels,
            width,
            height,
            glyphs,
        }
    }

    /// Atlas width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Atlas height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Number of glyphs in the table.
    pub const fn glyph_count(&self) -> usize {
        self.glyphs
    }

    /// RGBA8 atlas pixels, row-major, 4 bytes per pixel.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Cell index of a character. Lowercase folds to uppercase; anything missing maps to `?` so a
    /// typo shows up as a box instead of eating a character.
    pub fn cell_of(&self, ch: char) -> Option<u32> {
        let folded = ch.to_ascii_uppercase();
        if (folded as u32) < 128 {
            let cell = self.cells[folded as usize];
            if cell != u32::MAX {
                return Some(cell);
            }
        }
        let placeholder = self.cells['?' as usize];
        if placeholder == u32::MAX {
            None
        } else {
            Some(placeholder)
        }
    }

    /// `[min_u, min_v, max_u, max_v]` of a glyph's ink inside the atlas.
    pub fn uv(&self, ch: char) -> Option<[f32; 4]> {
        let cell = self.cell_of(ch)?;
        let cx = (cell % COLUMNS) as f32 * CELL as f32;
        let cy = (cell / COLUMNS) as f32 * CELL as f32;
        // Inset by a fraction of a pixel so nearest-neighbour sampling stays inside the ink cell.
        let inset = 0.001;
        let u0 = (cx + inset) / self.width as f32;
        let v0 = (cy + inset) / self.height as f32;
        let u1 = (cx + GLYPH_W as f32 - inset) / self.width as f32;
        let v1 = (cy + GLYPH_H as f32 - inset) / self.height as f32;
        Some([u0, v0, u1, v1])
    }

    /// Width of `text` in pixels: `glyphs × (ADVANCE + tracking) × scale − (tracking × scale)`.
    pub fn measure(&self, text: &str, scale: f32, tracking: f32) -> f32 {
        let n = text.chars().count() as f32;
        if n <= 0.0 {
            return 0.0;
        }
        (n * (ADVANCE + tracking) - tracking) * scale
    }

    /// Height of one line of text.
    pub fn line_height(&self, scale: f32) -> f32 {
        LINE_HEIGHT * scale
    }

    /// Turn a string into glyph quads. `(x, y)` is the top-left of the first glyph's cell (adjusted
    /// horizontally for alignment).
    pub fn layout(
        &self,
        text: &str,
        x: f32,
        y: f32,
        scale: f32,
        tracking: f32,
        align: Align,
        color: [f32; 4],
        glow: f32,
    ) -> Vec<Quad> {
        let width = self.measure(text, scale, tracking);
        let start_x = match align {
            Align::Left => x,
            Align::Center => x - width * 0.5,
            Align::Right => x - width,
        };
        let ink_w = GLYPH_W as f32 * scale;
        let ink_h = GLYPH_H as f32 * scale;
        let advance = (ADVANCE + tracking) * scale;

        let mut out = Vec::with_capacity(text.chars().count());
        for (i, ch) in text.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let Some(uv) = self.uv(ch) else { continue };
            let gx = start_x + i as f32 * advance;
            // The glyph sits in the top-left of its cell; sample only the inked part.
            out.push(Quad::glyph(gx, y, ink_w, ink_h, color, uv).with_glow(glow));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Font {
        Font::build()
    }

    /// The character set the UI is allowed to rely on.
    const REQUIRED: &str = " !\"#%&'()*+,-./0123456789:;<=>?ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_";

    #[test]
    fn every_required_character_is_present() {
        let f = font();
        for ch in REQUIRED.chars() {
            let cell = f.cells[ch as usize];
            assert_ne!(cell, u32::MAX, "missing glyph for '{ch}'");
        }
    }

    #[test]
    fn atlas_size_matches_the_glyph_count() {
        let f = font();
        assert!(f.glyph_count() >= 60, "only {} glyphs", f.glyph_count());
        let rows = f.glyph_count().div_ceil(COLUMNS as usize) as u32;
        assert_eq!(f.width(), COLUMNS * CELL);
        assert_eq!(f.height(), rows * CELL);
        assert_eq!(f.pixels().len(), (f.width() * f.height() * 4) as usize);
    }

    #[test]
    fn glyphs_have_ink_and_the_gutters_stay_empty() {
        let f = font();
        // Cell 0 is the space: nothing inked.
        assert_eq!(f.cells[' ' as usize], 0);
        let cx = 0u32;
        let cy = 0u32;
        for y in 0..CELL {
            for x in 0..CELL {
                let px = f.pixels[((cy + y) * f.width() + (cx + x)) as usize * 4 + 3];
                assert_eq!(px, 0, "space glyph is inked at ({x},{y})");
            }
        }

        // 'A' is somewhere in the atlas; it must have ink, and its gutter must not.
        let cell = f.cells['A' as usize];
        let cx = (cell % COLUMNS) * CELL;
        let cy = (cell / COLUMNS) * CELL;
        let mut inked = 0;
        for y in 0..CELL {
            for x in 0..CELL {
                let a = f.pixels[((cy + y) * f.width() + (cx + x)) as usize * 4 + 3];
                if a != 0 {
                    inked += 1;
                    assert!(
                        x < GLYPH_W && y < GLYPH_H,
                        "'A' inked outside its ink box at ({x},{y})"
                    );
                }
            }
        }
        assert!(inked > 15, "'A' only has {inked} inked pixels");
    }

    #[test]
    fn digits_are_distinguishable() {
        let f = font();
        let ink = |ch: char| {
            let cell = f.cells[ch as usize];
            let cx = (cell % COLUMNS) * CELL;
            let cy = (cell / COLUMNS) * CELL;
            let mut sig = [0u8; GLYPH_H as usize * GLYPH_W as usize];
            for y in 0..GLYPH_H {
                for x in 0..GLYPH_W {
                    let i = ((cy + y) * f.width() + (cx + x)) as usize * 4 + 3;
                    sig[(y * GLYPH_W + x) as usize] = (f.pixels[i] > 0) as u8;
                }
            }
            sig
        };
        for a in '0'..='9' {
            for b in '0'..='9' {
                if a != b {
                    assert_ne!(ink(a), ink(b), "'{a}' and '{b}' look alike");
                }
            }
        }
    }

    #[test]
    fn lowercase_folds_to_uppercase() {
        let f = font();
        let upper = f.uv('A').unwrap();
        let lower = f.uv('a').unwrap();
        assert_eq!(upper, lower);
    }

    #[test]
    fn unknown_characters_fall_back_to_the_placeholder() {
        let f = font();
        assert_eq!(f.uv('™'), f.uv('?'));
        assert_eq!(f.uv('\u{0}'), f.uv('?'));
        assert!(f.uv('?').is_some());
    }

    #[test]
    fn measure_is_linear_in_glyphs_and_scale() {
        let f = font();
        assert_eq!(f.measure("", 2.0, 0.0), 0.0);
        assert_eq!(f.measure("A", 1.0, 0.0), ADVANCE);
        assert_eq!(f.measure("AB", 1.0, 0.0), ADVANCE * 2.0);
        assert_eq!(f.measure("AB", 2.0, 0.0), ADVANCE * 4.0);
        // Tracking adds between glyphs only, never after the last one.
        assert_eq!(f.measure("AB", 1.0, 2.0), ADVANCE * 2.0 + 2.0);
        assert_eq!(f.measure("ABC", 1.0, 1.0), ADVANCE * 3.0 + 2.0);
        assert!(f.measure("SCORE", 2.0, 0.0) > 50.0);
    }

    #[test]
    fn layout_skips_spaces_and_respects_alignment() {
        let f = font();
        let left = f.layout("HI", 100.0, 50.0, 1.0, 0.0, Align::Left, [1.0; 4], 0.0);
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].x, 100.0);
        assert_eq!(left[1].x, 100.0 + ADVANCE);

        let centred = f.layout("HI", 100.0, 50.0, 1.0, 0.0, Align::Center, [1.0; 4], 0.0);
        assert_eq!(centred[0].x, 100.0 - f.measure("HI", 1.0, 0.0) * 0.5);

        let right = f.layout("HI", 100.0, 50.0, 1.0, 0.0, Align::Right, [1.0; 4], 0.0);
        assert_eq!(right[0].x, 100.0 - f.measure("HI", 1.0, 0.0));

        let spaced = f.layout("HI", 0.0, 0.0, 1.0, 4.0, Align::Left, [1.0; 4], 0.0);
        assert_eq!(spaced[1].x, ADVANCE + 4.0);

        assert!(f
            .layout("", 0.0, 0.0, 1.0, 0.0, Align::Left, [1.0; 4], 0.0)
            .is_empty());
        assert_eq!(
            f.layout(" ", 0.0, 0.0, 1.0, 0.0, Align::Left, [1.0; 4], 0.0)
                .len(),
            0
        );
    }

    #[test]
    fn glyph_quads_carry_colour_glow_and_uv() {
        let f = font();
        let q = &f.layout(
            "S",
            0.0,
            0.0,
            2.0,
            0.0,
            Align::Left,
            [0.2, 0.8, 1.0, 0.5],
            0.9,
        )[0];
        assert_eq!(q.color, [0.2, 0.8, 1.0, 0.5]);
        assert_eq!(q.glow, 0.9);
        assert_eq!(q.w, GLYPH_W as f32 * 2.0);
        assert_eq!(q.h, GLYPH_H as f32 * 2.0);
        let uv = q.uv;
        assert!(
            uv[0] < uv[2] && uv[1] < uv[3],
            "uv must be a real rect: {uv:?}"
        );
        assert!(uv[2] <= 1.0 && uv[3] <= 1.0);
    }

    #[test]
    fn line_height_scales() {
        let f = font();
        assert_eq!(f.line_height(1.0), LINE_HEIGHT);
        assert_eq!(f.line_height(3.0), LINE_HEIGHT * 3.0);
    }
}
