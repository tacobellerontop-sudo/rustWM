use serde::{Deserialize, Serialize};

use crate::layout::{LayoutKind, Ratios};
use crate::{MonitorInfo, WindowHandle};

/// Where a newly managed window goes in its workspace's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InsertPos {
    #[default]
    AfterFocused,
    End,
    Start,
}

/// An ordered list of tiled windows plus the layout that places them.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub name: String,
    /// Tiled windows; list position is layout position.
    pub tiled: Vec<WindowHandle>,
    pub focused: Option<WindowHandle>,
    pub layout: LayoutKind,
    pub ratios: Ratios,
}

impl Workspace {
    pub fn new(name: impl Into<String>, layout: LayoutKind) -> Self {
        Self {
            name: name.into(),
            tiled: Vec::new(),
            focused: None,
            layout,
            ratios: Ratios::default(),
        }
    }

    pub fn contains(&self, h: WindowHandle) -> bool {
        self.tiled.contains(&h)
    }

    pub fn index_of(&self, h: WindowHandle) -> Option<usize> {
        self.tiled.iter().position(|x| *x == h)
    }

    pub fn insert(&mut self, h: WindowHandle, pos: InsertPos) {
        let at = match pos {
            InsertPos::Start => 0,
            InsertPos::End => self.tiled.len(),
            InsertPos::AfterFocused => self
                .focused
                .and_then(|f| self.index_of(f))
                .map_or(self.tiled.len(), |i| i + 1),
        };
        self.tiled.insert(at, h);
    }

    /// Removes a window. If it was focused, focus moves to the window that
    /// took its place (or the new last one).
    pub fn remove(&mut self, h: WindowHandle) -> bool {
        let Some(i) = self.index_of(h) else {
            return false;
        };
        self.tiled.remove(i);
        if self.focused == Some(h) {
            self.focused = self
                .tiled
                .get(i.min(self.tiled.len().saturating_sub(1)))
                .copied();
        }
        true
    }

    pub fn swap(&mut self, a: WindowHandle, b: WindowHandle) {
        if let (Some(i), Some(j)) = (self.index_of(a), self.index_of(b)) {
            self.tiled.swap(i, j);
        }
    }

    /// The window `step` places away from `from` in list order, wrapping.
    pub fn cycle(&self, from: WindowHandle, step: isize) -> Option<WindowHandle> {
        let i = self.index_of(from)? as isize;
        let n = self.tiled.len() as isize;
        (n > 1).then(|| self.tiled[(i + step).rem_euclid(n) as usize])
    }
}

/// A physical monitor and the workspaces that live on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub info: MonitorInfo,
    pub workspaces: Vec<Workspace>,
    pub active: usize,
}

impl Monitor {
    pub fn new(info: MonitorInfo, layout: LayoutKind) -> Self {
        Self {
            info,
            workspaces: vec![Workspace::new("1", layout)],
            active: 0,
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(ids: &[isize]) -> Workspace {
        let mut w = Workspace::new("1", LayoutKind::Dwindle);
        w.tiled = ids.iter().map(|i| WindowHandle(*i)).collect();
        w
    }

    fn ids(w: &Workspace) -> Vec<isize> {
        w.tiled.iter().map(|h| h.0).collect()
    }

    #[test]
    fn insert_positions() {
        let mut w = ws(&[1, 2, 3]);
        w.focused = Some(WindowHandle(2));
        w.insert(WindowHandle(9), InsertPos::AfterFocused);
        assert_eq!(ids(&w), vec![1, 2, 9, 3]);
        w.insert(WindowHandle(8), InsertPos::Start);
        w.insert(WindowHandle(7), InsertPos::End);
        assert_eq!(ids(&w), vec![8, 1, 2, 9, 3, 7]);
    }

    #[test]
    fn insert_after_focused_without_focus_appends() {
        let mut w = ws(&[1]);
        w.insert(WindowHandle(2), InsertPos::AfterFocused);
        assert_eq!(ids(&w), vec![1, 2]);
    }

    #[test]
    fn removing_focused_moves_focus_to_successor() {
        let mut w = ws(&[1, 2, 3]);
        w.focused = Some(WindowHandle(2));
        w.remove(WindowHandle(2));
        assert_eq!(w.focused, Some(WindowHandle(3)));
        w.remove(WindowHandle(3));
        assert_eq!(w.focused, Some(WindowHandle(1)));
        w.remove(WindowHandle(1));
        assert_eq!(w.focused, None);
    }

    #[test]
    fn cycle_wraps() {
        let w = ws(&[1, 2, 3]);
        assert_eq!(w.cycle(WindowHandle(3), 1), Some(WindowHandle(1)));
        assert_eq!(w.cycle(WindowHandle(1), -1), Some(WindowHandle(3)));
        assert_eq!(ws(&[1]).cycle(WindowHandle(1), 1), None);
    }
}
