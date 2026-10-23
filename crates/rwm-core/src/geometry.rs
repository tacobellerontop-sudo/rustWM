use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl FromStr for Direction {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        })
    }
}

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

    /// Distance from this rect to `other` along `dir`, if `other` lies in
    /// that direction (its centre is past ours). Used for geometric focus.
    fn distance_towards(&self, other: &Rect, dir: Direction) -> Option<(bool, i32, i32)> {
        let (cx, cy) = self.center();
        let (ox, oy) = other.center();
        // (overlaps on the perpendicular axis, gap along dir, centre offset across)
        let overlap_y = other.y < self.bottom() && self.y < other.bottom();
        let overlap_x = other.x < self.right() && self.x < other.right();
        match dir {
            Direction::Left if ox < cx => {
                Some((overlap_y, (self.x - other.right()).max(0), (oy - cy).abs()))
            }
            Direction::Right if ox > cx => {
                Some((overlap_y, (other.x - self.right()).max(0), (oy - cy).abs()))
            }
            Direction::Up if oy < cy => {
                Some((overlap_x, (self.y - other.bottom()).max(0), (ox - cx).abs()))
            }
            Direction::Down if oy > cy => {
                Some((overlap_x, (other.y - self.bottom()).max(0), (ox - cx).abs()))
            }
            _ => None,
        }
    }

    /// Picks the candidate nearest to this rect in `dir`: windows that overlap
    /// on the other axis win, then the smallest gap, then the best alignment.
    pub fn nearest<T: Copy>(
        &self,
        dir: Direction,
        candidates: impl IntoIterator<Item = (T, Rect)>,
    ) -> Option<T> {
        candidates
            .into_iter()
            .filter_map(|(id, r)| {
                self.distance_towards(&r, dir)
                    .map(|(overlap, gap, off)| ((!overlap, gap, off), id))
            })
            .min_by_key(|(key, _)| *key)
            .map(|(_, id)| id)
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
    fn nearest_prefers_overlapping_neighbour() {
        // Focused window on the left half; right half split top/bottom.
        let focused = Rect::new(0, 0, 960, 1080);
        let top = Rect::new(960, 0, 960, 540);
        let bottom = Rect::new(960, 540, 960, 540);
        let far = Rect::new(3000, 0, 100, 100);
        let pick = focused.nearest(Direction::Right, [(1, top), (2, bottom), (3, far)]);
        assert_eq!(pick, Some(1));
        assert_eq!(
            bottom.nearest(Direction::Up, [(1, top), (0, focused)]),
            Some(1)
        );
        assert_eq!(top.nearest(Direction::Left, [(0, focused)]), Some(0));
        assert_eq!(
            focused.nearest(Direction::Left, [(1, top), (2, bottom)]),
            None
        );
    }

    #[test]
    fn contains_point_is_half_open() {
        let r = Rect::new(0, 0, 10, 10);
        assert!(r.contains_point(0, 0));
        assert!(!r.contains_point(10, 5));
    }
}
