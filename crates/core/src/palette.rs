//! Per-tetromino colours — the one place the domain is allowed to talk colour.
//!
//! A colour is an intrinsic property of a tetromino (players identify pieces by it), so the mapping
//! lives in the core; *how* a colour is lit, bloomed or tone-mapped is the renderer's business and it
//! never sees these types beyond an RGBA triple handed to it through a [`crate`](crate) quad.
//!
//! Values are linear-ish sRGB floats in 0.0..=1.0, tuned for a dark background with bloom.

use crate::piece::Tetromino;

/// `[r, g, b]` per [`Tetromino::ALL`] index, 0.0..=1.0.
pub const COLORS: [[f32; 3]; 7] = [
    [0.20, 0.95, 1.00], // I — cyan
    [0.35, 0.45, 1.00], // J — blue
    [1.00, 0.52, 0.16], // L — orange
    [1.00, 0.86, 0.20], // O — yellow
    [0.35, 1.00, 0.42], // S — green
    [0.78, 0.42, 1.00], // T — purple
    [1.00, 0.33, 0.42], // Z — red
];

/// Darker companion used for the interior of a block, `[r, g, b]` per piece.
pub const COLORS_DIM: [[f32; 3]; 7] = [
    [0.05, 0.34, 0.40],
    [0.09, 0.13, 0.42],
    [0.42, 0.19, 0.05],
    [0.44, 0.36, 0.06],
    [0.10, 0.40, 0.15],
    [0.29, 0.14, 0.42],
    [0.42, 0.11, 0.15],
];

impl Tetromino {
    /// Bright edge / glow colour of this piece.
    pub const fn color(self) -> [f32; 3] {
        COLORS[self.color_index()]
    }

    /// Body colour of this piece.
    pub const fn color_dim(self) -> [f32; 3] {
        COLORS_DIM[self.color_index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_covers_every_piece_and_is_in_range() {
        assert_eq!(COLORS.len(), Tetromino::ALL.len());
        assert_eq!(COLORS_DIM.len(), Tetromino::ALL.len());
        for c in COLORS.iter().chain(COLORS_DIM.iter()) {
            for v in c {
                assert!((0.0..=1.0).contains(v), "component out of range: {v}");
            }
        }
    }

    #[test]
    fn every_piece_is_visually_distinct() {
        for (i, a) in COLORS.iter().enumerate() {
            for (j, b) in COLORS.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "pieces {i} and {j} share a colour");
                }
            }
        }
    }

    #[test]
    fn every_piece_has_a_non_black_body() {
        for kind in Tetromino::ALL {
            let [r, g, b] = kind.color_dim();
            assert!(r + g + b > 0.2, "{kind} body colour is too dark to see");
        }
    }
}
