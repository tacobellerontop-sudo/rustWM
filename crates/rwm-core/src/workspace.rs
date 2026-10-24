use serde::{Deserialize, Serialize};

use crate::layout::{LayoutKind, Ratios};
use crate::tree::Tree;
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

/// How workspaces behave across monitors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMode {
    /// Each monitor switches workspaces on its own.
    #[default]
    PerMonitor,
    /// Switching a workspace switches every monitor, as if all monitors
    /// were one big desktop.
    Global,
}

impl WorkspaceMode {
    pub fn name(self) -> &'static str {
        match self {
            WorkspaceMode::PerMonitor => "per-monitor",
            WorkspaceMode::Global => "global",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            WorkspaceMode::PerMonitor => WorkspaceMode::Global,
            WorkspaceMode::Global => WorkspaceMode::PerMonitor,
        }
    }
}

impl std::str::FromStr for WorkspaceMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [WorkspaceMode::PerMonitor, WorkspaceMode::Global]
            .into_iter()
            .find(|m| m.name() == s)
            .ok_or(())
    }
}

impl std::fmt::Display for WorkspaceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// An ordered list of tiled windows plus the layout that places them.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub name: String,
    /// Tiled windows; list position is layout position.
    pub tiled: Vec<WindowHandle>,
    /// Floating windows keep whatever rect they have.
    pub floating: Vec<WindowHandle>,
    pub focused: Option<WindowHandle>,
    /// A window covering the whole monitor, if any.
    pub fullscreen: Option<WindowHandle>,
    pub layout: LayoutKind,
    pub ratios: Ratios,
    /// How dwindle divides the space; see [`Tree`].
    pub tree: Tree,
}

impl Workspace {
    pub fn new(name: impl Into<String>, layout: LayoutKind) -> Self {
        Self {
            name: name.into(),
            tiled: Vec::new(),
            floating: Vec::new(),
            focused: None,
            fullscreen: None,
            layout,
            ratios: Ratios::default(),
            tree: Tree::default(),
        }
    }

    pub fn contains(&self, h: WindowHandle) -> bool {
        self.tiled.contains(&h) || self.floating.contains(&h)
    }

    pub fn is_floating(&self, h: WindowHandle) -> bool {
        self.floating.contains(&h)
    }

    pub fn is_empty(&self) -> bool {
        self.tiled.is_empty() && self.floating.is_empty()
    }

    /// Every window: tiled in layout order, then floating.
    pub fn windows(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        self.tiled.iter().chain(&self.floating).copied()
    }

    /// Adds a window as tiled (at `pos`) or floating.
    pub fn add(&mut self, h: WindowHandle, floating: bool, pos: InsertPos) {
        if floating {
            self.floating.push(h);
        } else {
            self.insert(h, pos);
        }
    }

    pub fn index_of(&self, h: WindowHandle) -> Option<usize> {
        self.tiled.iter().position(|x| *x == h)
    }

    pub fn insert(&mut self, h: WindowHandle, pos: InsertPos) {
        // Like Hyprland, the new window takes half of the focused one.
        match pos {
            InsertPos::AfterFocused => {
                let anchor = self.focused.filter(|f| self.tiled.contains(f));
                self.tree.insert(h, anchor, false);
            }
            InsertPos::End => self.tree.insert(h, None, false),
            InsertPos::Start => self.tree.insert(h, None, true),
        }
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

    /// Removes a window. If it was focused, focus moves to the tiled window
    /// that took its place (or the new last one), else to a floating one.
    pub fn remove(&mut self, h: WindowHandle) -> bool {
        self.tree.remove(h);
        let successor = if let Some(i) = self.index_of(h) {
            self.tiled.remove(i);
            self.tiled
                .get(i.min(self.tiled.len().saturating_sub(1)))
                .copied()
        } else if let Some(i) = self.floating.iter().position(|x| *x == h) {
            self.floating.remove(i);
            self.tiled.last().copied()
        } else {
            return false;
        };
        if self.focused == Some(h) {
            self.focused = successor.or_else(|| self.floating.last().copied());
        }
        if self.fullscreen == Some(h) {
            self.fullscreen = None;
        }
        true
    }

    pub fn swap(&mut self, a: WindowHandle, b: WindowHandle) {
        if let (Some(i), Some(j)) = (self.index_of(a), self.index_of(b)) {
            self.tiled.swap(i, j);
            self.tree.swap(a, b);
        }
    }

    /// Stops tiling `h` (e.g. it starts floating) without touching focus.
    pub fn untile(&mut self, h: WindowHandle) {
        self.tiled.retain(|x| *x != h);
        self.tree.remove(h);
    }

    /// The window `step` places away from `from` in list order (tiled, then
    /// floating), wrapping.
    pub fn cycle(&self, from: WindowHandle, step: isize) -> Option<WindowHandle> {
        let all: Vec<_> = self.windows().collect();
        let i = all.iter().position(|x| *x == from)? as isize;
        let n = all.len() as isize;
        (n > 1).then(|| all[(i + step).rem_euclid(n) as usize])
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
    /// A monitor with one workspace per name (at least one).
    pub fn new(info: MonitorInfo, layout: LayoutKind, names: &[String]) -> Self {
        let mut workspaces: Vec<_> = names
            .iter()
            .map(|n| Workspace::new(n.clone(), layout))
            .collect();
        if workspaces.is_empty() {
            workspaces.push(Workspace::new("1", layout));
        }
        Self {
            info,
            workspaces,
            active: 0,
        }
    }

    /// Finds a workspace by name, or by 1-based number.
    pub fn find_workspace(&self, name: &str) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|w| w.name == name)
            .or_else(|| {
                name.parse::<usize>()
                    .ok()
                    .filter(|n| (1..=self.workspaces.len()).contains(n))
                    .map(|n| n - 1)
            })
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
    fn floating_windows_are_separate_but_contained() {
        let mut w = ws(&[1]);
        w.add(WindowHandle(5), true, InsertPos::End);
        assert!(w.contains(WindowHandle(5)) && w.is_floating(WindowHandle(5)));
        assert_eq!(w.windows().map(|h| h.0).collect::<Vec<_>>(), vec![1, 5]);
        w.focused = Some(WindowHandle(1));
        w.remove(WindowHandle(1));
        assert_eq!(w.focused, Some(WindowHandle(5)));
    }

    #[test]
    fn removing_fullscreen_clears_it() {
        let mut w = ws(&[1, 2]);
        w.fullscreen = Some(WindowHandle(2));
        w.remove(WindowHandle(2));
        assert_eq!(w.fullscreen, None);
    }

    #[test]
    fn find_workspace_by_name_or_number() {
        let info = MonitorInfo {
            id: "m".into(),
            rect: Default::default(),
            work_area: Default::default(),
            scale: 1.0,
            primary: true,
        };
        let names: Vec<String> = ["web", "code", "chat"].map(String::from).into();
        let m = Monitor::new(info, LayoutKind::Dwindle, &names);
        assert_eq!(m.find_workspace("code"), Some(1));
        assert_eq!(m.find_workspace("3"), Some(2));
        assert_eq!(m.find_workspace("4"), None);
        assert_eq!(m.find_workspace("0"), None);
    }

    #[test]
    fn cycle_wraps() {
        let w = ws(&[1, 2, 3]);
        assert_eq!(w.cycle(WindowHandle(3), 1), Some(WindowHandle(1)));
        assert_eq!(w.cycle(WindowHandle(1), -1), Some(WindowHandle(3)));
        assert_eq!(ws(&[1]).cycle(WindowHandle(1), 1), None);
    }
}
