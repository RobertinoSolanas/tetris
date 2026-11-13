//! The palette. Kept in one file so the game has one look instead of forty opinions.
//!
//! Colours are linear floats; the renderer's tone map and the bloom pass decide how bright they end
//! up on screen.

/// Foreground text.
pub const TEXT: [f32; 4] = [0.92, 0.96, 1.0, 1.0];
/// Dim labels ("SCORE", "LEVEL").
pub const LABEL: [f32; 4] = [0.45, 0.62, 0.80, 1.0];
/// Secondary text (descriptions, hints).
pub const MUTED: [f32; 4] = [0.42, 0.48, 0.62, 1.0];
/// Panel bodies.
pub const PANEL: [f32; 4] = [0.05, 0.07, 0.13, 0.82];
/// Panel borders.
pub const PANEL_EDGE: [f32; 4] = [0.22, 0.62, 0.95, 0.55];
/// Playfield interior.
pub const WELL: [f32; 4] = [0.02, 0.03, 0.06, 0.92];
/// Playfield frame.
pub const WELL_EDGE: [f32; 4] = [0.30, 0.80, 1.0, 0.85];
/// Grid inside the well.
pub const GRID: [f32; 4] = [0.10, 0.22, 0.35, 0.35];
/// The "danger" tint used when the stack gets high.
pub const DANGER: [f32; 4] = [1.0, 0.25, 0.30, 1.0];
/// Accent used for the score and highlights.
pub const ACCENT: [f32; 4] = [0.35, 0.95, 1.0, 1.0];
/// Warm accent (combos, streaks).
pub const WARM: [f32; 4] = [1.0, 0.72, 0.30, 1.0];
/// Ghost piece outline.
pub const GHOST: [f32; 4] = [0.55, 0.85, 1.0, 0.42];
/// Overlay backdrop.
pub const SCRIM: [f32; 4] = [0.01, 0.015, 0.04, 0.78];
/// Background gradient, top and bottom.
pub const SKY_TOP: [f32; 4] = [0.045, 0.06, 0.135, 1.0];
pub const SKY_BOTTOM: [f32; 4] = [0.01, 0.012, 0.035, 1.0];
/// Drifting background grid.
pub const BG_GRID: [f32; 4] = [0.10, 0.35, 0.55, 0.16];

/// Tint a colour towards white by `amount` (for flashes).
pub fn tint_towards_white(color: [f32; 4], amount: f32) -> [f32; 4] {
    let a = amount.clamp(0.0, 1.0);
    [
        color[0] + (1.0 - color[0]) * a,
        color[1] + (1.0 - color[1]) * a,
        color[2] + (1.0 - color[2]) * a,
        color[3],
    ]
}

/// Scale alpha.
pub fn with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    let mut c = color;
    c[3] = alpha.clamp(0.0, 1.0);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_entries_are_in_range() {
        for c in [
            TEXT, LABEL, MUTED, PANEL, PANEL_EDGE, WELL, WELL_EDGE, GRID, DANGER, ACCENT, WARM,
            GHOST, SCRIM, SKY_TOP, SKY_BOTTOM, BG_GRID,
        ] {
            for v in c {
                assert!((0.0..=1.0).contains(&v), "component {v} out of range");
            }
        }
    }

    #[test]
    fn text_is_brighter_than_its_background() {
        let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        assert!(
            luma(TEXT) > luma(SKY_TOP) + 0.5,
            "contrast is the whole point"
        );
        assert!(luma(LABEL) > luma(SKY_TOP));
        assert!(luma(WELL) < luma(WELL_EDGE));
    }

    #[test]
    fn tint_and_alpha_helpers() {
        let white = tint_towards_white([0.0, 0.0, 0.0, 1.0], 1.0);
        assert_eq!(white, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(
            tint_towards_white([0.5, 0.5, 0.5, 1.0], 0.0),
            [0.5, 0.5, 0.5, 1.0]
        );
        // Out-of-range amounts are clamped, not wrapped.
        assert_eq!(
            tint_towards_white([0.0, 0.0, 0.0, 1.0], 9.0),
            [1.0, 1.0, 1.0, 1.0]
        );
        assert_eq!(with_alpha([1.0, 1.0, 1.0, 1.0], -1.0)[3], 0.0);
        assert_eq!(with_alpha([1.0, 1.0, 1.0, 1.0], 5.0)[3], 1.0);
    }
}
