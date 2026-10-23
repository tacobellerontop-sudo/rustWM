//! Layout algorithms: pure functions from a window count and an area to rects.
//!
//! Layouts never see window handles or the OS; they only answer "where do the
//! n windows of this workspace go", so they are tested with golden rects.

use serde::{Deserialize, Serialize};

use crate::Rect;

/// Gaps in logical pixels (at 100% scale); scaled per monitor at arrange time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gaps {
    /// Between neighbouring windows.
    pub inner: i32,
    /// Between windows and the edge of the work area.
    pub outer: i32,
    /// No gaps at all when a workspace has a single window.
    pub smart: bool,
}

impl Default for Gaps {
    fn default() -> Self {
        Self {
            inner: 8,
            outer: 8,
            smart: true,
        }
    }
}

impl Gaps {
    pub fn scaled(self, scale: f32) -> Self {
        let s = |v: i32| (v as f32 * scale).round() as i32;
        Self {
            inner: s(self.inner),
            outer: s(self.outer),
            smart: self.smart,
        }
    }
}

pub const DEFAULT_RATIO: f32 = 0.5;
pub const MIN_RATIO: f32 = 0.1;
pub const MAX_RATIO: f32 = 0.9;

/// Per-split ratio overrides from resizing. Split `i` is the one that gives
/// window `i` its share of the remaining space. Missing entries mean 0.5.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ratios(pub Vec<f32>);

impl Ratios {
    pub fn get(&self, split: usize) -> f32 {
        self.0.get(split).copied().unwrap_or(DEFAULT_RATIO)
    }

    pub fn set(&mut self, split: usize, ratio: f32) {
        if self.0.len() <= split {
            self.0.resize(split + 1, DEFAULT_RATIO);
        }
        self.0[split] = ratio.clamp(MIN_RATIO, MAX_RATIO);
    }

    pub fn reset(&mut self) {
        self.0.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutKind {
    /// Hyprland's default: each window takes half of the space left by the
    /// previous one, splitting along the longer side.
    #[default]
    Dwindle,
}

impl LayoutKind {
    pub fn arrange(self, count: usize, area: Rect, gaps: Gaps, ratios: &Ratios) -> Vec<Rect> {
        match self {
            LayoutKind::Dwindle => dwindle(count, area, gaps, ratios),
        }
    }
}

/// Splits `r` at `ratio` along its longer side and leaves `gap` between the
/// two halves. Returns (first, rest).
fn split_with_gap(r: Rect, ratio: f32, gap: i32) -> (Rect, Rect) {
    if r.w >= r.h {
        let usable = (r.w - gap).max(0);
        let first = (usable as f32 * ratio).round() as i32;
        (
            Rect::new(r.x, r.y, first, r.h),
            Rect::new(r.x + first + gap, r.y, usable - first, r.h),
        )
    } else {
        let usable = (r.h - gap).max(0);
        let first = (usable as f32 * ratio).round() as i32;
        (
            Rect::new(r.x, r.y, r.w, first),
            Rect::new(r.x, r.y + first + gap, r.w, usable - first),
        )
    }
}

fn effective_gaps(count: usize, gaps: Gaps) -> Gaps {
    if gaps.smart && count == 1 {
        Gaps {
            inner: 0,
            outer: 0,
            smart: true,
        }
    } else {
        gaps
    }
}

pub fn dwindle(count: usize, area: Rect, gaps: Gaps, ratios: &Ratios) -> Vec<Rect> {
    let gaps = effective_gaps(count, gaps);
    let mut rest = area.inset(gaps.outer);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        if i == count - 1 {
            out.push(rest);
        } else {
            let (first, remaining) = split_with_gap(rest, ratios.get(i), gaps.inner);
            out.push(first);
            rest = remaining;
        }
    }
    out
}

/// Equal-width columns across `area`, left to right. The last column absorbs
/// any rounding remainder so the columns cover the area exactly.
pub fn columns(count: usize, area: Rect) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let base = area.w / n;
    (0..n)
        .map(|i| {
            let w = if i == n - 1 {
                area.w - base * (n - 1)
            } else {
                base
            };
            Rect::new(area.x + base * i, area.y, w, area.h)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1920, 1040);
    const NO_GAPS: Gaps = Gaps {
        inner: 0,
        outer: 0,
        smart: false,
    };

    #[test]
    fn dwindle_empty_and_single() {
        assert!(dwindle(0, AREA, NO_GAPS, &Ratios::default()).is_empty());
        assert_eq!(dwindle(1, AREA, NO_GAPS, &Ratios::default()), vec![AREA]);
    }

    #[test]
    fn dwindle_spirals_along_longer_side() {
        let rects = dwindle(4, AREA, NO_GAPS, &Ratios::default());
        assert_eq!(
            rects,
            vec![
                Rect::new(0, 0, 960, 1040),    // left half (wide area: split x)
                Rect::new(960, 0, 960, 520),   // right half is tall: split y
                Rect::new(960, 520, 480, 520), // bottom-right quarter is wide: split x
                Rect::new(1440, 520, 480, 520),
            ]
        );
    }

    #[test]
    fn dwindle_with_gaps() {
        let gaps = Gaps {
            inner: 10,
            outer: 20,
            smart: true,
        };
        let rects = dwindle(2, Rect::new(0, 0, 1000, 500), gaps, &Ratios::default());
        assert_eq!(
            rects,
            vec![Rect::new(20, 20, 475, 460), Rect::new(505, 20, 475, 460)]
        );
    }

    #[test]
    fn smart_gaps_drop_gaps_for_one_window() {
        let gaps = Gaps {
            inner: 10,
            outer: 20,
            smart: true,
        };
        assert_eq!(dwindle(1, AREA, gaps, &Ratios::default()), vec![AREA]);
        let dumb = Gaps {
            smart: false,
            ..gaps
        };
        assert_eq!(
            dwindle(1, AREA, dumb, &Ratios::default()),
            vec![AREA.inset(20)]
        );
    }

    #[test]
    fn dwindle_honours_ratios() {
        let mut ratios = Ratios::default();
        ratios.set(0, 0.7);
        let rects = dwindle(2, Rect::new(0, 0, 1000, 500), NO_GAPS, &ratios);
        assert_eq!(
            rects,
            vec![Rect::new(0, 0, 700, 500), Rect::new(700, 0, 300, 500)]
        );
    }

    #[test]
    fn ratios_are_clamped() {
        let mut ratios = Ratios::default();
        ratios.set(2, 5.0);
        assert_eq!(ratios.get(2), MAX_RATIO);
        assert_eq!(ratios.get(0), DEFAULT_RATIO);
    }

    #[test]
    fn dwindle_tiles_never_overlap() {
        for n in 1..12 {
            let rects = dwindle(n, AREA, Gaps::default(), &Ratios::default());
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    let overlap =
                        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
                    assert!(!overlap, "n={n}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn scaled_gaps() {
        let g = Gaps::default().scaled(1.5);
        assert_eq!((g.inner, g.outer), (12, 12));
    }

    #[test]
    fn three_columns_cover_area() {
        let rects = columns(3, Rect::new(0, 0, 1000, 500));
        assert_eq!(
            rects,
            vec![
                Rect::new(0, 0, 333, 500),
                Rect::new(333, 0, 333, 500),
                Rect::new(666, 0, 334, 500),
            ]
        );
    }
}
