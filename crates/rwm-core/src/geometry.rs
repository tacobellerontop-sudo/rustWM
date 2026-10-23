use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle in physical pixels.
///
/// `x`/`y` is the top-left corner. Width and height are signed so that
/// arithmetic on gaps and borders cannot underflow; a non-positive size is
/// treated as empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Shrinks the rectangle by `amount` on every side.
    pub fn inset(&self, amount: i32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.w - 2 * amount).max(0),
            (self.h - 2 * amount).max(0),
        )
    }

    /// Splits horizontally (side by side) at `ratio` of the width.
    pub fn split_h(&self, ratio: f32) -> (Self, Self) {
        let left = (self.w as f32 * ratio).round() as i32;
        (
            Self::new(self.x, self.y, left, self.h),
            Self::new(self.x + left, self.y, self.w - left, self.h),
        )
    }

    /// Splits vertically (stacked) at `ratio` of the height.
    pub fn split_v(&self, ratio: f32) -> (Self, Self) {
        let top = (self.h as f32 * ratio).round() as i32;
        (
            Self::new(self.x, self.y, self.w, top),
            Self::new(self.x, self.y + top, self.w, self.h - top),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_h_covers_whole_rect() {
        let r = Rect::new(0, 0, 1001, 500);
        let (a, b) = r.split_h(0.5);
        assert_eq!(a.w + b.w, r.w);
        assert_eq!(b.x, a.right());
    }

    #[test]
    fn split_v_covers_whole_rect() {
        let r = Rect::new(10, 20, 300, 777);
        let (a, b) = r.split_v(0.3);
        assert_eq!(a.h + b.h, r.h);
        assert_eq!(b.y, a.bottom());
    }

    #[test]
    fn inset_never_goes_negative() {
        assert_eq!(Rect::new(0, 0, 10, 10).inset(8), Rect::new(8, 8, 0, 0));
    }

    #[test]
    fn contains_point_is_half_open() {
        let r = Rect::new(0, 0, 10, 10);
        assert!(r.contains_point(0, 0));
        assert!(!r.contains_point(10, 5));
    }
}
