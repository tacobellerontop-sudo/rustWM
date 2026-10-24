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
    /// komorebi/dwm style: one master window on the left at the master ratio,
    /// the rest stacked on the right.
    Master,
    /// Every window fills the work area; the focused one is on top.
    Monocle,
}

impl LayoutKind {
    pub const ALL: [LayoutKind; 3] = [LayoutKind::Dwindle, LayoutKind::Master, LayoutKind::Monocle];

    pub fn arrange(self, count: usize, area: Rect, gaps: Gaps, ratios: &Ratios) -> Vec<Rect> {
        match self {
            LayoutKind::Dwindle => dwindle(count, area, gaps, ratios),
            LayoutKind::Master => master(count, area, gaps, ratios),
            LayoutKind::Monocle => monocle(count, area, gaps),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::Dwindle => "dwindle",
            LayoutKind::Master => "master",
            LayoutKind::Monocle => "monocle",
        }
    }

    /// The next layout in `cycle-layout` order.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Adjusts `ratios` so the window at `index` of `count` grows by `delta`
    /// (shrinks if negative). Returns whether anything changed.
    pub fn resize(self, ratios: &mut Ratios, index: usize, count: usize, delta: f32) -> bool {
        if count < 2 {
            return false;
        }
        let (split, sign) = match self {
            LayoutKind::Monocle => return false,
            // Window i owns the first part of split i; the last window owns
            // the second part of the final split.
            LayoutKind::Dwindle if index < count - 1 => (index, 1.0),
            LayoutKind::Dwindle => (count - 2, -1.0),
            LayoutKind::Master if index == 0 => (0, 1.0),
            LayoutKind::Master => (0, -1.0),
        };
        ratios.set(split, ratios.get(split) + sign * delta);
        true
    }
}

impl LayoutKind {
    /// The splits whose ratios move window `index`'s edges, in the order to
    /// fit them (outer split first).
    fn splits_for(self, index: usize, count: usize) -> Vec<usize> {
        match self {
            LayoutKind::Monocle => Vec::new(),
            LayoutKind::Master => vec![0],
            LayoutKind::Dwindle => {
                let mut s = Vec::new();
                if index > 0 {
                    s.push(index - 1);
                }
                if index + 1 < count {
                    s.push(index);
                }
                s
            }
        }
    }

    /// Adjusts `ratios` so window `index` comes as close as possible to
    /// `target`: used when the user resizes a tiled window with the mouse.
    /// Returns whether any ratio changed.
    pub fn fit(
        self,
        ratios: &mut Ratios,
        index: usize,
        count: usize,
        area: Rect,
        gaps: Gaps,
        target: Rect,
    ) -> bool {
        let before = ratios.clone();
        let error = |ratios: &Ratios| {
            let r = self.arrange(count, area, gaps, ratios)[index];
            (r.x - target.x).abs()
                + (r.y - target.y).abs()
                + (r.right() - target.right()).abs()
                + (r.bottom() - target.bottom()).abs()
        };
        // Two passes, because the outer split shifts the space the inner one divides.
        for _ in 0..2 {
            for split in self.splits_for(index, count) {
                let mut best = (error(ratios), ratios.get(split));
                let steps = ((MAX_RATIO - MIN_RATIO) / 0.0025) as usize;
                for step in 0..=steps {
                    let r = MIN_RATIO + step as f32 * 0.0025;
                    ratios.set(split, r);
                    let e = error(ratios);
                    if e < best.0 {
                        best = (e, r);
                    }
                }
                ratios.set(split, best.1);
            }
        }
        *ratios != before
    }
}

impl std::str::FromStr for LayoutKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|l| l.name() == s).ok_or(())
    }
}

impl std::fmt::Display for LayoutKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
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

/// Splits `r` into `count` equal rows with `gap` between them; the last row
/// absorbs the rounding remainder.
fn rows(count: usize, r: Rect, gap: i32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let usable = (r.h - gap * (n - 1)).max(0);
    let base = usable / n;
    (0..n)
        .map(|i| {
            let h = if i == n - 1 {
                usable - base * (n - 1)
            } else {
                base
            };
            Rect::new(r.x, r.y + (base + gap) * i, r.w, h)
        })
        .collect()
}

pub fn master(count: usize, area: Rect, gaps: Gaps, ratios: &Ratios) -> Vec<Rect> {
    let gaps = effective_gaps(count, gaps);
    let area = area.inset(gaps.outer);
    match count {
        0 => Vec::new(),
        1 => vec![area],
        _ => {
            let usable = (area.w - gaps.inner).max(0);
            let mw = (usable as f32 * ratios.get(0)).round() as i32;
            let main = Rect::new(area.x, area.y, mw, area.h);
            let stack = Rect::new(area.x + mw + gaps.inner, area.y, usable - mw, area.h);
            let mut out = vec![main];
            out.extend(rows(count - 1, stack, gaps.inner));
            out
        }
    }
}

pub fn monocle(count: usize, area: Rect, gaps: Gaps) -> Vec<Rect> {
    // Monocle shows one window at a time, so smart gaps treat it as alone.
    let gaps = effective_gaps(1, gaps);
    vec![area.inset(gaps.outer); count]
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
    fn master_stack() {
        let rects = master(4, Rect::new(0, 0, 1000, 900), NO_GAPS, &Ratios::default());
        assert_eq!(
            rects,
            vec![
                Rect::new(0, 0, 500, 900),
                Rect::new(500, 0, 500, 300),
                Rect::new(500, 300, 500, 300),
                Rect::new(500, 600, 500, 300),
            ]
        );
        assert_eq!(master(1, AREA, NO_GAPS, &Ratios::default()), vec![AREA]);
    }

    #[test]
    fn master_with_gaps_and_ratio() {
        let gaps = Gaps {
            inner: 10,
            outer: 10,
            smart: true,
        };
        let mut ratios = Ratios::default();
        ratios.set(0, 0.6);
        let rects = master(3, Rect::new(0, 0, 1010, 520), gaps, &ratios);
        assert_eq!(
            rects,
            vec![
                Rect::new(10, 10, 588, 500),
                Rect::new(608, 10, 392, 245),
                Rect::new(608, 265, 392, 245),
            ]
        );
    }

    #[test]
    fn monocle_stacks_everything_full_size() {
        let gaps = Gaps::default();
        assert_eq!(monocle(3, AREA, gaps), vec![AREA; 3]);
        let dumb = Gaps {
            smart: false,
            ..gaps
        };
        assert_eq!(monocle(2, AREA, dumb), vec![AREA.inset(8); 2]);
    }

    #[test]
    fn layout_names_and_cycle() {
        for l in LayoutKind::ALL {
            assert_eq!(l.name().parse::<LayoutKind>(), Ok(l));
        }
        assert_eq!(LayoutKind::Dwindle.next(), LayoutKind::Master);
        assert_eq!(LayoutKind::Monocle.next(), LayoutKind::Dwindle);
        assert!("spiral".parse::<LayoutKind>().is_err());
    }

    #[test]
    fn resize_targets_the_right_split() {
        let mut r = Ratios::default();
        assert!(LayoutKind::Dwindle.resize(&mut r, 1, 3, 0.1));
        assert_eq!(r.get(1), 0.6);
        // The last window grows by shrinking the final split.
        LayoutKind::Dwindle.resize(&mut r, 2, 3, 0.1);
        assert_eq!(r.get(1), 0.5);

        let mut r = Ratios::default();
        LayoutKind::Master.resize(&mut r, 2, 3, 0.1);
        assert_eq!(r.get(0), 0.4);
        assert!(!LayoutKind::Monocle.resize(&mut r, 0, 3, 0.1));
        assert!(!LayoutKind::Dwindle.resize(&mut r, 0, 1, 0.1));
    }

    #[test]
    fn fit_follows_a_dragged_edge() {
        // Two windows side by side; the user drags the left one's right edge
        // from x=960 to x=1200.
        let mut r = Ratios::default();
        let target = Rect::new(0, 0, 1200, 1040);
        assert!(LayoutKind::Dwindle.fit(&mut r, 0, 2, AREA, NO_GAPS, target));
        let rects = dwindle(2, AREA, NO_GAPS, &r);
        assert!((rects[0].w - 1200).abs() <= 5, "{rects:?}");

        // Dragging the right window's left edge works too.
        let mut r = Ratios::default();
        LayoutKind::Dwindle.fit(&mut r, 1, 2, AREA, NO_GAPS, Rect::new(700, 0, 1220, 1040));
        assert!((dwindle(2, AREA, NO_GAPS, &r)[1].x - 700).abs() <= 5);
    }

    #[test]
    fn fit_moves_both_edges_of_a_middle_window() {
        // Window 1 of 3 is the top-right one; drag its left and bottom edges.
        // (Kept narrower than tall so dwindle still splits it top/bottom.)
        let mut r = Ratios::default();
        let target = Rect::new(900, 0, 1020, 700);
        LayoutKind::Dwindle.fit(&mut r, 1, 3, AREA, NO_GAPS, target);
        let got = dwindle(3, AREA, NO_GAPS, &r)[1];
        assert!(
            (got.x - 900).abs() <= 5 && (got.bottom() - 700).abs() <= 5,
            "{got:?}"
        );
    }

    #[test]
    fn fit_master_ratio_and_monocle_noop() {
        let mut r = Ratios::default();
        LayoutKind::Master.fit(&mut r, 2, 3, AREA, NO_GAPS, Rect::new(1300, 520, 620, 520));
        assert!((master(3, AREA, NO_GAPS, &r)[2].x - 1300).abs() <= 5);
        assert!(!LayoutKind::Monocle.fit(&mut r, 0, 2, AREA, NO_GAPS, Rect::new(0, 0, 10, 10)));
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
