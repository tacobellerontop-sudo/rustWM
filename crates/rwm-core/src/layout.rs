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

/// Per-split ratio overrides from resizing, plus the master layout's
/// settings. Split `i` is the one that gives window `i` its share of the
/// remaining space. Missing entries mean 0.5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ratios {
    pub splits: Vec<f32>,
    /// How many windows share the master area (Hyprland's `nmaster`).
    pub master_count: usize,
    /// Which side of the screen the master area is on.
    pub master_side: Side,
}

impl Default for Ratios {
    fn default() -> Self {
        Self {
            splits: Vec::new(),
            master_count: 1,
            master_side: Side::Left,
        }
    }
}

impl Ratios {
    pub fn get(&self, split: usize) -> f32 {
        self.splits.get(split).copied().unwrap_or(DEFAULT_RATIO)
    }

    pub fn set(&mut self, split: usize, ratio: f32) {
        if self.splits.len() <= split {
            self.splits.resize(split + 1, DEFAULT_RATIO);
        }
        self.splits[split] = ratio.clamp(MIN_RATIO, MAX_RATIO);
    }

    /// Forgets the resized ratios; the master settings stay.
    pub fn reset(&mut self) {
        self.splits.clear();
    }
}

/// A side of the work area, for the master layout's orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Top => "top",
            Side::Bottom => "bottom",
        }
    }
}

impl std::str::FromStr for Side {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|x| x.name() == s).ok_or(())
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
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
    /// Equal-width columns, left to right (komorebi's `columns`).
    Columns,
    /// Equal-height rows, top to bottom (komorebi's `rows`).
    Rows,
    /// As close to a square grid as the window count allows.
    Grid,
    /// For ultrawide screens: the first window in the middle at the master
    /// ratio, the second on the left, the rest stacked on the right
    /// (komorebi's `ultrawide-vertical-stack`, Hyprland's centred master).
    Center,
}

impl LayoutKind {
    pub const ALL: [LayoutKind; 7] = [
        LayoutKind::Dwindle,
        LayoutKind::Master,
        LayoutKind::Monocle,
        LayoutKind::Columns,
        LayoutKind::Rows,
        LayoutKind::Grid,
        LayoutKind::Center,
    ];

    /// The layouts `cycle-layout` goes through unless the config says otherwise.
    pub const DEFAULT_CYCLE: [LayoutKind; 3] =
        [LayoutKind::Dwindle, LayoutKind::Master, LayoutKind::Monocle];

    pub fn arrange(self, count: usize, area: Rect, gaps: Gaps, ratios: &Ratios) -> Vec<Rect> {
        match self {
            LayoutKind::Dwindle => dwindle(count, area, gaps, ratios),
            LayoutKind::Master => master(count, area, gaps, ratios),
            LayoutKind::Monocle => monocle(count, area, gaps),
            LayoutKind::Columns => {
                let gaps = effective_gaps(count, gaps);
                columns(count, area.inset(gaps.outer), gaps.inner)
            }
            LayoutKind::Rows => {
                let gaps = effective_gaps(count, gaps);
                rows(count, area.inset(gaps.outer), gaps.inner)
            }
            LayoutKind::Grid => grid(count, area, gaps),
            LayoutKind::Center => center(count, area, gaps, ratios),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::Dwindle => "dwindle",
            LayoutKind::Master => "master",
            LayoutKind::Monocle => "monocle",
            LayoutKind::Columns => "columns",
            LayoutKind::Rows => "rows",
            LayoutKind::Grid => "grid",
            LayoutKind::Center => "center",
        }
    }

    /// The layout `step` places after this one in `cycle` (wrapping). A
    /// layout not in the cycle steps to the cycle's first one.
    pub fn cycled(self, cycle: &[LayoutKind], step: isize) -> Self {
        if cycle.is_empty() {
            return self;
        }
        match cycle.iter().position(|l| *l == self) {
            Some(i) => cycle[(i as isize + step).rem_euclid(cycle.len() as isize) as usize],
            None => cycle[0],
        }
    }

    /// The next layout in the default `cycle-layout` order.
    pub fn next(self) -> Self {
        self.cycled(&Self::DEFAULT_CYCLE, 1)
    }

    /// Adjusts `ratios` so the window at `index` of `count` grows by `delta`
    /// (shrinks if negative). Returns whether anything changed.
    pub fn resize(self, ratios: &mut Ratios, index: usize, count: usize, delta: f32) -> bool {
        if count < 2 {
            return false;
        }
        let (split, sign) = match self {
            LayoutKind::Monocle | LayoutKind::Columns | LayoutKind::Rows | LayoutKind::Grid => {
                return false;
            }
            LayoutKind::Center if index == 0 => (0, 1.0),
            LayoutKind::Center => (0, -1.0),
            // Window i owns the first part of split i; the last window owns
            // the second part of the final split.
            LayoutKind::Dwindle if index < count - 1 => (index, 1.0),
            LayoutKind::Dwindle => (count - 2, -1.0),
            LayoutKind::Master if count <= ratios.master_count.max(1) => return false,
            LayoutKind::Master if index < ratios.master_count.max(1) => (0, 1.0),
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
            LayoutKind::Monocle | LayoutKind::Columns | LayoutKind::Rows | LayoutKind::Grid => {
                Vec::new()
            }
            LayoutKind::Master | LayoutKind::Center => vec![0],
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

/// The first `ratios.master_count` windows share the master area on
/// `ratios.master_side` at the master ratio; the rest split the other part.
pub fn master(count: usize, area: Rect, gaps: Gaps, ratios: &Ratios) -> Vec<Rect> {
    let (n, side) = (ratios.master_count.max(1), ratios.master_side);
    let gaps = effective_gaps(count, gaps);
    let area = area.inset(gaps.outer);
    // Lay out masters-on-the-left in a (possibly transposed) frame, then
    // map the rects back.
    let transposed = matches!(side, Side::Top | Side::Bottom);
    let t = |r: Rect| {
        if transposed {
            Rect::new(r.y, r.x, r.h, r.w)
        } else {
            r
        }
    };
    let frame = t(area);
    let mut out = if count <= n {
        rows(count, frame, gaps.inner)
    } else {
        let usable = (frame.w - gaps.inner).max(0);
        let mw = (usable as f32 * ratios.get(0)).round() as i32;
        let main = Rect::new(frame.x, frame.y, mw, frame.h);
        let stack = Rect::new(frame.x + mw + gaps.inner, frame.y, usable - mw, frame.h);
        let mut out = rows(n, main, gaps.inner);
        out.extend(rows(count - n, stack, gaps.inner));
        out
    };
    for r in &mut out {
        *r = t(*r);
        if matches!(side, Side::Right | Side::Bottom) {
            *r = flip(*r, area, side == Side::Right, side == Side::Bottom);
        }
    }
    out
}

pub fn monocle(count: usize, area: Rect, gaps: Gaps) -> Vec<Rect> {
    // Monocle shows one window at a time, so smart gaps treat it as alone.
    let gaps = effective_gaps(1, gaps);
    vec![area.inset(gaps.outer); count]
}

/// Equal-width columns across `area`, left to right, with `gap` between
/// them. The last column absorbs any rounding remainder so the columns cover
/// the area exactly.
pub fn columns(count: usize, area: Rect, gap: i32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let usable = (area.w - gap * (n - 1)).max(0);
    let base = usable / n;
    (0..n)
        .map(|i| {
            let w = if i == n - 1 {
                usable - base * (n - 1)
            } else {
                base
            };
            Rect::new(area.x + (base + gap) * i, area.y, w, area.h)
        })
        .collect()
}

/// Columns of rows, as square as possible: `ceil(sqrt(n))` columns, the
/// first ones taking an extra row when the count doesn't divide evenly.
pub fn grid(count: usize, area: Rect, gaps: Gaps) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let gaps = effective_gaps(count, gaps);
    let cols = (count as f64).sqrt().ceil() as usize;
    let (per, extra) = (count / cols, count % cols);
    columns(cols, area.inset(gaps.outer), gaps.inner)
        .into_iter()
        .enumerate()
        .flat_map(|(i, col)| rows(per + usize::from(i < extra), col, gaps.inner))
        .collect()
}

/// The first window in the middle at the master ratio, the second on its
/// left and the rest in rows on its right. Two windows split like `master`.
pub fn center(count: usize, area: Rect, gaps: Gaps, ratios: &Ratios) -> Vec<Rect> {
    if count < 3 {
        let ratios = Ratios {
            master_count: 1,
            master_side: Side::Left,
            ..ratios.clone()
        };
        return master(count, area, gaps, &ratios);
    }
    let gaps = effective_gaps(count, gaps);
    let area = area.inset(gaps.outer);
    let usable = (area.w - 2 * gaps.inner).max(0);
    let mw = (usable as f32 * ratios.get(0)).round() as i32;
    let side = (usable - mw) / 2;
    let left = Rect::new(area.x, area.y, side, area.h);
    let main = Rect::new(area.x + side + gaps.inner, area.y, mw, area.h);
    let rx = main.right() + gaps.inner;
    let right = Rect::new(rx, area.y, area.right() - rx, area.h);
    let mut out = vec![main, left];
    out.extend(rows(count - 2, right, gaps.inner));
    out
}

/// Mirrors `r` inside `area`: left-right when `h`, top-bottom when `v`.
pub fn flip(r: Rect, area: Rect, h: bool, v: bool) -> Rect {
    let x = if h {
        area.x + area.right() - r.right()
    } else {
        r.x
    };
    let y = if v {
        area.y + area.bottom() - r.bottom()
    } else {
        r.y
    };
    Rect::new(x, y, r.w, r.h)
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
    fn master_count_and_side() {
        let area = Rect::new(0, 0, 1000, 800);
        let mut r = Ratios {
            master_count: 2,
            ..Ratios::default()
        };
        assert_eq!(
            master(3, area, NO_GAPS, &r),
            vec![
                Rect::new(0, 0, 500, 400),
                Rect::new(0, 400, 500, 400),
                Rect::new(500, 0, 500, 800),
            ]
        );
        // All masters: they share the whole area.
        assert_eq!(
            master(2, area, NO_GAPS, &r),
            vec![Rect::new(0, 0, 1000, 400), Rect::new(0, 400, 1000, 400)]
        );
        r.master_count = 1;
        r.master_side = Side::Right;
        assert_eq!(
            master(2, area, NO_GAPS, &r),
            vec![Rect::new(500, 0, 500, 800), Rect::new(0, 0, 500, 800)]
        );
        r.master_side = Side::Top;
        assert_eq!(
            master(3, area, NO_GAPS, &r),
            vec![
                Rect::new(0, 0, 1000, 400),
                Rect::new(0, 400, 500, 400),
                Rect::new(500, 400, 500, 400),
            ]
        );
        r.master_side = Side::Bottom;
        assert_eq!(
            master(2, area, NO_GAPS, &r)[0],
            Rect::new(0, 400, 1000, 400)
        );
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
        let cycle = [LayoutKind::Dwindle, LayoutKind::Grid];
        assert_eq!(LayoutKind::Dwindle.cycled(&cycle, -1), LayoutKind::Grid);
        assert_eq!(LayoutKind::Grid.cycled(&cycle, 1), LayoutKind::Dwindle);
        assert_eq!(LayoutKind::Master.cycled(&cycle, 1), LayoutKind::Dwindle);
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
        let rects = columns(3, Rect::new(0, 0, 1000, 500), 0);
        assert_eq!(
            rects,
            vec![
                Rect::new(0, 0, 333, 500),
                Rect::new(333, 0, 333, 500),
                Rect::new(666, 0, 334, 500),
            ]
        );
    }

    #[test]
    fn columns_and_rows_with_gaps() {
        let gaps = Gaps {
            inner: 10,
            outer: 10,
            smart: true,
        };
        let area = Rect::new(0, 0, 1000, 500);
        assert_eq!(
            LayoutKind::Columns.arrange(2, area, gaps, &Ratios::default()),
            vec![Rect::new(10, 10, 485, 480), Rect::new(505, 10, 485, 480)]
        );
        assert_eq!(
            LayoutKind::Rows.arrange(2, area, gaps, &Ratios::default()),
            vec![Rect::new(10, 10, 980, 235), Rect::new(10, 255, 980, 235)]
        );
        assert_eq!(
            LayoutKind::Columns.arrange(1, area, gaps, &Ratios::default()),
            vec![area]
        );
    }

    #[test]
    fn grid_is_square_ish() {
        // 5 windows: 3 columns, the first two with two rows each.
        let rects = grid(5, Rect::new(0, 0, 900, 600), NO_GAPS);
        assert_eq!(
            rects,
            vec![
                Rect::new(0, 0, 300, 300),
                Rect::new(0, 300, 300, 300),
                Rect::new(300, 0, 300, 300),
                Rect::new(300, 300, 300, 300),
                Rect::new(600, 0, 300, 600),
            ]
        );
        assert_eq!(grid(4, AREA, NO_GAPS).len(), 4);
    }

    #[test]
    fn center_puts_the_first_window_in_the_middle() {
        let rects = center(4, Rect::new(0, 0, 2000, 1000), NO_GAPS, &Ratios::default());
        assert_eq!(
            rects,
            vec![
                Rect::new(500, 0, 1000, 1000),
                Rect::new(0, 0, 500, 1000),
                Rect::new(1500, 0, 500, 500),
                Rect::new(1500, 500, 500, 500),
            ]
        );
        let mut r = Ratios::default();
        assert!(LayoutKind::Center.resize(&mut r, 0, 4, 0.1));
        assert_eq!(r.get(0), 0.6);
        assert_eq!(center(2, AREA, NO_GAPS, &r).len(), 2);
    }

    #[test]
    fn flip_mirrors_inside_the_area() {
        let area = Rect::new(100, 0, 1000, 500);
        let r = Rect::new(100, 0, 300, 100);
        assert_eq!(flip(r, area, true, false), Rect::new(800, 0, 300, 100));
        assert_eq!(flip(r, area, false, true), Rect::new(100, 400, 300, 100));
        assert_eq!(flip(flip(r, area, true, true), area, true, true), r);
    }
}
