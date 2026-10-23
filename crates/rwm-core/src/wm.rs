use std::collections::{BTreeSet, HashMap};

use tracing::{debug, info, warn};

use crate::command::{Command, SPECIAL_PREFIX, Target};
use crate::geometry::Direction;
use crate::layout::{Gaps, LayoutKind};
use crate::rules::{self, Rule, Verdict};
use crate::state::{MonitorState, State, WindowState, WorkspaceState};
use crate::workspace::{InsertPos, Monitor, Workspace};
use crate::{Event, Platform, Rect, WindowHandle, WindowInfo};

/// Whether the event loop should keep running after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// The `reload` command ran: the caller should re-read the config and
    /// pass it to [`Wm::set_settings`].
    Reload,
    Exit,
}

/// User-tunable behaviour, filled from the config file.
#[derive(Debug, Clone)]
pub struct Settings {
    pub gaps: Gaps,
    pub new_window: InsertPos,
    pub default_layout: LayoutKind,
    /// Workspace names, created on every monitor.
    pub workspaces: Vec<String>,
    /// Command line for the `terminal` command; `None` tries `wt.exe`, then `cmd.exe`.
    pub terminal: Option<String>,
    pub rules: Vec<Rule>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gaps: Gaps::default(),
            new_window: InsertPos::AfterFocused,
            default_layout: LayoutKind::Dwindle,
            workspaces: (1..=9).map(|i| i.to_string()).collect(),
            terminal: None,
            rules: Vec::new(),
        }
    }
}

const FALLBACK_TERMINALS: &[&str] = &["wt.exe", "cmd.exe"];

/// The name of the normal key-binding mode.
pub const DEFAULT_MODE: &str = "default";

/// A shown special workspace covers this fraction of the work area.
const SPECIAL_SCALE: f32 = 0.9;

/// Where a window lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loc {
    /// A regular workspace: monitor index and workspace index.
    Ws { monitor: usize, workspace: usize },
    /// A special workspace, by index into `Wm::specials`.
    Special(usize),
}

/// What we remember about a managed window for queries.
#[derive(Debug, Clone, Default)]
struct Meta {
    title: String,
    exe: String,
    class: String,
}

impl From<&WindowInfo> for Meta {
    fn from(w: &WindowInfo) -> Self {
        Self {
            title: w.title.clone(),
            exe: w.exe.clone(),
            class: w.class.clone(),
        }
    }
}

/// The window manager: owns all state and is the only thing that calls into
/// the [`Platform`].
pub struct Wm<P: Platform> {
    platform: P,
    settings: Settings,
    monitors: Vec<Monitor>,
    focused_monitor: usize,
    /// Special (scratchpad) workspaces, created on first use.
    specials: Vec<Workspace>,
    /// The special workspace shown, and the monitor it is shown on.
    special_shown: Option<(usize, usize)>,
    /// Focus is inside the shown special workspace.
    special_focused: bool,
    mode: String,
    /// Windows rustWM itself hid. They must be shown again on exit or crash,
    /// and their `WindowHidden` events must not be read as closes.
    hidden_by_us: BTreeSet<WindowHandle>,
    /// Windows a rule said to ignore, so title changes don't re-check them.
    ignored: BTreeSet<WindowHandle>,
    /// The rect each tiled window was last given, so unchanged windows are not
    /// moved again and geometric focus works from the layout, not the OS.
    applied: HashMap<WindowHandle, Rect>,
    meta: HashMap<WindowHandle, Meta>,
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
            specials: Vec::new(),
            special_shown: None,
            special_focused: false,
            mode: DEFAULT_MODE.into(),
            hidden_by_us: BTreeSet::new(),
            ignored: BTreeSet::new(),
            applied: HashMap::new(),
            meta: HashMap::new(),
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
                Monitor::new(m, layout, &self.settings.workspaces)
            })
            .collect();
        let foreground = self.platform.foreground();
        for w in self.platform.windows()? {
            if !w.manageable || w.minimized {
                continue;
            }
            let monitor = self.monitor_at(w.rect).unwrap_or(0);
            let verdict = rules::evaluate(&self.settings.rules, &w);
            debug!(handle = ?w.handle, exe = %w.exe, title = %w.title, ?verdict, "adopting window");
            if self
                .place(w.handle, monitor, &verdict, InsertPos::End)
                .is_some()
            {
                self.meta.insert(w.handle, Meta::from(&w));
            }
        }
        for mon in &mut self.monitors {
            for ws in &mut mon.workspaces {
                let first = ws.windows().next();
                ws.focused = first;
            }
        }
        if let Some(fg) = foreground {
            self.set_focus_state(fg);
        }
        info!(count = self.all_windows().count(), "windows adopted");
        self.apply();
        Ok(())
    }

    /// Replaces the settings (after a config reload) and retiles.
    pub fn set_settings(&mut self, settings: Settings) {
        let layout = settings.default_layout;
        let names = settings.workspaces.clone();
        self.settings = settings;
        for mi in 0..self.monitors.len() {
            self.resize_workspaces(mi, &names, layout);
        }
        self.applied.clear();
        self.apply();
    }

    pub fn handle(&mut self, event: Event) -> Flow {
        debug!(?event, "event");
        match event {
            Event::WindowShown(h) | Event::WindowRestored(h) => self.maybe_manage(h),
            Event::WindowTitleChanged(h) => {
                if self.meta.contains_key(&h) {
                    if let Some(w) = self.platform.window_info(h) {
                        self.meta.insert(h, Meta::from(&w));
                    }
                    return Flow::Continue;
                }
                self.maybe_manage(h);
            }
            Event::WindowHidden(h) => {
                if !self.hidden_by_us.contains(&h) {
                    self.unmanage(h);
                }
            }
            Event::WindowDestroyed(h) => {
                self.hidden_by_us.remove(&h);
                self.ignored.remove(&h);
                self.unmanage(h);
            }
            Event::WindowMinimized(h) => self.unmanage(h),
            Event::WindowFocused(h) => {
                self.set_focus_state(h);
                return Flow::Continue;
            }
            Event::WindowMoveEnded(h) => self.window_dropped(h),
            Event::DisplayChanged => self.refresh_monitors(),
            Event::Command(cmd) | Event::Request { command: cmd, .. } => {
                match cmd.parse::<Command>() {
                    Ok(c) => return self.execute(c),
                    Err(e) => warn!(command = %cmd, error = %e, "invalid command"),
                }
            }
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
            Command::Workspace(name) => self.switch_workspace(&name),
            Command::MoveToWorkspace(name) => self.move_to_workspace(&name),
            Command::FocusMonitor(dir) => self.focus_monitor(dir),
            Command::MoveToMonitor(dir) => {
                if let (Some(h), Some(m)) = (self.focused(), self.monitor_towards(dir)) {
                    if let Some(loc) = self.locate(h) {
                        self.transfer(h, loc, m);
                    }
                }
            }
            Command::Layout(layout) => self.current_ws_mut().layout = layout,
            Command::CycleLayout => {
                let ws = self.current_ws_mut();
                ws.layout = ws.layout.next();
            }
            Command::LayoutReset => self.current_ws_mut().ratios.reset(),
            Command::Resize(delta) => self.resize(delta),
            Command::ToggleFloat => self.toggle_float(),
            Command::ToggleFullscreen => {
                if let Some(h) = self.focused() {
                    let ws = self.current_ws_mut();
                    ws.fullscreen = if ws.fullscreen == Some(h) {
                        None
                    } else {
                        Some(h)
                    };
                    self.applied.remove(&h);
                }
            }
            Command::ToggleSpecial(name) => self.toggle_special(&name),
            Command::Mode(mode) => {
                info!(%mode, "binding mode");
                self.mode = mode;
            }
            Command::Reload => return Flow::Reload,
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
        self.monitors
            .iter()
            .enumerate()
            .find_map(|(mi, m)| {
                m.workspaces
                    .iter()
                    .position(|ws| ws.contains(h))
                    .map(|wi| Loc::Ws {
                        monitor: mi,
                        workspace: wi,
                    })
            })
            .or_else(|| {
                self.specials
                    .iter()
                    .position(|ws| ws.contains(h))
                    .map(Loc::Special)
            })
    }

    fn ws(&self, loc: Loc) -> &Workspace {
        match loc {
            Loc::Ws { monitor, workspace } => &self.monitors[monitor].workspaces[workspace],
            Loc::Special(i) => &self.specials[i],
        }
    }

    fn ws_mut(&mut self, loc: Loc) -> &mut Workspace {
        match loc {
            Loc::Ws { monitor, workspace } => &mut self.monitors[monitor].workspaces[workspace],
            Loc::Special(i) => &mut self.specials[i],
        }
    }

    fn is_visible(&self, loc: Loc) -> bool {
        match loc {
            Loc::Ws { monitor, workspace } => self.monitors[monitor].active == workspace,
            Loc::Special(i) => self.special_shown.is_some_and(|(s, _)| s == i),
        }
    }

    /// The workspace commands act on: the shown special workspace when it has
    /// focus, otherwise the focused monitor's active workspace.
    fn current_loc(&self) -> Loc {
        match self.special_shown {
            Some((i, _)) if self.special_focused => Loc::Special(i),
            _ => Loc::Ws {
                monitor: self.focused_monitor,
                workspace: self.monitors[self.focused_monitor].active,
            },
        }
    }

    fn current_ws_mut(&mut self) -> &mut Workspace {
        let loc = self.current_loc();
        self.ws_mut(loc)
    }

    /// The monitor whose bounds contain the point.
    fn monitor_at_point(&self, (x, y): (i32, i32)) -> Option<usize> {
        self.monitors
            .iter()
            .position(|m| m.info.rect.contains_point(x, y))
    }

    /// The monitor whose bounds contain the centre of `rect`.
    fn monitor_at(&self, rect: Rect) -> Option<usize> {
        self.monitor_at_point(rect.center())
    }

    /// The focused window of the current workspace.
    pub fn focused(&self) -> Option<WindowHandle> {
        if self.monitors.is_empty() {
            return None;
        }
        self.ws(self.current_loc()).focused
    }

    pub fn focused_monitor(&self) -> usize {
        self.focused_monitor
    }

    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// Every window on screen: visible workspaces, then the shown special one.
    pub fn managed(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        let special = self.special_shown.map(|(i, _)| &self.specials[i]);
        self.monitors
            .iter()
            .flat_map(|m| m.workspace().windows())
            .chain(special.into_iter().flat_map(|ws| ws.windows()))
    }

    /// Every managed window on every workspace, visible or not.
    pub fn all_windows(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        self.monitors
            .iter()
            .flat_map(|m| m.workspaces.iter())
            .chain(&self.specials)
            .flat_map(|ws| ws.windows())
    }

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn platform(&self) -> &P {
        &self.platform
    }

    pub fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }

    /// A snapshot of everything, for queries and status bars.
    pub fn snapshot(&self) -> State {
        let window = |ws: &Workspace, h: WindowHandle| {
            let m = self.meta.get(&h).cloned().unwrap_or_default();
            WindowState {
                handle: h.0,
                title: m.title,
                exe: m.exe,
                class: m.class,
                floating: ws.is_floating(h),
                fullscreen: ws.fullscreen == Some(h),
            }
        };
        let workspace = |ws: &Workspace| WorkspaceState {
            name: ws.name.clone(),
            layout: ws.layout,
            focused: ws.focused.map(|h| h.0),
            windows: ws.windows().map(|h| window(ws, h)).collect(),
        };
        State {
            focused_monitor: self.focused_monitor,
            focused_window: self.focused().map(|h| {
                let ws = self.ws(self.current_loc());
                window(ws, h)
            }),
            mode: self.mode.clone(),
            special: self
                .special_shown
                .map(|(i, _)| self.specials[i].name.clone()),
            monitors: self
                .monitors
                .iter()
                .map(|m| MonitorState {
                    id: m.info.id.clone(),
                    rect: m.info.rect,
                    work_area: m.info.work_area,
                    scale: m.info.scale,
                    active_workspace: m.workspace().name.clone(),
                    workspaces: m.workspaces.iter().map(workspace).collect(),
                })
                .collect(),
        }
    }

    // ---- managing windows ---------------------------------------------------

    fn maybe_manage(&mut self, h: WindowHandle) {
        if !self.hidden_by_us.contains(&h) && !self.ignored.contains(&h) && self.locate(h).is_none()
        {
            self.manage(h);
        }
    }

    fn manage(&mut self, h: WindowHandle) {
        let Some(w) = self.platform.window_info(h) else {
            return;
        };
        if !w.manageable || w.minimized || self.monitors.is_empty() {
            return;
        }
        let mut verdict = rules::evaluate(&self.settings.rules, &w);
        debug!(handle = ?h, exe = %w.exe, title = %w.title, ?verdict, "managing window");
        if verdict.ignore {
            self.ignored.insert(h);
            return;
        }
        // While a special workspace has focus, new windows open inside it.
        if verdict.workspace.is_none() && self.special_focused {
            if let Some((i, _)) = self.special_shown {
                verdict.workspace = Some(format!("{SPECIAL_PREFIX}{}", self.specials[i].name));
            }
        }
        let pos = self.settings.new_window;
        if let Some(loc) = self.place(h, self.focused_monitor, &verdict, pos) {
            self.meta.insert(h, Meta::from(&w));
            if self.is_visible(loc) {
                self.ws_mut(loc).focused = Some(h);
            }
        }
    }

    /// Finds or creates the special workspace called `name`.
    fn special(&mut self, name: &str) -> usize {
        match self.specials.iter().position(|ws| ws.name == name) {
            Some(i) => i,
            None => {
                self.specials
                    .push(Workspace::new(name, self.settings.default_layout));
                self.specials.len() - 1
            }
        }
    }

    /// Resolves a workspace name on monitor `mi`: a regular name or number,
    /// or `special:<name>`.
    fn resolve(&mut self, mi: usize, name: &str) -> Option<Loc> {
        if let Some(special) = name.strip_prefix(SPECIAL_PREFIX) {
            return Some(Loc::Special(self.special(special)));
        }
        self.monitors[mi].find_workspace(name).map(|wi| Loc::Ws {
            monitor: mi,
            workspace: wi,
        })
    }

    /// Adds `h` on monitor `mi`, on the workspace a rule names (hidden if that
    /// workspace is not visible) or the active one. Returns where it went.
    fn place(
        &mut self,
        h: WindowHandle,
        mi: usize,
        verdict: &Verdict,
        pos: InsertPos,
    ) -> Option<Loc> {
        if verdict.ignore {
            self.ignored.insert(h);
            return None;
        }
        let active = Loc::Ws {
            monitor: mi,
            workspace: self.monitors.get(mi)?.active,
        };
        let loc = match verdict.workspace.as_deref() {
            Some(name) => self.resolve(mi, name).unwrap_or_else(|| {
                warn!(workspace = name, "rule names an unknown workspace");
                active
            }),
            None => active,
        };
        self.ws_mut(loc).add(h, verdict.float, pos);
        if !self.is_visible(loc) {
            self.hide(h);
        }
        Some(loc)
    }

    fn unmanage(&mut self, h: WindowHandle) {
        if let Some(loc) = self.locate(h) {
            self.ws_mut(loc).remove(h);
            self.applied.remove(&h);
            self.meta.remove(&h);
        }
    }

    /// The user finished dragging `h`. A tiled window dropped onto another
    /// tiled window swaps with it; dropped on another monitor it moves there;
    /// otherwise it snaps back into its slot.
    fn window_dropped(&mut self, h: WindowHandle) {
        let Some(loc) = self.locate(h) else {
            return;
        };
        let floating = self.ws(loc).is_floating(h);
        self.applied.remove(&h);
        let point = self
            .platform
            .cursor_pos()
            .or_else(|| self.platform.window_info(h).map(|w| w.rect.center()));
        let Some(point) = point else {
            return;
        };
        let under = (!floating)
            .then(|| {
                self.managed().find(|o| {
                    *o != h
                        && self
                            .applied
                            .get(o)
                            .is_some_and(|r| r.contains_point(point.0, point.1))
                })
            })
            .flatten();
        let from_monitor = match loc {
            Loc::Ws { monitor, .. } => Some(monitor),
            Loc::Special(_) => None,
        };
        let target_monitor = self.monitor_at_point(point);
        if let Some(m) =
            target_monitor.filter(|m| Some(*m) != from_monitor && from_monitor.is_some())
        {
            self.transfer_keep_rect(h, loc, m);
        }
        if let (Some(other), Some(now)) = (under, self.locate(h)) {
            if self.locate(other) == Some(now) && !self.ws(now).is_floating(other) {
                self.ws_mut(now).swap(h, other);
            }
        }
    }

    /// Moves `h` to the active workspace of monitor `to` and keeps it focused.
    /// Floating windows are recentred on the new monitor.
    fn transfer(&mut self, h: WindowHandle, from: Loc, to: usize) {
        if self.transfer_keep_rect(h, from, to) {
            self.center_floating(h, to);
        }
    }

    /// Like [`Wm::transfer`] but leaves a floating window where it is.
    /// Returns whether the window is floating.
    fn transfer_keep_rect(&mut self, h: WindowHandle, from: Loc, to: usize) -> bool {
        let floating = self.ws(from).is_floating(h);
        self.ws_mut(from).remove(h);
        let pos = self.settings.new_window;
        let ws = self.monitors[to].workspace_mut();
        ws.add(h, floating, pos);
        ws.focused = Some(h);
        self.focused_monitor = to;
        self.special_focused = false;
        floating
    }

    fn set_focus_state(&mut self, h: WindowHandle) {
        match self.locate(h) {
            Some(loc @ Loc::Ws { monitor, .. }) if self.is_visible(loc) => {
                self.focused_monitor = monitor;
                self.special_focused = false;
                self.ws_mut(loc).focused = Some(h);
            }
            Some(loc @ Loc::Special(_)) if self.is_visible(loc) => {
                self.special_focused = true;
                if let Some((_, m)) = self.special_shown {
                    self.focused_monitor = m;
                }
                self.ws_mut(loc).focused = Some(h);
            }
            _ => {}
        }
    }

    fn focus_window(&mut self, h: WindowHandle) {
        self.set_focus_state(h);
        if let Err(e) = self.platform.focus(h) {
            warn!(handle = ?h, error = %e, "focus failed");
        }
    }

    /// Makes the workspace list of monitor `mi` match `names`: renames by
    /// position, adds missing ones, and folds removed ones into the last.
    fn resize_workspaces(&mut self, mi: usize, names: &[String], layout: LayoutKind) {
        let names: Vec<String> = if names.is_empty() {
            vec!["1".into()]
        } else {
            names.to_vec()
        };
        let mon = &mut self.monitors[mi];
        while mon.workspaces.len() < names.len() {
            mon.workspaces.push(Workspace::new("", layout));
        }
        let removed: Vec<_> = mon.workspaces.drain(names.len()..).collect();
        for (ws, name) in mon.workspaces.iter_mut().zip(&names) {
            ws.name = name.clone();
        }
        let last = names.len() - 1;
        let was_active_removed = mon.active > last;
        mon.active = mon.active.min(last);
        for ws in removed {
            for h in ws.windows().collect::<Vec<_>>() {
                let floating = ws.is_floating(h);
                self.monitors[mi].workspaces[last].add(h, floating, InsertPos::End);
                if self.monitors[mi].active == last {
                    self.show(h);
                }
            }
        }
        if was_active_removed {
            let hidden: Vec<_> = self.monitors[mi].workspace().windows().collect();
            for h in hidden {
                self.show(h);
            }
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
                None => self
                    .monitors
                    .push(Monitor::new(info, layout, &self.settings.workspaces)),
            }
        }
        // Windows on disconnected monitors move to the primary monitor's
        // workspace with the same position, keeping its visibility rules.
        for gone in old {
            for (wi, ws) in gone.workspaces.into_iter().enumerate() {
                let target = wi.min(self.monitors[0].workspaces.len() - 1);
                for h in ws.windows().collect::<Vec<_>>() {
                    self.monitors[0].workspaces[target].add(h, ws.is_floating(h), InsertPos::End);
                    if target == self.monitors[0].active {
                        self.show(h);
                    } else {
                        self.hide(h);
                    }
                }
            }
        }
        self.focused_monitor = self.focused_monitor.min(self.monitors.len() - 1);
        if let Some((i, m)) = self.special_shown {
            self.special_shown = Some((i, m.min(self.monitors.len() - 1)));
        }
        self.applied.clear();
    }

    // ---- commands -----------------------------------------------------------

    /// The rect focus and movement start from: the focused window's slot, or
    /// the focused monitor's work area when its workspace is empty.
    fn origin(&self) -> Option<Rect> {
        match self.focused() {
            Some(h) => self.rect_of(h),
            None => self
                .monitors
                .get(self.focused_monitor)
                .map(|m| m.info.work_area),
        }
    }

    /// Where a visible window is: its layout slot, or the OS rect if floating.
    fn rect_of(&self, h: WindowHandle) -> Option<Rect> {
        self.applied
            .get(&h)
            .copied()
            .or_else(|| self.platform.window_info(h).map(|w| w.rect))
    }

    fn neighbour(&self, target: Target) -> Option<WindowHandle> {
        let current = self.current_loc();
        // In monocle every window has the same rect, so directions walk the list.
        let monocle = self.ws(current).layout == LayoutKind::Monocle && self.focused().is_some();
        match target {
            Target::Dir(Direction::Left | Direction::Up) if monocle => self.neighbour(Target::Prev),
            Target::Dir(Direction::Right | Direction::Down) if monocle => {
                self.neighbour(Target::Next)
            }
            Target::Dir(dir) => {
                let from = self.origin()?;
                let me = self.focused();
                // Inside a shown special workspace, stay inside it.
                let in_special = matches!(current, Loc::Special(_));
                from.nearest(
                    dir,
                    self.managed()
                        .filter(|h| Some(*h) != me)
                        .filter(|h| !in_special || self.ws(current).contains(*h))
                        .filter_map(|h| self.rect_of(h).map(|r| (h, r))),
                )
            }
            Target::Next | Target::Prev => {
                let step = if target == Target::Next { 1 } else { -1 };
                self.ws(current).cycle(self.focused()?, step)
            }
        }
    }

    /// The monitor in `dir` from the focused one, by geometry.
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

    fn focus_monitor(&mut self, dir: Direction) {
        let Some(m) = self.monitor_towards(dir) else {
            return;
        };
        self.focused_monitor = m;
        self.special_focused = false;
        if let Some(h) = self.monitors[m].workspace().focused {
            self.focus_window(h);
        }
    }

    fn move_focused(&mut self, target: Target) {
        let Some(h) = self.focused() else {
            return;
        };
        let Some(loc) = self.locate(h) else {
            return;
        };
        if self.ws(loc).is_floating(h) {
            return;
        }
        match self.neighbour(target) {
            Some(other) => match self.locate(other) {
                Some(oloc) if oloc == loc => {
                    let ws = self.ws_mut(loc);
                    if !ws.is_floating(other) {
                        ws.swap(h, other);
                    }
                }
                Some(Loc::Ws { monitor, .. }) => self.transfer(h, loc, monitor),
                _ => {}
            },
            // Nothing to swap with: if a monitor lies that way, move onto it.
            None => {
                if let (Target::Dir(dir), Loc::Ws { .. }) = (target, loc) {
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
        let ws = self.current_ws_mut();
        let other = match ws.index_of(h) {
            Some(0) => ws.tiled.get(1).copied(),
            Some(_) => ws.tiled.first().copied(),
            None => None,
        };
        if let Some(o) = other {
            ws.swap(h, o);
        }
    }

    fn resize(&mut self, delta: f32) {
        let Some(h) = self.focused() else {
            return;
        };
        let ws = self.current_ws_mut();
        if let Some(i) = ws.index_of(h) {
            let (layout, count) = (ws.layout, ws.tiled.len());
            layout.resize(&mut ws.ratios, i, count, delta);
        }
    }

    fn toggle_float(&mut self) {
        let Some(h) = self.focused() else {
            return;
        };
        let pos = self.settings.new_window;
        let mi = self.focused_monitor;
        let ws = self.current_ws_mut();
        if ws.is_floating(h) {
            ws.floating.retain(|x| *x != h);
            ws.insert(h, pos);
        } else {
            ws.tiled.retain(|x| *x != h);
            ws.floating.push(h);
            self.applied.remove(&h);
            self.center_floating(h, mi);
        }
    }

    /// Centres a floating window on monitor `mi` at two thirds of its work area.
    fn center_floating(&mut self, h: WindowHandle, mi: usize) {
        let area = self.monitors[mi].info.work_area;
        let (w, h_) = (area.w * 2 / 3, area.h * 2 / 3);
        let rect = Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h_) / 2, w, h_);
        if let Err(e) = self.platform.set_rects(&[(h, rect)]) {
            warn!(error = %e, "could not place floating window");
        }
    }

    fn switch_workspace(&mut self, name: &str) {
        let mi = self.focused_monitor;
        let Some(target) = self.monitors.get(mi).and_then(|m| m.find_workspace(name)) else {
            warn!(workspace = name, "no such workspace");
            return;
        };
        self.special_focused = false;
        let mon = &self.monitors[mi];
        if target == mon.active {
            return;
        }
        let to_show: Vec<_> = mon.workspaces[target].windows().collect();
        let to_hide: Vec<_> = mon.workspace().windows().collect();
        // Show the new workspace before hiding the old one to avoid a flash
        // of desktop between the two.
        self.monitors[mi].active = target;
        for h in to_show {
            self.show(h);
        }
        for h in to_hide {
            self.hide(h);
        }
        match self.monitors[mi].workspace().focused {
            Some(h) => self.focus_window(h),
            None => debug!(workspace = name, "switched to empty workspace"),
        }
    }

    fn move_to_workspace(&mut self, name: &str) {
        let Some(h) = self.focused() else {
            return;
        };
        let (Some(from), Some(to)) = (self.locate(h), self.resolve(self.focused_monitor, name))
        else {
            warn!(workspace = name, "no such workspace");
            return;
        };
        if from == to {
            return;
        }
        let floating = self.ws(from).is_floating(h);
        self.ws_mut(from).remove(h);
        let pos = self.settings.new_window;
        let dst = self.ws_mut(to);
        dst.add(h, floating, pos);
        dst.focused = Some(h);
        self.applied.remove(&h);
        if self.is_visible(to) {
            // Still on screen (e.g. out of a shown special workspace): keep it focused.
            self.show(h);
            self.focus_window(h);
        } else {
            self.hide(h);
            if let Some(next) = self.ws(from).focused {
                self.focus_window(next);
            }
        }
    }

    fn toggle_special(&mut self, name: &str) {
        let idx = self.special(name);
        let shown = self.special_shown;
        if let Some((current, _)) = shown {
            for h in self.specials[current].windows().collect::<Vec<_>>() {
                self.hide(h);
            }
            self.special_shown = None;
            self.special_focused = false;
            if current == idx {
                if let Some(h) = self.monitors[self.focused_monitor].workspace().focused {
                    self.focus_window(h);
                }
                return;
            }
        }
        self.special_shown = Some((idx, self.focused_monitor));
        self.special_focused = true;
        let windows: Vec<_> = self.specials[idx].windows().collect();
        if windows.is_empty() {
            // An empty scratchpad is no use; open the terminal in it.
            self.spawn_terminal();
            return;
        }
        for h in windows {
            self.applied.remove(&h);
            self.show(h);
        }
        if let Some(h) = self.specials[idx].focused {
            self.focus_window(h);
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
        let mut slots = Vec::new();
        for m in &self.monitors {
            let gaps = self.settings.gaps.scaled(m.info.scale);
            slots.extend(arrange(m.workspace(), m.info.work_area, m.info.rect, gaps));
        }
        if let Some((i, mi)) = self.special_shown {
            if let Some(m) = self.monitors.get(mi) {
                let area = m.info.work_area;
                let (w, h) = (
                    (area.w as f32 * SPECIAL_SCALE) as i32,
                    (area.h as f32 * SPECIAL_SCALE) as i32,
                );
                let overlay = Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h);
                let gaps = self.settings.gaps.scaled(m.info.scale);
                slots.extend(arrange(&self.specials[i], overlay, overlay, gaps));
            }
        }
        let moves: Vec<_> = slots
            .into_iter()
            .filter(|(h, r)| self.applied.get(h) != Some(r))
            .collect();
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

    fn show(&mut self, handle: WindowHandle) {
        if self.hidden_by_us.remove(&handle) {
            if let Err(e) = self.platform.show(handle) {
                warn!(?handle, error = %e, "show failed");
            }
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

/// The target rect of every tiled (and fullscreen) window of a workspace.
fn arrange(ws: &Workspace, area: Rect, full: Rect, gaps: Gaps) -> Vec<(WindowHandle, Rect)> {
    let fullscreen = ws.fullscreen.filter(|h| ws.contains(*h));
    let tiled: Vec<_> = ws
        .tiled
        .iter()
        .copied()
        .filter(|h| Some(*h) != fullscreen)
        .collect();
    let rects = ws.layout.arrange(tiled.len(), area, gaps, &ws.ratios);
    tiled
        .into_iter()
        .zip(rects)
        .chain(fullscreen.map(|h| (h, full)))
        .collect()
}
