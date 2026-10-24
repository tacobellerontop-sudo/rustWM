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
    /// A window filling the work area, if any.
    pub maximized: Option<WindowHandle>,
    pub layout: LayoutKind,
    pub ratios: Ratios,
    /// How dwindle divides the space; see [`Tree`].
    pub tree: Tree,
    /// Groups of windows sharing one tile (komorebi stacks, Hyprland
    /// groups). The member that is in `tiled` is shown; the rest are hidden.
    pub stacks: Vec<Vec<WindowHandle>>,
    /// The layout is mirrored left-right.
    pub flip_h: bool,
    /// The layout is mirrored top-bottom.
    pub flip_v: bool,
}

impl Workspace {
    pub fn new(name: impl Into<String>, layout: LayoutKind) -> Self {
        Self {
            name: name.into(),
            tiled: Vec::new(),
            floating: Vec::new(),
            focused: None,
            fullscreen: None,
            maximized: None,
            layout,
            ratios: Ratios::default(),
            tree: Tree::default(),
            stacks: Vec::new(),
            flip_h: false,
            flip_v: false,
        }
    }

    pub fn contains(&self, h: WindowHandle) -> bool {
        self.tiled.contains(&h) || self.floating.contains(&h) || self.stack_of(h).is_some()
    }

    pub fn is_floating(&self, h: WindowHandle) -> bool {
        self.floating.contains(&h)
    }

    pub fn is_empty(&self) -> bool {
        self.tiled.is_empty() && self.floating.is_empty()
    }

    /// Every window on screen when the workspace is: tiled in layout order,
    /// then floating. Leaves out windows hidden in stacks.
    pub fn windows(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        self.tiled.iter().chain(&self.floating).copied()
    }

    /// Every window, including those hidden behind others in a stack.
    pub fn all(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        self.windows().chain(self.stacked_hidden())
    }

    /// Windows hidden behind the shown member of their stack.
    pub fn stacked_hidden(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        self.stacks
            .iter()
            .flatten()
            .copied()
            .filter(|h| !self.tiled.contains(h))
    }

    /// The index of the stack `h` is in.
    pub fn stack_of(&self, h: WindowHandle) -> Option<usize> {
        self.stacks.iter().position(|s| s.contains(&h))
    }

    /// Puts `new` in tiled window `old`'s slot, keeping focus, fullscreen and
    /// maximize on the slot.
    fn replace_tiled(&mut self, old: WindowHandle, new: WindowHandle) {
        if let Some(i) = self.index_of(old) {
            self.tiled[i] = new;
            self.tree.replace(old, new);
            for slot in [&mut self.focused, &mut self.fullscreen, &mut self.maximized] {
                if *slot == Some(old) {
                    *slot = Some(new);
                }
            }
        }
    }

    /// Takes `h` out of its stack. If it was the shown member, the next one
    /// takes over its slot and is returned (the caller shows it); a stack
    /// left with one window stops being a stack. `h` itself leaves `tiled`.
    pub fn leave_stack(&mut self, h: WindowHandle) -> Option<WindowHandle> {
        let si = self.stack_of(h)?;
        let shown = self.tiled.contains(&h);
        let stack = &mut self.stacks[si];
        let i = stack.iter().position(|x| *x == h)?;
        stack.remove(i);
        let next = (shown && !stack.is_empty()).then(|| stack[i.min(stack.len() - 1)]);
        if stack.len() <= 1 {
            self.stacks.remove(si);
        }
        match next {
            Some(n) => {
                self.replace_tiled(h, n);
                Some(n)
            }
            None => {
                self.tiled.retain(|x| *x != h);
                self.tree.remove(h);
                None
            }
        }
    }

    /// Moves tiled window `h` into tiled window `target`'s tile (joining its
    /// stack, or starting one) and shows `h` there. Returns whether it did.
    pub fn stack_onto(&mut self, h: WindowHandle, target: WindowHandle) -> bool {
        if h == target || self.index_of(h).is_none() || self.index_of(target).is_none() {
            return false;
        }
        let focused = self.focused;
        self.leave_stack(h);
        self.tiled.retain(|x| *x != h);
        self.tree.remove(h);
        let si = self.stack_of(target).unwrap_or_else(|| {
            self.stacks.push(vec![target]);
            self.stacks.len() - 1
        });
        let at = self.stacks[si]
            .iter()
            .position(|x| *x == target)
            .unwrap_or(0)
            + 1;
        self.stacks[si].insert(at, h);
        self.replace_tiled(target, h);
        if focused == Some(h) || focused == Some(target) {
            self.focused = Some(h);
        }
        true
    }

    /// Shows the member `step` places from shown member `h` in its stack.
    /// Returns the window now shown.
    pub fn cycle_stack(&mut self, h: WindowHandle, step: isize) -> Option<WindowHandle> {
        let stack = &self.stacks[self.stack_of(h)?];
        if !self.tiled.contains(&h) {
            return None;
        }
        let i = stack.iter().position(|x| *x == h)? as isize;
        let next = stack[(i + step).rem_euclid(stack.len() as isize) as usize];
        if next == h {
            return None;
        }
        self.replace_tiled(h, next);
        Some(next)
    }

    /// Shows hidden stack member `h` in its stack's tile.
    pub fn bring_to_front(&mut self, h: WindowHandle) {
        let Some(si) = self.stack_of(h) else {
            return;
        };
        if let Some(front) = self.stacks[si]
            .iter()
            .copied()
            .find(|x| self.tiled.contains(x))
        {
            self.replace_tiled(front, h);
        }
    }

    /// Takes shown member `h` out of its stack into a tile of its own next
    /// to it. Returns whether it did.
    pub fn unstack(&mut self, h: WindowHandle) -> bool {
        if !self.tiled.contains(&h) {
            return false;
        }
        let Some(next) = self.leave_stack(h) else {
            return false;
        };
        self.tree.insert(h, Some(next), false);
        let at = self.index_of(next).map_or(self.tiled.len(), |i| i + 1);
        self.tiled.insert(at, h);
        self.focused = Some(h);
        true
    }

    /// Stacks every tile into the focused one (or the first).
    pub fn stack_all(&mut self) {
        let Some(&first) = self.tiled.first() else {
            return;
        };
        let target = self
            .focused
            .filter(|f| self.tiled.contains(f))
            .unwrap_or(first);
        for other in self.tiled.clone() {
            if other == target {
                continue;
            }
            let members = match self.stack_of(other) {
                Some(si) => self.stacks.remove(si),
                None => vec![other],
            };
            self.tiled.retain(|x| *x != other);
            self.tree.remove(other);
            let si = self.stack_of(target).unwrap_or_else(|| {
                self.stacks.push(vec![target]);
                self.stacks.len() - 1
            });
            self.stacks[si].extend(members);
        }
    }

    /// Gives every stacked window a tile of its own again.
    pub fn unstack_all(&mut self) {
        for stack in std::mem::take(&mut self.stacks) {
            let Some(mut anchor) = stack.iter().copied().find(|h| self.tiled.contains(h)) else {
                continue;
            };
            for h in stack {
                if self.tiled.contains(&h) {
                    continue;
                }
                self.tree.insert(h, Some(anchor), false);
                let at = self.index_of(anchor).map_or(self.tiled.len(), |i| i + 1);
                self.tiled.insert(at, h);
                anchor = h;
            }
        }
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
        if self.stack_of(h).is_some() {
            let was_focused = self.focused == Some(h);
            let next = self.leave_stack(h);
            if let Some(n) = next {
                // Its stack-mate took the slot (and focus, fullscreen, maximize).
                if was_focused {
                    self.focused = Some(n);
                }
                return true;
            }
            if !self.tiled.contains(&h) && !self.floating.contains(&h) {
                // A hidden member, or the last of its stack already untiled.
                if self.focused == Some(h) {
                    self.focused = self.tiled.last().copied();
                }
                return true;
            }
        }
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
        if self.maximized == Some(h) {
            self.maximized = None;
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
    /// A stack-mate takes its slot if it had one.
    pub fn untile(&mut self, h: WindowHandle) {
        if self.stack_of(h).is_some() {
            self.leave_stack(h);
        }
        self.tiled.retain(|x| *x != h);
        self.tree.remove(h);
        if self.maximized == Some(h) {
            self.maximized = None;
        }
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
    /// The workspace shown before the active one, for `workspace last`.
    pub last_active: Option<usize>,
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
            last_active: None,
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

    #[test]
    fn stacking_shares_a_tile() {
        let mut w = Workspace::new("1", LayoutKind::Dwindle);
        for i in 1..=3 {
            w.insert(WindowHandle(i), InsertPos::End);
        }
        w.focused = Some(WindowHandle(3));
        assert!(w.stack_onto(WindowHandle(3), WindowHandle(1)));
        // 3 shows in 1's slot; 1 is hidden behind it.
        assert_eq!(ids(&w), vec![3, 2]);
        assert_eq!(w.stacks, vec![vec![WindowHandle(1), WindowHandle(3)]]);
        assert_eq!(
            w.stacked_hidden().collect::<Vec<_>>(),
            vec![WindowHandle(1)]
        );
        assert!(w.contains(WindowHandle(1)));
        assert_eq!(w.focused, Some(WindowHandle(3)));

        assert_eq!(w.cycle_stack(WindowHandle(3), 1), Some(WindowHandle(1)));
        assert_eq!(ids(&w), vec![1, 2]);
        assert_eq!(w.focused, Some(WindowHandle(1)));

        // Closing the shown member reveals the next one in the same slot.
        assert!(w.remove(WindowHandle(1)));
        assert_eq!(ids(&w), vec![3, 2]);
        assert!(w.stacks.is_empty());
        assert_eq!(w.focused, Some(WindowHandle(3)));
    }

    #[test]
    fn unstack_and_stack_all() {
        let mut w = Workspace::new("1", LayoutKind::Dwindle);
        for i in 1..=4 {
            w.insert(WindowHandle(i), InsertPos::End);
        }
        w.focused = Some(WindowHandle(2));
        w.stack_all();
        assert_eq!(ids(&w), vec![2]);
        assert_eq!(w.all().count(), 4);
        assert!(w.unstack(WindowHandle(2)));
        assert_eq!(w.tiled.len(), 2);
        assert_eq!(w.focused, Some(WindowHandle(2)));
        w.unstack_all();
        assert_eq!(w.tiled.len(), 4);
        assert!(w.stacks.is_empty());
        assert_eq!(w.tree.leaves().len(), 4);
    }

    #[test]
    fn removing_a_hidden_member_keeps_the_slot() {
        let mut w = Workspace::new("1", LayoutKind::Dwindle);
        w.insert(WindowHandle(1), InsertPos::End);
        w.insert(WindowHandle(2), InsertPos::End);
        w.stack_onto(WindowHandle(2), WindowHandle(1));
        w.remove(WindowHandle(1));
        assert_eq!(ids(&w), vec![2]);
        assert!(w.stacks.is_empty());
        w.untile(WindowHandle(2));
        assert!(w.tiled.is_empty() && w.tree.leaves().is_empty());
    }
}
