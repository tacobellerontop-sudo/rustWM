use std::collections::{BTreeSet, HashMap};

use tracing::{debug, info, warn};

use crate::command::{Command, Target};
use crate::geometry::Direction;
use crate::layout::{Gaps, LayoutKind};
use crate::workspace::{InsertPos, Monitor};
use crate::{Event, Platform, Rect, WindowHandle};

/// Whether the event loop should keep running after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// User-tunable behaviour. Filled from the config file in M3.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub gaps: Gaps,
    pub new_window: InsertPos,
    pub default_layout: LayoutKind,
    /// Command line for the `terminal` command; `None` tries `wt.exe`, then `cmd.exe`.
    pub terminal: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gaps: Gaps::default(),
            new_window: InsertPos::AfterFocused,
            default_layout: LayoutKind::Dwindle,
            terminal: None,
        }
    }
}

const FALLBACK_TERMINALS: &[&str] = &["wt.exe", "cmd.exe"];

/// Where a window lives: monitor index and workspace index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Loc {
    monitor: usize,
    workspace: usize,
}

/// The window manager: owns all state and is the only thing that calls into
/// the [`Platform`].
pub struct Wm<P: Platform> {
    platform: P,
    settings: Settings,
    monitors: Vec<Monitor>,
    focused_monitor: usize,
    /// Windows rustWM itself hid. They must be shown again on exit or crash,
    /// and their `WindowHidden` events must not be read as closes.
    hidden_by_us: BTreeSet<WindowHandle>,
    /// The rect each tiled window was last given, so unchanged windows are not
    /// moved again and geometric focus works from the layout, not the OS.
    applied: HashMap<WindowHandle, Rect>,
}

impl<P: Platform> Wm<P> {
    pub fn new(platform: P) -> Self {
        Self::with_settings(platform, Settings::default())
    }

    pub fn with_settings(platform: P, settings: Settings) -> Self {
        Self {
            platform,
            settings,
            monitors: Vec::new(),
            focused_monitor: 0,
            hidden_by_us: BTreeSet::new(),
            applied: HashMap::new(),
        }
    }

    /// Reads monitors, adopts every manageable window already on screen onto
    /// the monitor it is on, and tiles them.
    pub fn start(&mut self) -> crate::platform::Result<()> {
        let layout = self.settings.default_layout;
        self.monitors = self
            .platform
            .monitors()?
            .into_iter()
            .map(|m| {
                info!(id = %m.id, work_area = ?m.work_area, scale = m.scale, "monitor");
                Monitor::new(m, layout)
            })
            .collect();
        let foreground = self.platform.foreground();
        for w in self.platform.windows()? {
            if w.manageable && !w.minimized {
                debug!(handle = ?w.handle, exe = %w.exe, title = %w.title, "adopting window");
                let m = self.monitor_at(w.rect).unwrap_or(0);
                if let Some(mon) = self.monitors.get_mut(m) {
                    mon.workspace_mut().insert(w.handle, InsertPos::End);
                }
            }
        }
        for mon in &mut self.monitors {
            let ws = mon.workspace_mut();
            ws.focused = ws.tiled.first().copied();
        }
        if let Some(fg) = foreground {
            self.set_focus_state(fg);
        }
        info!(count = self.managed().count(), "windows adopted");
        self.apply();
        Ok(())
    }

    pub fn handle(&mut self, event: Event) -> Flow {
        debug!(?event, "event");
        match event {
            Event::WindowShown(h) | Event::WindowRestored(h) | Event::WindowTitleChanged(h) => {
                if !self.hidden_by_us.contains(&h) && self.locate(h).is_none() {
                    self.manage(h);
                }
            }
            Event::WindowHidden(h) => {
                if !self.hidden_by_us.contains(&h) {
                    self.unmanage(h);
                }
            }
            Event::WindowDestroyed(h) => {
                self.hidden_by_us.remove(&h);
                self.unmanage(h);
            }
            Event::WindowMinimized(h) => self.unmanage(h),
            Event::WindowFocused(h) => {
                self.set_focus_state(h);
                return Flow::Continue;
            }
            Event::WindowMoveEnded(h) => self.window_dropped(h),
            Event::DisplayChanged => self.refresh_monitors(),
            Event::Command(cmd) => match cmd.parse::<Command>() {
                Ok(c) => return self.execute(c),
                Err(e) => warn!(command = %cmd, error = %e, "invalid command"),
            },
            Event::Shutdown => {
                self.restore();
                return Flow::Exit;
            }
        }
        self.apply();
        Flow::Continue
    }

    /// Runs one command and reapplies layouts.
    pub fn execute(&mut self, cmd: Command) -> Flow {
        debug!(%cmd, "command");
        match cmd {
            Command::Focus(target) => {
                if let Some(h) = self.neighbour(target) {
                    self.focus_window(h);
                }
            }
            Command::Move(target) => self.move_focused(target),
            Command::Promote => self.promote(),
            Command::Close => {
                if let Some(h) = self.focused() {
                    if let Err(e) = self.platform.close(h) {
                        warn!(error = %e, "close failed");
                    }
                }
            }
            Command::Terminal => self.spawn_terminal(),
            Command::Exec(line) => {
                if let Err(e) = self.platform.spawn(&line) {
                    warn!(error = %e, "exec failed");
                }
            }
            Command::Retile => self.applied.clear(),
            Command::Restore => self.restore(),
            Command::Quit => {
                self.restore();
                return Flow::Exit;
            }
        }
        self.apply();
        Flow::Continue
    }

    // ---- state queries ------------------------------------------------------

    fn locate(&self, h: WindowHandle) -> Option<Loc> {
        self.monitors.iter().enumerate().find_map(|(mi, m)| {
            m.workspaces
                .iter()
                .position(|ws| ws.contains(h))
                .map(|wi| Loc {
                    monitor: mi,
                    workspace: wi,
                })
        })
    }

    /// The monitor whose bounds contain the centre of `rect`.
    fn monitor_at(&self, rect: Rect) -> Option<usize> {
        let (x, y) = rect.center();
        self.monitors
            .iter()
            .position(|m| m.info.rect.contains_point(x, y))
    }

    /// The focused window, if the focused monitor's workspace has one.
    pub fn focused(&self) -> Option<WindowHandle> {
        self.monitors.get(self.focused_monitor)?.workspace().focused
    }

    /// Every tiled window on every visible workspace.
    pub fn managed(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        self.monitors
            .iter()
            .flat_map(|m| m.workspace().tiled.iter().copied())
    }

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn platform(&self) -> &P {
        &self.platform
    }

    pub fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }

    // ---- managing windows ---------------------------------------------------

    fn manage(&mut self, h: WindowHandle) {
        let Some(w) = self.platform.window_info(h) else {
            return;
        };
        if !w.manageable || w.minimized || self.monitors.is_empty() {
            return;
        }
        debug!(handle = ?h, exe = %w.exe, title = %w.title, "managing window");
        let pos = self.settings.new_window;
        let ws = self.monitors[self.focused_monitor].workspace_mut();
        ws.insert(h, pos);
        ws.focused = Some(h);
    }

    fn unmanage(&mut self, h: WindowHandle) {
        if let Some(loc) = self.locate(h) {
            self.monitors[loc.monitor].workspaces[loc.workspace].remove(h);
            self.applied.remove(&h);
        }
    }

    /// The user finished dragging `h`. Dropped on another monitor, it moves
    /// there; otherwise it snaps back into its slot.
    fn window_dropped(&mut self, h: WindowHandle) {
        let Some(loc) = self.locate(h) else {
            return;
        };
        self.applied.remove(&h);
        let target = self
            .platform
            .window_info(h)
            .and_then(|w| self.monitor_at(w.rect));
        if let Some(m) = target.filter(|m| *m != loc.monitor) {
            self.transfer(h, loc, m);
        }
    }

    /// Moves `h` to the active workspace of monitor `to` and keeps it focused.
    fn transfer(&mut self, h: WindowHandle, from: Loc, to: usize) {
        self.monitors[from.monitor].workspaces[from.workspace].remove(h);
        let pos = self.settings.new_window;
        let ws = self.monitors[to].workspace_mut();
        ws.insert(h, pos);
        ws.focused = Some(h);
        self.focused_monitor = to;
    }

    fn set_focus_state(&mut self, h: WindowHandle) {
        if let Some(loc) = self.locate(h) {
            self.focused_monitor = loc.monitor;
            self.monitors[loc.monitor].workspaces[loc.workspace].focused = Some(h);
        }
    }

    fn focus_window(&mut self, h: WindowHandle) {
        self.set_focus_state(h);
        if let Err(e) = self.platform.focus(h) {
            warn!(handle = ?h, error = %e, "focus failed");
        }
    }

    fn refresh_monitors(&mut self) {
        let infos = match self.platform.monitors() {
            Ok(m) if !m.is_empty() => m,
            Ok(_) => return,
            Err(e) => {
                warn!(error = %e, "failed to re-read monitors");
                return;
            }
        };
        let layout = self.settings.default_layout;
        let mut old = std::mem::take(&mut self.monitors);
        for info in infos {
            match old.iter().position(|m| m.info.id == info.id) {
                Some(i) => {
                    let mut m = old.remove(i);
                    m.info = info;
                    self.monitors.push(m);
                }
                None => self.monitors.push(Monitor::new(info, layout)),
            }
        }
        // Windows on disconnected monitors move to the primary one.
        for gone in old {
            for ws in gone.workspaces {
                for h in ws.tiled {
                    self.monitors[0].workspace_mut().insert(h, InsertPos::End);
                }
            }
        }
        self.focused_monitor = self.focused_monitor.min(self.monitors.len() - 1);
        self.applied.clear();
    }

    // ---- commands -----------------------------------------------------------

    /// The rect focus and movement start from: the focused window's slot, or
    /// the focused monitor's work area when its workspace is empty.
    fn origin(&self) -> Option<Rect> {
        match self.focused() {
            Some(h) => self.applied.get(&h).copied(),
            None => self
                .monitors
                .get(self.focused_monitor)
                .map(|m| m.info.work_area),
        }
    }

    fn neighbour(&self, target: Target) -> Option<WindowHandle> {
        match target {
            Target::Dir(dir) => {
                let from = self.origin()?;
                let me = self.focused();
                from.nearest(
                    dir,
                    self.managed()
                        .filter(|h| Some(*h) != me)
                        .filter_map(|h| self.applied.get(&h).map(|r| (h, *r))),
                )
            }
            Target::Next | Target::Prev => {
                let step = if target == Target::Next { 1 } else { -1 };
                let h = self.focused()?;
                self.monitors[self.focused_monitor]
                    .workspace()
                    .cycle(h, step)
            }
        }
    }

    /// The monitor in `dir` from the focused one, by work-area geometry.
    fn monitor_towards(&self, dir: Direction) -> Option<usize> {
        let from = self.monitors.get(self.focused_monitor)?.info.rect;
        from.nearest(
            dir,
            self.monitors
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != self.focused_monitor)
                .map(|(i, m)| (i, m.info.rect)),
        )
    }

    fn move_focused(&mut self, target: Target) {
        let Some(h) = self.focused() else {
            return;
        };
        let Some(loc) = self.locate(h) else {
            return;
        };
        match self.neighbour(target) {
            Some(other) => match self.locate(other) {
                Some(oloc) if oloc == loc => {
                    self.monitors[loc.monitor].workspaces[loc.workspace].swap(h, other);
                }
                Some(oloc) => self.transfer(h, loc, oloc.monitor),
                None => {}
            },
            // Nothing to swap with: if a monitor lies that way, move onto it.
            None => {
                if let Target::Dir(dir) = target {
                    if let Some(m) = self.monitor_towards(dir) {
                        self.transfer(h, loc, m);
                    }
                }
            }
        }
    }

    /// Swaps the focused window with the first slot; from the first slot,
    /// swaps with the second (like Hyprland's `swapwithmaster`).
    fn promote(&mut self) {
        let Some(h) = self.focused() else {
            return;
        };
        let ws = self.monitors[self.focused_monitor].workspace_mut();
        let other = match ws.index_of(h) {
            Some(0) => ws.tiled.get(1).copied(),
            Some(_) => ws.tiled.first().copied(),
            None => None,
        };
        if let Some(o) = other {
            ws.swap(h, o);
        }
    }

    fn spawn_terminal(&mut self) {
        if let Some(t) = self.settings.terminal.clone() {
            if let Err(e) = self.platform.spawn(&t) {
                warn!(error = %e, "terminal failed to start");
            }
            return;
        }
        for t in FALLBACK_TERMINALS {
            match self.platform.spawn(t) {
                Ok(()) => return,
                Err(e) => debug!(terminal = t, error = %e, "fallback terminal unavailable"),
            }
        }
        warn!("no terminal could be started; set `terminal` in the config");
    }

    // ---- applying to the OS -------------------------------------------------

    /// Computes every visible workspace's layout and moves the windows whose
    /// slot changed since the last apply.
    pub fn apply(&mut self) {
        let mut moves = Vec::new();
        for m in &self.monitors {
            let ws = m.workspace();
            let gaps = self.settings.gaps.scaled(m.info.scale);
            let rects = ws
                .layout
                .arrange(ws.tiled.len(), m.info.work_area, gaps, &ws.ratios);
            for (&h, rect) in ws.tiled.iter().zip(rects) {
                if self.applied.get(&h) != Some(&rect) {
                    moves.push((h, rect));
                }
            }
        }
        if moves.is_empty() {
            return;
        }
        for &(h, r) in &moves {
            self.applied.insert(h, r);
        }
        if let Err(e) = self.platform.set_rects(&moves) {
            warn!(error = %e, "some windows could not be moved");
        }
    }

    /// Hides a window and remembers that we did, so [`Wm::restore`] can undo it.
    pub fn hide(&mut self, handle: WindowHandle) {
        self.hidden_by_us.insert(handle);
        if let Err(e) = self.platform.hide(handle) {
            warn!(?handle, error = %e, "hide failed");
            self.hidden_by_us.remove(&handle);
        }
    }

    /// Shows every window rustWM hid. Called on exit and by the `restore` command.
    pub fn restore(&mut self) {
        for handle in std::mem::take(&mut self.hidden_by_us) {
            if let Err(e) = self.platform.show(handle) {
                warn!(?handle, error = %e, "restore failed");
            }
        }
    }
}
