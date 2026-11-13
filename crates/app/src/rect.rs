//! A pixel rectangle plus the little placement helpers the scene builder keeps needing.

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(self) -> f32 {
        self.x + self.w
    }

    pub const fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub const fn centre_x(self) -> f32 {
        self.x + self.w * 0.5
    }

    pub const fn centre_y(self) -> f32 {
        self.y + self.h * 0.5
    }

    /// Shrink inwards by `m` on every side.
    pub const fn inset(self, m: f32) -> Self {
        Self {
            x: self.x + m,
            y: self.y + m,
            w: (self.w - m * 2.0).max(0.0),
            h: (self.h - m * 2.0).max(0.0),
        }
    }

    /// A sub-rect of `rows` rows starting at `row`, used to split a panel into slots.
    pub const fn row(self, row: f32, rows: f32, height: f32) -> Self {
        Self {
            x: self.x,
            y: self.y + self.h * (row / rows),
            w: self.w,
            h: (self.h / rows).min(height.max(1.0)),
        }
    }

    /// Stack a block of `h` pixels from the top, returning it and the remainder.
    pub fn take_top(self, h: f32) -> (Self, Self) {
        let h = h.clamp(0.0, self.h);
        (
            Self {
                x: self.x,
                y: self.y,
                w: self.w,
                h,
            },
            Self {
                x: self.x,
                y: self.y + h,
                w: self.w,
                h: self.h - h,
            },
        )
    }

    /// Centre a box of `w`×`h` inside self.
    pub const fn centred(self, w: f32, h: f32) -> Self {
        Self {
            x: self.x + (self.w - w) * 0.5,
            y: self.y + (self.h - h) * 0.5,
            w,
            h,
        }
    }

    /// Is the rect big enough to be worth drawing at all?
    pub const fn visible(self) -> bool {
        self.w > 0.5 && self.h > 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_centre() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.right(), 110.0);
        assert_eq!(r.bottom(), 70.0);
        assert_eq!(r.centre_x(), 60.0);
        assert_eq!(r.centre_y(), 45.0);
    }

    #[test]
    fn inset_never_flips() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).inset(30.0);
        assert_eq!((r.w, r.h), (0.0, 0.0));
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).inset(2.0);
        assert_eq!((r.x, r.y, r.w, r.h), (2.0, 2.0, 6.0, 6.0));
    }

    #[test]
    fn take_top_partitions_without_losing_area() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let (top, rest) = r.take_top(30.0);
        assert_eq!(top.h, 30.0);
        assert_eq!(rest.y, 30.0);
        assert_eq!(rest.h, 70.0);
        // Clamped when asked for more than there is.
        let (top, rest) = r.take_top(1_000.0);
        assert_eq!(top.h, 100.0);
        assert_eq!(rest.h, 0.0);
    }

    #[test]
    fn centred_boxes_are_symmetric() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0).centred(20.0, 10.0);
        assert_eq!((r.x, r.y), (40.0, 45.0));
    }

    #[test]
    fn visible_rejects_sub_pixel_slivers() {
        assert!(Rect::new(0.0, 0.0, 2.0, 2.0).visible());
        assert!(!Rect::new(0.0, 0.0, 0.4, 10.0).visible());
    }

    #[test]
    fn row_splits_a_panel_evenly() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let second = r.row(1.0, 4.0, 20.0);
        assert_eq!(second.y, 25.0);
        assert_eq!(second.h, 20.0);
    }
}
