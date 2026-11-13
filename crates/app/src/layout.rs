//! Layout maths for the HUD and the well — pure, so it can be tested without a window.

use crate::rect::Rect;

/// Field geometry in logical (physical-pixel) units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    /// UI scale: 1.0 on a 1× display, 2.0 on a HiDPI panel.
    pub scale: f32,
    /// Size of one playfield cell in pixels.
    pub cell: f32,
    /// Top-left of the playfield.
    pub field_x: f32,
    pub field_y: f32,
    pub field_w: f32,
    pub field_h: f32,
    /// Side panels.
    pub left: Rect,
    pub right: Rect,
}

impl Layout {
    /// Centre the 10×22 well, then split what is left between the two side panels.
    pub fn new(width: f32, height: f32, scale: f32) -> Self {
        let width = width.max(64.0);
        let height = height.max(64.0);
        let scale = scale.max(0.5);

        // Leave a margin top and bottom so the well never touches the window edge.
        let vertical_margin = 24.0 * scale;
        let available_h = (height - vertical_margin * 2.0).max(40.0);
        let available_w = (width * 0.46).max(40.0);

        let cell = (available_h / crate::FIELD_ROWS as f32)
            .min(available_w / crate::FIELD_COLS as f32)
            .max(6.0);

        let field_w = cell * crate::FIELD_COLS as f32;
        let field_h = cell * crate::FIELD_ROWS as f32;
        let field_x = (width - field_w) * 0.5 + (width * 0.06);
        let field_y = (height - field_h) * 0.5;

        let panel_gap = 18.0 * scale;
        let left_width = (field_x - panel_gap * 2.0).max(60.0);
        let left = Rect::new(panel_gap, field_y, left_width, field_h);
        let right_x = field_x + field_w + panel_gap;
        let right_width = (width - right_x - panel_gap).max(60.0);
        let right = Rect::new(right_x, field_y, right_width, field_h);

        Self {
            width,
            height,
            scale,
            cell,
            field_x,
            field_y,
            field_w,
            field_h,
            left,
            right,
        }
    }

    /// Pixel rect of a board cell. Rows above the field (negative after the buffer clamp) still map
    /// sensibly, which is how a piece spawning in the buffer stays visible.
    pub fn cell_rect(&self, col: i32, row: i32) -> Rect {
        Rect::new(
            self.field_x + col as f32 * self.cell,
            self.field_y + row as f32 * self.cell,
            self.cell,
            self.cell,
        )
    }

    /// Font size for body text.
    pub fn body(&self) -> f32 {
        (2.0 * self.scale).max(1.0)
    }

    /// Font size for headings.
    pub fn heading(&self) -> f32 {
        3.0 * self.scale
    }

    /// Font size for the big score readout.
    pub fn score_size(&self) -> f32 {
        (4.0 * self.scale).max(2.0)
    }

    /// Font size for the title screen.
    pub fn title(&self) -> f32 {
        (10.0 * self.scale).max(4.0)
    }

    /// Does the window have enough room for the side panels' detail?
    pub fn compact(&self) -> bool {
        self.width < 900.0 || self.height < 560.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_is_centred_and_fits() {
        let l = Layout::new(1280.0, 800.0, 1.0);
        assert!(l.cell >= 10.0);
        assert!(l.field_w <= 1280.0);
        assert!(l.field_h <= 800.0);
        assert!(l.field_y >= 0.0);
        assert!(l.field_y + l.field_h <= 800.0 + 0.001);
        // The well sits right of centre to leave room for the taller left panel.
        assert!(l.field_x > (1280.0 - l.field_w) * 0.5);
    }

    #[test]
    fn cells_map_to_pixels_consistently() {
        let l = Layout::new(1280.0, 800.0, 1.0);
        let top = l.cell_rect(0, 0);
        assert_eq!((top.x, top.y), (l.field_x, l.field_y));
        assert_eq!(top.w, l.cell);
        let next = l.cell_rect(1, 1);
        assert_eq!(next.x, l.field_x + l.cell);
        assert_eq!(next.y, l.field_y + l.cell);
        // The bottom-right cell ends exactly at the field edge.
        let last = l.cell_rect(9, 21);
        assert!((last.right() - (l.field_x + l.field_w)).abs() < 0.01);
        assert!((last.bottom() - (l.field_y + l.field_h)).abs() < 0.01);
    }

    #[test]
    fn absurd_sizes_still_produce_a_usable_layout() {
        for (w, h) in [(1.0, 1.0), (0.0, 0.0), (640.0, 200.0), (4000.0, 3000.0)] {
            let l = Layout::new(w, h, 1.0);
            assert!(l.cell >= 6.0, "{w}×{h} → cell {}", l.cell);
            assert!(l.field_w.is_finite() && l.field_h.is_finite());
            assert!(l.left.w >= 1.0);
            assert!(l.right.w >= 1.0);
        }
    }

    #[test]
    fn scale_scales_the_text() {
        let one = Layout::new(1280.0, 800.0, 1.0);
        let two = Layout::new(1280.0, 800.0, 2.0);
        assert!(two.title() > one.title());
        assert!(two.score_size() > one.score_size());
        assert!(two.body() > one.body());
        // But the well is bounded by the window, not by the scale.
        assert!(two.cell <= one.cell + 0.001);
    }

    #[test]
    fn compact_mode_kicks_in_on_small_windows() {
        assert!(Layout::new(700.0, 500.0, 1.0).compact());
        assert!(!Layout::new(1600.0, 900.0, 1.0).compact());
    }
}
