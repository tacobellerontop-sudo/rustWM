//! Hyprland-style dwindle: a binary tree where a new window splits the
//! focused window's space in half, leaving every other window where it is.
//!
//! The workspace's ordered `tiled` list stays the source of truth for which
//! windows tile; the tree only remembers how their space is divided. Each
//! split cuts along the longer side of its rect, like Hyprland's default.
//!
//! Windows can have a minimum size (a console that won't go below so many
//! columns). Splits move just enough to give such windows their minimum when
//! the space allows, instead of letting them overflow onto their neighbours.

use std::collections::HashMap;

use crate::layout::{DEFAULT_RATIO, Gaps, MAX_RATIO, MIN_RATIO};
use crate::{Rect, WindowHandle};

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Leaf(WindowHandle),
    Split {
        /// Share of the first child.
        ratio: f32,
        /// Split along the shorter side instead of the longer one
        /// (`toggle-split`).
        flipped: bool,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// Known minimum (width, height) of windows, in physical pixels.
pub type Mins = HashMap<WindowHandle, (i32, i32)>;

/// The least space a window without a known minimum is squeezed down to when
/// a neighbour needs room.
const FLOOR: (i32, i32) = (150, 100);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    root: Option<Node>,
}

impl Node {
    fn contains(&self, h: WindowHandle) -> bool {
        match self {
            Node::Leaf(x) => *x == h,
            Node::Split { first, second, .. } => first.contains(h) || second.contains(h),
        }
    }

    fn leaves(&self, out: &mut Vec<WindowHandle>) {
        match self {
            Node::Leaf(x) => out.push(*x),
            Node::Split { first, second, .. } => {
                first.leaves(out);
                second.leaves(out);
            }
        }
    }

    /// Replaces leaf `anchor` with a split of it and `h`. Returns whether
    /// `anchor` was found.
    fn split_leaf(&mut self, anchor: WindowHandle, h: WindowHandle, new_first: bool) -> bool {
        match self {
            Node::Leaf(x) if *x == anchor => {
                let (a, b) = if new_first { (h, anchor) } else { (anchor, h) };
                *self = Node::Split {
                    ratio: DEFAULT_RATIO,
                    flipped: false,
                    first: Box::new(Node::Leaf(a)),
                    second: Box::new(Node::Leaf(b)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                first.split_leaf(anchor, h, new_first) || second.split_leaf(anchor, h, new_first)
            }
        }
    }

    /// Removes leaf `h`; its sibling takes the parent's place. Returns the
    /// node that should replace `self` (`None` if `self` was that leaf).
    fn without(self, h: WindowHandle) -> Option<Node> {
        match self {
            Node::Leaf(x) if x == h => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split {
                ratio,
                flipped,
                first,
                second,
            } => match (first.without(h), second.without(h)) {
                (Some(a), Some(b)) => Some(Node::Split {
                    ratio,
                    flipped,
                    first: Box::new(a),
                    second: Box::new(b),
                }),
                (Some(n), None) | (None, Some(n)) => Some(n),
                (None, None) => None,
            },
        }
    }

    fn rename(&mut self, from: WindowHandle, to: WindowHandle) {
        match self {
            Node::Leaf(x) if *x == from => *x = to,
            Node::Leaf(_) => {}
            Node::Split { first, second, .. } => {
                first.rename(from, to);
                second.rename(from, to);
            }
        }
    }

    fn arrange(&self, r: Rect, gap: i32, mins: &Mins, out: &mut Vec<(WindowHandle, Rect)>) {
        match self {
            Node::Leaf(h) => out.push((*h, r)),
            Node::Split {
                ratio,
                flipped,
                first,
                second,
            } => {
                let ratio = fitted_ratio(*ratio, *flipped, first, second, r, gap, mins);
                let (a, b) = split(r, ratio, gap, *flipped);
                first.arrange(a, gap, mins, out);
                second.arrange(b, gap, mins, out);
            }
        }
    }
}

/// `ratio`, moved if needed so both children get at least their minimum size
/// along the split. If they can't both fit, the space is shared in
/// proportion to their minimums.
fn fitted_ratio(
    ratio: f32,
    flipped: bool,
    first: &Node,
    second: &Node,
    r: Rect,
    gap: i32,
    mins: &Mins,
) -> f32 {
    if mins.is_empty() {
        return ratio;
    }
    let across = side_by_side(r, flipped);
    let along = |(w, h): (i32, i32)| if across { w } else { h };
    let usable = (along((r.w, r.h)) - gap).max(1);
    let wanted = (usable as f32 * ratio).round() as i32;
    // A child's minimum depends on its own shape (dwindle splits the longer
    // side), which depends on this split: go round until it settles.
    let mut ratio_now = ratio;
    for _ in 0..4 {
        let (a, b) = split(r, ratio_now, gap, flipped);
        let need_a = along(first.min_size(a, gap, mins));
        let need_b = along(second.min_size(b, gap, mins));
        let first_len = if need_a + need_b > usable {
            (usable as i64 * need_a as i64 / (need_a + need_b).max(1) as i64) as i32
        } else {
            wanted.max(need_a).min(usable - need_b)
        };
        let next = first_len as f32 / usable as f32;
        if (next - ratio_now).abs() < f32::EPSILON {
            break;
        }
        ratio_now = next;
    }
    ratio_now
}

impl Node {
    /// The smallest (width, height) this subtree can take in `r`.
    fn min_size(&self, r: Rect, gap: i32, mins: &Mins) -> (i32, i32) {
        match self {
            Node::Leaf(h) => mins.get(h).copied().unwrap_or(FLOOR),
            Node::Split {
                ratio,
                flipped,
                first,
                second,
            } => {
                let (a, b) = split(r, *ratio, gap, *flipped);
                let (aw, ah) = first.min_size(a, gap, mins);
                let (bw, bh) = second.min_size(b, gap, mins);
                if side_by_side(r, *flipped) {
                    (aw + gap + bw, ah.max(bh))
                } else {
                    (aw.max(bw), ah + gap + bh)
                }
            }
        }
    }

    /// The ratios of the splits above leaf `h`, nearest first, as mutable
    /// references, with whether `h` is in the first child.
    fn ancestors(&mut self, h: WindowHandle) -> Vec<(&mut f32, bool)> {
        match self {
            Node::Leaf(_) => Vec::new(),
            Node::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if first.contains(h) {
                    let mut v = first.ancestors(h);
                    v.push((ratio, true));
                    v
                } else if second.contains(h) {
                    let mut v = second.ancestors(h);
                    v.push((ratio, false));
                    v
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn reset(&mut self) {
        if let Node::Split {
            ratio,
            flipped,
            first,
            second,
        } = self
        {
            *ratio = DEFAULT_RATIO;
            *flipped = false;
            first.reset();
            second.reset();
        }
    }

    /// The `flipped` flag of the split right above leaf `h`.
    fn parent_flip(&mut self, h: WindowHandle) -> Option<&mut bool> {
        let Node::Split {
            flipped,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };
        if matches!(**first, Node::Leaf(x) if x == h) || matches!(**second, Node::Leaf(x) if x == h)
        {
            return Some(flipped);
        }
        if first.contains(h) {
            first.parent_flip(h)
        } else {
            second.parent_flip(h)
        }
    }
}

/// Whether a split of `r` puts its halves side by side: along the longer
/// side, or the shorter one when `flipped`.
fn side_by_side(r: Rect, flipped: bool) -> bool {
    (r.w >= r.h) != flipped
}

/// Splits `r` at `ratio` with `gap` between the halves: along its longer
/// side, or its shorter one when `flipped`.
fn split(r: Rect, ratio: f32, gap: i32, flipped: bool) -> (Rect, Rect) {
    if side_by_side(r, flipped) {
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

impl Tree {
    pub fn contains(&self, h: WindowHandle) -> bool {
        self.root.as_ref().is_some_and(|n| n.contains(h))
    }

    /// Windows in tree order (left/top first).
    pub fn leaves(&self) -> Vec<WindowHandle> {
        let mut out = Vec::new();
        if let Some(n) = &self.root {
            n.leaves(&mut out);
        }
        out
    }

    /// Adds `h` by splitting `anchor` in half (`h` goes second, or first if
    /// `new_first`). Without a usable anchor it splits the last window, or
    /// the first one when `new_first`, which builds the classic dwindle spiral.
    pub fn insert(&mut self, h: WindowHandle, anchor: Option<WindowHandle>, new_first: bool) {
        if self.contains(h) {
            return;
        }
        let Some(root) = &mut self.root else {
            self.root = Some(Node::Leaf(h));
            return;
        };
        let leaves = {
            let mut v = Vec::new();
            root.leaves(&mut v);
            v
        };
        let anchor = anchor.filter(|a| leaves.contains(a)).or(if new_first {
            leaves.first().copied()
        } else {
            leaves.last().copied()
        });
        if let Some(a) = anchor {
            root.split_leaf(a, h, new_first);
        }
    }

    pub fn remove(&mut self, h: WindowHandle) {
        self.root = self.root.take().and_then(|n| n.without(h));
    }

    /// Puts `new` where `old` is (for stacks, whose shown member changes).
    pub fn replace(&mut self, old: WindowHandle, new: WindowHandle) {
        if old != new && !self.contains(new) {
            if let Some(root) = &mut self.root {
                root.rename(old, new);
            }
        }
    }

    /// Exchanges the places of two windows.
    pub fn swap(&mut self, a: WindowHandle, b: WindowHandle) {
        if a == b || !self.contains(a) || !self.contains(b) {
            return;
        }
        if let Some(root) = &mut self.root {
            // Three-way rename through a handle no real window uses.
            let tmp = WindowHandle(isize::MIN);
            root.rename(a, tmp);
            root.rename(b, a);
            root.rename(tmp, b);
        }
    }

    /// The rects of `windows` inside `area`. Leaves not in `windows` are left
    /// out (their sibling takes the space) and windows the tree doesn't know
    /// yet are added by splitting the last one.
    pub fn arrange(
        &self,
        windows: &[WindowHandle],
        area: Rect,
        gaps: Gaps,
        mins: &Mins,
    ) -> Vec<(WindowHandle, Rect)> {
        let tree = self.reconciled(windows);
        let gaps = if gaps.smart && windows.len() == 1 {
            Gaps {
                inner: 0,
                outer: 0,
                smart: true,
            }
        } else {
            gaps
        };
        let mut out = Vec::with_capacity(windows.len());
        if let Some(root) = &tree.root {
            root.arrange(area.inset(gaps.outer), gaps.inner, mins, &mut out);
        }
        out
    }

    /// A copy holding exactly `windows`.
    fn reconciled(&self, windows: &[WindowHandle]) -> Tree {
        let mut t = self.clone();
        for h in self.leaves() {
            if !windows.contains(&h) {
                t.remove(h);
            }
        }
        for &h in windows {
            t.insert(h, None, false);
        }
        t
    }

    /// Grows window `h` by `delta` (shrinks if negative) by moving the split
    /// nearest to it. Returns whether anything changed.
    pub fn resize(&mut self, h: WindowHandle, delta: f32) -> bool {
        let Some(root) = &mut self.root else {
            return false;
        };
        let Some((ratio, is_first)) = root.ancestors(h).into_iter().next() else {
            return false;
        };
        let sign = if is_first { 1.0 } else { -1.0 };
        let new = (*ratio + sign * delta).clamp(MIN_RATIO, MAX_RATIO);
        let changed = new != *ratio;
        *ratio = new;
        changed
    }

    /// Changes the splits around `h` so its rect comes as close as possible
    /// to `target`: used when the user drags a window's edge.
    pub fn fit(
        &mut self,
        h: WindowHandle,
        windows: &[WindowHandle],
        area: Rect,
        gaps: Gaps,
        mins: &Mins,
        target: Rect,
    ) -> bool {
        // Work on exactly what is on screen. Callers don't fit while a window
        // is hidden from the tree (fullscreen), so nothing is lost.
        let mut t = self.reconciled(windows);
        let error = |t: &Tree| {
            t.arrange(windows, area, gaps, mins)
                .into_iter()
                .find(|(w, _)| *w == h)
                .map_or(i32::MAX, |(_, r)| {
                    (r.x - target.x).abs()
                        + (r.y - target.y).abs()
                        + (r.right() - target.right()).abs()
                        + (r.bottom() - target.bottom()).abs()
                })
        };
        let depth = t.root.as_mut().map_or(0, |r| r.ancestors(h).len());
        let mut changed = false;
        // Two passes: an outer split shifts the space an inner one divides.
        for _ in 0..2 {
            for level in 0..depth.min(4) {
                let start = error(&t);
                let mut best = (start, None);
                let steps = ((MAX_RATIO - MIN_RATIO) / 0.0025) as usize;
                for step in 0..=steps {
                    let r = MIN_RATIO + step as f32 * 0.0025;
                    let mut c = t.clone();
                    if let Some(root) = &mut c.root {
                        *root.ancestors(h)[level].0 = r;
                    }
                    let e = error(&c);
                    if e < best.0 {
                        best = (e, Some(r));
                    }
                }
                if let (Some(r), Some(root)) = (best.1, &mut t.root) {
                    *root.ancestors(h)[level].0 = r;
                    changed = true;
                }
            }
        }
        if changed {
            *self = t;
        }
        changed
    }

    /// Turns the split holding `h` between side by side and stacked
    /// (Hyprland's `togglesplit`). Returns whether `h` has a split.
    pub fn toggle_split(&mut self, h: WindowHandle) -> bool {
        match self.root.as_mut().and_then(|r| r.parent_flip(h)) {
            Some(f) => {
                *f = !*f;
                true
            }
            None => false,
        }
    }

    /// Puts every split back to half and half, along the longer side.
    pub fn reset(&mut self) {
        if let Some(root) = &mut self.root {
            root.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Ratios, dwindle};

    const AREA: Rect = Rect::new(0, 0, 1920, 1040);
    const NO_GAPS: Gaps = Gaps {
        inner: 0,
        outer: 0,
        smart: false,
    };
    const W: fn(isize) -> WindowHandle = WindowHandle;

    fn ids(n: isize) -> Vec<WindowHandle> {
        (1..=n).map(W).collect()
    }

    fn rect_of(t: &Tree, windows: &[WindowHandle], h: isize) -> Rect {
        t.arrange(windows, AREA, NO_GAPS, &Mins::new())
            .into_iter()
            .find(|(w, _)| *w == W(h))
            .unwrap()
            .1
    }

    #[test]
    fn appending_builds_the_classic_spiral() {
        let mut t = Tree::default();
        for i in 1..=5 {
            t.insert(W(i), None, false);
        }
        let got: Vec<Rect> = t
            .arrange(&ids(5), AREA, NO_GAPS, &Mins::new())
            .into_iter()
            .map(|x| x.1)
            .collect();
        assert_eq!(got, dwindle(5, AREA, NO_GAPS, &Ratios::default()));
    }

    #[test]
    fn new_window_halves_the_focused_one_only() {
        // 1 | 2 over 3. Open 4 while 1 is focused: 1 is split, 2 and 3 stay.
        let mut t = Tree::default();
        for i in 1..=3 {
            t.insert(W(i), None, false);
        }
        let before2 = rect_of(&t, &ids(3), 2);
        let before3 = rect_of(&t, &ids(3), 3);
        t.insert(W(4), Some(W(1)), false);
        let all = ids(4);
        assert_eq!(rect_of(&t, &all, 2), before2);
        assert_eq!(rect_of(&t, &all, 3), before3);
        assert_eq!(rect_of(&t, &all, 1), Rect::new(0, 0, 960, 520));
        assert_eq!(rect_of(&t, &all, 4), Rect::new(0, 520, 960, 520));
        assert_eq!(t.leaves(), vec![W(1), W(4), W(2), W(3)]);
    }

    #[test]
    fn closing_gives_space_to_the_sibling() {
        let mut t = Tree::default();
        for i in 1..=3 {
            t.insert(W(i), None, false);
        }
        t.insert(W(4), Some(W(1)), false);
        t.remove(W(4));
        let all = ids(3);
        assert_eq!(rect_of(&t, &all, 1), Rect::new(0, 0, 960, 1040));
        t.remove(W(1));
        // The two left split side by side, as the space is wider than tall.
        assert_eq!(rect_of(&t, &[W(2), W(3)], 2), Rect::new(0, 0, 960, 1040));
    }

    #[test]
    fn arrange_skips_hidden_and_adds_unknown() {
        let mut t = Tree::default();
        t.insert(W(1), None, false);
        t.insert(W(2), None, false);
        // Window 2 fullscreen: 1 gets everything. Window 3 unknown: appended.
        assert_eq!(rect_of(&t, &[W(1)], 1), AREA);
        assert_eq!(
            t.arrange(&[W(1), W(2), W(3)], AREA, NO_GAPS, &Mins::new())
                .len(),
            3
        );
    }

    #[test]
    fn swap_resize_and_reset() {
        let mut t = Tree::default();
        t.insert(W(1), None, false);
        t.insert(W(2), None, false);
        t.swap(W(1), W(2));
        assert_eq!(rect_of(&t, &ids(2), 2).x, 0);
        assert!(t.resize(W(2), 0.1));
        assert_eq!(rect_of(&t, &ids(2), 2).w, 1152);
        assert!(t.resize(W(1), 0.1));
        assert_eq!(rect_of(&t, &ids(2), 1).w, 960);
        t.resize(W(1), 0.3);
        t.reset();
        assert_eq!(rect_of(&t, &ids(2), 1).w, 960);
    }

    #[test]
    fn fit_follows_a_dragged_edge() {
        let mut t = Tree::default();
        for i in 1..=3 {
            t.insert(W(i), None, false);
        }
        // Drag window 3's top edge (bottom right) up from 520 to 300.
        let target = Rect::new(960, 300, 960, 740);
        assert!(t.fit(W(3), &ids(3), AREA, NO_GAPS, &Mins::new(), target));
        let r = rect_of(&t, &ids(3), 3);
        assert!((r.y - 300).abs() <= 5, "{r:?}");
        assert_eq!(rect_of(&t, &ids(3), 1).w, 960);
    }

    #[test]
    fn splits_make_room_for_minimum_sizes() {
        let mut t = Tree::default();
        t.insert(W(1), None, false);
        t.insert(W(2), None, false);
        let mut mins = Mins::new();
        // Window 2 needs 1200px: window 1 gives way.
        mins.insert(W(2), (1200, 300));
        let got = t.arrange(&ids(2), AREA, NO_GAPS, &mins);
        assert_eq!(got[1].1, Rect::new(720, 0, 1200, 1040));
        assert_eq!(got[0].1, Rect::new(0, 0, 720, 1040));
        // Both need more than there is: shared by their minimums.
        mins.insert(W(1), (1200, 300));
        let got = t.arrange(&ids(2), AREA, NO_GAPS, &mins);
        assert_eq!(got[0].1.w, 960);

        // Nested: 1 | (2 over 3); 3 needs 1100 wide, so the outer split moves
        // and nobody is squeezed below the floor.
        let mut t = Tree::default();
        for i in 1..=3 {
            t.insert(W(i), None, false);
        }
        let mut mins = Mins::new();
        mins.insert(W(3), (1100, 200));
        let got = t.arrange(&ids(3), AREA, NO_GAPS, &mins);
        assert!(got[2].1.w >= 1100, "{got:?}");
        assert!(
            got.iter().all(|(_, r)| r.w >= FLOOR.0 && r.h >= FLOOR.1),
            "{got:?}"
        );
        let covered: i64 = got.iter().map(|(_, r)| r.w as i64 * r.h as i64).sum();
        assert_eq!(covered, 1920 * 1040);
    }
}
