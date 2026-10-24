use std::collections::{BTreeSet, HashMap};

use tracing::{debug, info, warn};

use crate::command::{Axis, Command, DEFAULT_PIXEL_STEP, SPECIAL_PREFIX, Target};
use crate::geometry::Direction;
use crate::layout::{self, Gaps, LayoutKind};
use crate::rules::{self, Rule, Verdict};
use crate::state::{MonitorState, State, WindowState, WorkspaceState};
use crate::tree::Mins;
use crate::workspace::{InsertPos, Monitor, Workspace, WorkspaceMode};
use crate::{Border, Event, Platform, Rect, WindowHandle, WindowInfo};

/// Whether the event loop should keep running after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// The `reload` command ran: the caller should re-read the config and
    /// pass it to [`Wm::set_settings`].
    Reload,
    Exit,
}

/// The focus border's look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderSettings {
    pub enabled: bool,
    /// Thickness at 100% scaling; scaled per monitor.
    pub width: i32,
    /// 0xRRGGBB.
    pub color: u32,
}

impl Default for BorderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 3,
            color: 0x89b4fa,
        }
    }
}

/// Per-workspace settings that override the general ones, by workspace name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceOverride {
    /// Layout the workspace starts with (and returns to on reload).
    pub layout: Option<LayoutKind>,
    pub gaps: Option<Gaps>,
}

/// User-tunable behaviour, filled from the config file.
#[derive(Debug, Clone)]
pub struct Settings {
    pub gaps: Gaps,
    pub new_window: InsertPos,
    pub default_layout: LayoutKind,
    /// Workspace names, created on every monitor.
    pub workspaces: Vec<String>,
    /// Whether monitors switch workspaces together or on their own.
    pub workspace_mode: WorkspaceMode,
    /// Focus the window under the mouse when the mouse moves onto it.
    pub focus_follows_mouse: bool,
    /// Put the mouse in the middle of the window keyboard commands focus.
    pub mouse_follows_focus: bool,
    /// Switching to the workspace already shown goes back to the previous one.
    pub workspace_back_and_forth: bool,
    /// The layouts `cycle-layout` goes through, in order.
    pub layout_cycle: Vec<LayoutKind>,
    /// Opacity of windows without focus, 0.1 to 1.0 (1.0 leaves them alone).
    pub inactive_opacity: f32,
    pub border: BorderSettings,
    /// Command line for the `terminal` command; `None` tries `wt.exe`, then `cmd.exe`.
    pub terminal: Option<String>,
    /// Command lines run once when rustWM starts (a status bar, say).
    pub exec_on_start: Vec<String>,
    /// Overrides by workspace name.
    pub workspace_overrides: HashMap<String, WorkspaceOverride>,
    pub rules: Vec<Rule>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gaps: Gaps::default(),
            new_window: InsertPos::AfterFocused,
            default_layout: LayoutKind::Dwindle,
            workspaces: (1..=9).map(|i| i.to_string()).collect(),
            workspace_mode: WorkspaceMode::default(),
            focus_follows_mouse: false,
            mouse_follows_focus: false,
            workspace_back_and_forth: false,
            layout_cycle: LayoutKind::DEFAULT_CYCLE.to_vec(),
            inactive_opacity: 1.0,
            border: BorderSettings::default(),
            terminal: None,
            exec_on_start: Vec::new(),
            workspace_overrides: HashMap::new(),
            rules: Vec::new(),
        }
    }
}

const FALLBACK_TERMINALS: &[&str] = &["wt.exe", "cmd.exe"];

/// A shown special workspace covers this fraction of the work area.
const SPECIAL_SCALE: f32 = 0.9;

/// How many previously focused windows `focus last` remembers.
const FOCUS_HISTORY: usize = 32;

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
    /// Windows rustWM itself hid. They must be shown again on exit or crash,
    /// and their `WindowHidden` events must not be read as closes.
    hidden_by_us: BTreeSet<WindowHandle>,
    /// Windows we hid whose `WindowHidden` event hasn't come back yet. It can
    /// arrive after we showed the window again (switching back quickly,
    /// cycling a stack) and must not be read as the app hiding itself.
    hide_events_pending: BTreeSet<WindowHandle>,
    /// Windows a rule said to ignore, so title changes don't re-check them.
    ignored: BTreeSet<WindowHandle>,
    /// The rect each tiled window was last given, so unchanged windows are not
    /// moved again and geometric focus works from the layout, not the OS.
    applied: HashMap<WindowHandle, Rect>,
    meta: HashMap<WindowHandle, Meta>,
    /// Windows that must tile even if they look fixed-size (a `tile` rule, or
    /// the user tiled them by hand).
    force_tiled: BTreeSet<WindowHandle>,
    /// Windows were moved and should be checked with [`Event::Verify`].
    verify_pending: bool,
    /// The window the user is dragging, if any.
    moving: Option<WindowHandle>,
    /// The border as last drawn.
    border: Option<Border>,
    /// Minimum sizes of windows that have one, asked for when they open or
    /// learned when they refuse a smaller slot.
    mins: Mins,
    /// We just focused the desktop; its focus event must not move the
    /// focused monitor to wherever the mouse is.
    desktop_focus_pending: bool,
    /// Recently focused windows, most recent last, for `focus last`.
    focus_history: Vec<WindowHandle>,
    /// Floating windows that stay on screen when their monitor switches
    /// workspace.
    pinned: BTreeSet<WindowHandle>,
    /// `toggle-pause`: leave windows alone until resumed.
    paused: bool,
    /// Opacity we set on windows, when not opaque.
    opacity: HashMap<WindowHandle, u8>,
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
            hidden_by_us: BTreeSet::new(),
            hide_events_pending: BTreeSet::new(),
            ignored: BTreeSet::new(),
            applied: HashMap::new(),
            meta: HashMap::new(),
            force_tiled: BTreeSet::new(),
            verify_pending: false,
            moving: None,
            border: None,
            mins: Mins::new(),
            desktop_focus_pending: false,
            focus_history: Vec::new(),
            pinned: BTreeSet::new(),
            paused: false,
            opacity: HashMap::new(),
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
        for mi in 0..self.monitors.len() {
            self.apply_workspace_layouts(mi);
        }
        let foreground = self.platform.foreground();
        for w in self.platform.windows()? {
            if !w.manageable || w.minimized {
                continue;
            }
            let verdict = self.verdict_for(&w);
            let monitor = verdict
                .monitor
                .filter(|m| *m < self.monitors.len())
                .or_else(|| self.monitor_at(w.rect))
                .unwrap_or(0);
            if let Some(min) = self.platform.min_size(w.handle) {
                self.mins.insert(w.handle, min);
            }
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
        self.update_border();
        self.update_opacity();
        for line in self.settings.exec_on_start.clone() {
            if let Err(e) = self.platform.spawn(&line) {
                warn!(command = %line, error = %e, "exec_on_start failed");
            }
        }
        Ok(())
    }

    /// Gives the workspaces of monitor `mi` the layout their
    /// `[workspace.<name>]` section asks for.
    fn apply_workspace_layouts(&mut self, mi: usize) {
        let overrides = &self.settings.workspace_overrides;
        for ws in &mut self.monitors[mi].workspaces {
            if let Some(layout) = overrides.get(&ws.name).and_then(|o| o.layout) {
                ws.layout = layout;
            }
        }
    }

    /// The gaps of a workspace at a monitor's scale.
    fn gaps_for(&self, ws: &Workspace, scale: f32) -> Gaps {
        self.settings
            .workspace_overrides
            .get(&ws.name)
            .and_then(|o| o.gaps)
            .unwrap_or(self.settings.gaps)
            .scaled(scale)
    }

    /// Replaces the settings (after a config reload) and retiles.
    pub fn set_settings(&mut self, settings: Settings) {
        let layout = settings.default_layout;
        let names = settings.workspaces.clone();
        self.settings = settings;
        for mi in 0..self.monitors.len() {
            self.resize_workspaces(mi, &names, layout);
            self.apply_workspace_layouts(mi);
        }
        self.align_workspaces();
        self.applied.clear();
        self.apply();
    }

    pub fn handle(&mut self, event: Event) -> Flow {
        let flow = self.handle_inner(event);
        if flow == Flow::Exit {
            self.border = None;
            self.platform.set_border(None);
        } else {
            self.update_border();
            self.update_opacity();
        }
        flow
    }

    fn handle_inner(&mut self, event: Event) -> Flow {
        debug!(?event, "event");
        if self.paused
            && matches!(
                event,
                Event::WindowShown(_)
                    | Event::WindowRestored(_)
                    | Event::WindowTitleChanged(_)
                    | Event::MouseOver(_)
                    | Event::Verify
            )
        {
            return Flow::Continue;
        }
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
                if !self.hide_events_pending.remove(&h) && !self.hidden_by_us.contains(&h) {
                    self.unmanage(h);
                }
            }
            Event::WindowDestroyed(h) => {
                self.hidden_by_us.remove(&h);
                self.hide_events_pending.remove(&h);
                self.ignored.remove(&h);
                self.unmanage(h);
            }
            Event::WindowMinimized(h) => self.unmanage(h),
            Event::WindowFocused(h) => {
                if self.locate(h).is_some() {
                    self.set_focus_state(h);
                } else if std::mem::take(&mut self.desktop_focus_pending) {
                    // We focused the desktop ourselves; the monitor stays.
                } else {
                    // The desktop, the taskbar or an unmanaged window: the
                    // user is working on the monitor under the mouse.
                    self.focus_monitor_under_cursor();
                }
                return Flow::Continue;
            }
            Event::WindowMoveStarted(h) => {
                self.moving = Some(h);
                return Flow::Continue;
            }
            Event::WindowMoveEnded(h) => {
                self.moving = None;
                if !self.paused {
                    self.window_dropped(h);
                }
            }
            Event::MouseOver(h) => {
                self.mouse_over(h);
                return Flow::Continue;
            }
            Event::DisplayChanged => self.refresh_monitors(),
            Event::Command(cmd) | Event::Request { command: cmd, .. } => {
                match cmd.parse::<Command>() {
                    Ok(c) => return self.execute(c),
                    Err(e) => warn!(command = %cmd, error = %e, "invalid command"),
                }
            }
            Event::Verify => self.verify_sizes(),
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
        let before = (self.focused(), self.focused_monitor);
        let flow = self.execute_inner(cmd);
        if flow == Flow::Continue && (self.focused(), self.focused_monitor) != before {
            self.warp_cursor();
        }
        flow
    }

    fn execute_inner(&mut self, cmd: Command) -> Flow {
        match cmd {
            Command::Focus(Target::Last) => self.focus_last(),
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
            Command::SwapWorkspaces(dir) => self.swap_workspaces(dir),
            Command::WorkspaceMode(mode) => {
                let mode = mode.unwrap_or(self.settings.workspace_mode.toggled());
                info!(%mode, "workspace mode");
                self.settings.workspace_mode = mode;
                self.align_workspaces();
            }
            Command::MoveToWorkspace { name, follow } => self.move_to_workspace(&name, follow),
            Command::FocusMonitor(dir) => self.focus_monitor(dir),
            Command::MoveToMonitor(dir) => {
                if let (Some(h), Some(m)) = (self.focused(), self.monitor_towards(dir)) {
                    if let Some(loc) = self.locate(h) {
                        self.transfer(h, loc, m);
                    }
                }
            }
            Command::Layout(layout) => self.current_ws_mut().layout = layout,
            Command::CycleLayout(step) => {
                let cycle = self.settings.layout_cycle.clone();
                let ws = self.current_ws_mut();
                ws.layout = ws.layout.cycled(&cycle, step);
            }
            Command::FlipLayout(axis) => {
                let ws = self.current_ws_mut();
                match axis {
                    Axis::Horizontal => ws.flip_h = !ws.flip_h,
                    Axis::Vertical => ws.flip_v = !ws.flip_v,
                }
            }
            Command::ToggleSplit => {
                if let Some(h) = self.focused() {
                    let ws = self.current_ws_mut();
                    if ws.layout == LayoutKind::Dwindle && ws.index_of(h).is_some() {
                        let tiled = ws.tiled.clone();
                        // Make sure the tree holds what is on screen first.
                        for t in &tiled {
                            ws.tree.insert(*t, None, false);
                        }
                        ws.tree.toggle_split(h);
                    }
                }
            }
            Command::LayoutReset => {
                let ws = self.current_ws_mut();
                ws.ratios.reset();
                ws.tree.reset();
            }
            Command::Resize(delta) => self.resize(delta),
            Command::ResizeEdge(dir, px) => self.resize_edge(dir, px),
            Command::ToggleFloat => self.toggle_float(),
            Command::ToggleMaximize => {
                if let Some(h) = self.focused() {
                    let mi = self.focused_monitor;
                    let ws = self.current_ws_mut();
                    let was = ws.maximized == Some(h);
                    ws.maximized = if was { None } else { Some(h) };
                    let floating = ws.is_floating(h);
                    self.applied.remove(&h);
                    if was && floating {
                        self.center_floating(h, mi);
                    }
                }
            }
            Command::TogglePin => self.toggle_pin(),
            Command::Center => {
                if let Some(h) = self.focused() {
                    if self.ws(self.current_loc()).is_floating(h) {
                        let mi = self.focused_monitor;
                        let size = self.platform.window_info(h).map(|w| (w.rect.w, w.rect.h));
                        if let Some(size) = size {
                            self.place_centred(h, mi, size);
                        }
                    }
                }
            }
            Command::Stack(dir) => self.stack(dir),
            Command::Unstack => {
                if let Some(h) = self.focused() {
                    self.current_ws_mut().unstack(h);
                }
            }
            Command::CycleStack(step) => {
                if let Some(h) = self.focused() {
                    if let Some(next) = self.current_ws_mut().cycle_stack(h, step) {
                        // Show and focus the new one before hiding the old one,
                        // or Windows hands focus to some other window.
                        self.reveal_slots();
                        self.focus_window(next);
                    }
                }
            }
            Command::StackAll => self.current_ws_mut().stack_all(),
            Command::UnstackAll => self.current_ws_mut().unstack_all(),
            Command::TogglePause => self.toggle_pause(),
            Command::Manage => {
                if let Some(h) = self.platform.foreground() {
                    if self.locate(h).is_none() {
                        self.ignored.remove(&h);
                        self.manage_window(h, true);
                    }
                }
            }
            Command::Unmanage => {
                if let Some(h) = self.focused() {
                    info!(handle = ?h, "unmanaging window");
                    self.unmanage(h);
                    self.ignored.insert(h);
                }
            }
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
            .flat_map(|ws| ws.all())
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
                maximized: ws.maximized == Some(h),
                pinned: self.pinned.contains(&h),
                stacked_hidden: ws.stack_of(h).is_some() && !ws.tiled.contains(&h),
            }
        };
        let workspace = |ws: &Workspace| WorkspaceState {
            name: ws.name.clone(),
            layout: ws.layout,
            focused: ws.focused.map(|h| h.0),
            windows: ws.all().map(|h| window(ws, h)).collect(),
            stacks: ws
                .stacks
                .iter()
                .map(|s| s.iter().map(|h| h.0).collect())
                .collect(),
        };
        State {
            focused_monitor: self.focused_monitor,
            focused_window: self.focused().map(|h| {
                let ws = self.ws(self.current_loc());
                window(ws, h)
            }),
            paused: self.paused,
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
            self.manage_window(h, false);
        }
    }

    /// Starts managing `h`. `force` (the `manage` command) skips the checks
    /// for whether it is a normal app window and `ignore` rules.
    fn manage_window(&mut self, h: WindowHandle, force: bool) {
        let Some(w) = self.platform.window_info(h) else {
            return;
        };
        if (!w.manageable && !force) || w.minimized || self.monitors.is_empty() {
            return;
        }
        let mut verdict = self.verdict_for(&w);
        if force {
            verdict.ignore = false;
        }
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
        if let Some((mw, mh)) = self.platform.min_size(h) {
            self.mins.insert(h, (mw, mh));
        }
        let monitor = verdict
            .monitor
            .filter(|m| *m < self.monitors.len())
            .unwrap_or(self.focused_monitor);
        if let Some(loc) = self.place(h, monitor, &verdict, pos) {
            self.meta.insert(h, Meta::from(&w));
            if self.is_visible(loc) {
                self.ws_mut(loc).focused = Some(h);
            }
            // Known not to fit even after the layout made room: float it now,
            // instead of tiling it and floating it once it refuses its slot.
            if !self.fits_its_slot(h, loc) {
                let size = self.mins.get(&h).copied().unwrap_or((w.rect.w, w.rect.h));
                info!(handle = ?h, exe = %w.exe, min = ?size, "window is too big for any slot, floating it");
                self.float_centred(h, loc, (size.0.max(w.rect.w), size.1.max(w.rect.h)));
            }
        }
    }

    /// Whether tiled window `h` gets at least its minimum size where the
    /// layout puts it. True for floating windows and windows without one.
    fn fits_its_slot(&self, h: WindowHandle, loc: Loc) -> bool {
        /// Pixels of slack before a window counts as not fitting.
        const TOLERANCE: i32 = 8;
        let Some(&(mw, mh)) = self.mins.get(&h) else {
            return true;
        };
        if self.force_tiled.contains(&h) || self.ws(loc).is_floating(h) {
            return true;
        }
        let Some((area, gaps)) = self.layout_area(loc) else {
            return true;
        };
        arrange(self.ws(loc), area, area, gaps, &self.mins)
            .into_iter()
            .find(|(x, _)| *x == h)
            .is_none_or(|(_, r)| r.w + TOLERANCE >= mw && r.h + TOLERANCE >= mh)
    }

    /// Floats `h` at `size` (at most the work area), centred on its monitor.
    fn float_centred(&mut self, h: WindowHandle, loc: Loc, size: (i32, i32)) {
        let ws = self.ws_mut(loc);
        ws.untile(h);
        if !ws.floating.contains(&h) {
            ws.floating.push(h);
        }
        self.applied.remove(&h);
        let mi = match loc {
            Loc::Ws { monitor, .. } => monitor,
            Loc::Special(_) => self.special_shown.map_or(self.focused_monitor, |(_, m)| m),
        };
        let area = self.monitors[mi].info.work_area;
        let (ww, wh) = (size.0.min(area.w), size.1.min(area.h));
        let rect = Rect::new(
            area.x + (area.w - ww) / 2,
            area.y + (area.h - wh) / 2,
            ww,
            wh,
        );
        if let Err(e) = self.platform.set_rects(&[(h, rect)]) {
            warn!(error = %e, "could not place floating window");
        }
    }

    /// Rules plus fixed-size detection: a window without a sizing border
    /// floats unless a `tile` rule says otherwise.
    fn verdict_for(&self, w: &WindowInfo) -> Verdict {
        let mut v = rules::evaluate(&self.settings.rules, w);
        if !w.resizable && !v.tile && !v.float {
            debug!(handle = ?w.handle, exe = %w.exe, "fixed-size window, floating it");
            v.float = true;
        }
        v
    }

    /// Floats tiled windows that did not take the size they were given (they
    /// enforce a minimum size or ignore resizing), unless a rule or the user
    /// insists they tile.
    fn verify_sizes(&mut self) {
        /// Pixels of slack before a window counts as refusing its slot.
        const TOLERANCE: i32 = 8;
        let mut refused = Vec::new();
        for h in self.managed().collect::<Vec<_>>() {
            let Some(slot) = self.applied.get(&h).copied() else {
                continue;
            };
            if self.force_tiled.contains(&h) {
                continue;
            }
            let Some(loc) = self.locate(h) else {
                continue;
            };
            if self.ws(loc).is_floating(h) || self.ws(loc).fullscreen == Some(h) {
                continue;
            }
            if let Some(w) = self.platform.window_info(h) {
                if w.rect.w > slot.w + TOLERANCE || w.rect.h > slot.h + TOLERANCE {
                    refused.push((h, loc, w));
                }
            }
        }
        for (h, loc, w) in refused {
            // Remember how big it insists on being, and first try to make
            // room for it by moving the splits (like a minimum size we knew
            // up front). Only if that can't work does it float.
            let slot = self.applied.get(&h).copied().unwrap_or(w.rect);
            let known = self.mins.get(&h).copied().unwrap_or((0, 0));
            let min = (
                if w.rect.w > slot.w + TOLERANCE {
                    w.rect.w
                } else {
                    known.0
                },
                if w.rect.h > slot.h + TOLERANCE {
                    w.rect.h
                } else {
                    known.1
                },
            );
            self.mins
                .insert(h, (min.0.max(known.0), min.1.max(known.1)));
            if self.fits_its_slot(h, loc) {
                info!(handle = ?h, exe = %w.exe, min = ?min, "window needs more room, re-tiling");
                continue;
            }
            info!(handle = ?h, exe = %w.exe, title = %w.title, ?slot, actual = ?w.rect,
                "window refused its tile size, floating it");
            self.float_centred(h, loc, (w.rect.w, w.rect.h));
        }
    }

    /// Focus follows mouse: focus `h` if it is a visible managed window and
    /// the user isn't busy with something else (a menu, the Start menu).
    fn mouse_over(&mut self, h: WindowHandle) {
        if !self.settings.focus_follows_mouse || self.moving.is_some() || self.focused() == Some(h)
        {
            return;
        }
        if !self.locate(h).is_some_and(|loc| self.is_visible(loc)) {
            return;
        }
        if let Some(fg) = self.platform.foreground() {
            let managed = self.locate(fg).is_some();
            let desktop = self
                .platform
                .window_info(fg)
                .is_some_and(|w| matches!(w.class.as_str(), "Progman" | "WorkerW"));
            if !managed && !desktop {
                return;
            }
        }
        self.focus_window(h);
    }

    /// Draws the border around the focused window, or hides it when no
    /// managed window has the foreground.
    fn update_border(&mut self) {
        let want = self.border_target();
        if want != self.border {
            self.border = want;
            self.platform.set_border(want);
        }
    }

    fn border_target(&self) -> Option<Border> {
        let b = self.settings.border;
        if !b.enabled || b.width <= 0 || self.monitors.is_empty() || self.paused {
            return None;
        }
        let h = self.focused()?;
        if self.moving == Some(h) || self.platform.foreground() != Some(h) {
            return None;
        }
        let loc = self.locate(h)?;
        if !self.is_visible(loc) || self.ws(loc).fullscreen == Some(h) {
            return None;
        }
        let rect = match self.applied.get(&h) {
            Some(r) if !self.ws(loc).is_floating(h) => *r,
            _ => self.platform.window_info(h)?.rect,
        };
        let mi = match loc {
            Loc::Ws { monitor, .. } => monitor,
            Loc::Special(_) => self.special_shown.map_or(self.focused_monitor, |(_, m)| m),
        };
        let scale = self.monitors.get(mi).map_or(1.0, |m| m.info.scale);
        Some(Border {
            window: h,
            rect,
            width: ((b.width as f32 * scale).round() as i32).max(1),
            color: b.color,
        })
    }

    /// Whether windows were moved since the last call, so the caller should
    /// send [`Event::Verify`] once they have had time to resize.
    pub fn take_verify_request(&mut self) -> bool {
        std::mem::take(&mut self.verify_pending)
    }

    /// The area and gaps a workspace is laid out in.
    fn layout_area(&self, loc: Loc) -> Option<(Rect, Gaps)> {
        let (mi, special) = match loc {
            Loc::Ws { monitor, .. } => (monitor, false),
            Loc::Special(_) => (self.special_shown?.1, true),
        };
        let m = self.monitors.get(mi)?;
        let gaps = self.gaps_for(self.ws(loc), m.info.scale);
        let area = if special {
            special_overlay(m.info.work_area)
        } else {
            m.info.work_area
        };
        Some((area, gaps))
    }

    /// The user resized tiled window `h` to `rect` with the mouse: change the
    /// layout ratios so its slot follows. Returns false if it could not.
    fn resize_to(&mut self, h: WindowHandle, loc: Loc, rect: Rect) -> bool {
        let Some((area, gaps)) = self.layout_area(loc) else {
            return false;
        };
        let mins = self.mins.clone();
        let ws = self.ws_mut(loc);
        if ws.fullscreen.is_some() || ws.maximized.is_some() {
            return false;
        }
        // Layouts work unflipped; mirror the wanted rect the same way.
        let rect = layout::flip(rect, area, ws.flip_h, ws.flip_v);
        let Some(index) = ws.index_of(h) else {
            return false;
        };
        let (layout, count) = (ws.layout, ws.tiled.len());
        if layout == LayoutKind::Dwindle {
            let tiled = ws.tiled.clone();
            return ws.tree.fit(h, &tiled, area, gaps, &mins, rect);
        }
        layout.fit(&mut ws.ratios, index, count, area, gaps, rect)
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
        if verdict.tile {
            self.force_tiled.insert(h);
        }
        if verdict.pin {
            self.pinned.insert(h);
        }
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
            self.force_tiled.remove(&h);
            self.mins.remove(&h);
            self.pinned.remove(&h);
            self.focus_history.retain(|x| *x != h);
            if self.opacity.remove(&h).is_some() {
                // Gone already if it was closed; that's fine.
                let _ = self.platform.set_opacity(h, 255);
            }
        }
    }

    /// The user finished dragging `h`. A tiled window dropped onto another
    /// tiled window swaps with it; dropped on another monitor it moves there;
    /// otherwise it snaps back into its slot.
    fn window_dropped(&mut self, h: WindowHandle) {
        /// A size change bigger than this means the user resized, not moved.
        const RESIZE_THRESHOLD: i32 = 10;
        let Some(loc) = self.locate(h) else {
            return;
        };
        let floating = self.ws(loc).is_floating(h);
        let slot = self.applied.remove(&h);
        if let (false, Some(slot), Some(w)) = (floating, slot, self.platform.window_info(h)) {
            let resized = (w.rect.w - slot.w).abs() > RESIZE_THRESHOLD
                || (w.rect.h - slot.h).abs() > RESIZE_THRESHOLD;
            if resized {
                if self.resize_to(h, loc, w.rect) {
                    debug!(handle = ?h, rect = ?w.rect, "resized by mouse");
                }
                return;
            }
        }
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
        if self.locate(h).is_some_and(|loc| self.is_visible(loc)) {
            self.focus_history.retain(|x| *x != h);
            self.focus_history.push(h);
            if self.focus_history.len() > FOCUS_HISTORY {
                self.focus_history.remove(0);
            }
        }
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
            for h in ws.all().collect::<Vec<_>>() {
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
                None => {
                    self.monitors
                        .push(Monitor::new(info, layout, &self.settings.workspaces));
                    self.apply_workspace_layouts(self.monitors.len() - 1);
                }
            }
        }
        // Windows on disconnected monitors move to the primary monitor's
        // workspace with the same position, keeping its visibility rules.
        for gone in old {
            for (wi, ws) in gone.workspaces.into_iter().enumerate() {
                let target = wi.min(self.monitors[0].workspaces.len() - 1);
                for h in ws.all().collect::<Vec<_>>() {
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
        self.align_workspaces();
        self.applied.clear();
    }

    /// In global mode, puts every monitor on the focused monitor's workspace.
    fn align_workspaces(&mut self) {
        if self.settings.workspace_mode != WorkspaceMode::Global || self.monitors.is_empty() {
            return;
        }
        let target = self.monitors[self.focused_monitor].active;
        let mut to_hide = Vec::new();
        for mi in 0..self.monitors.len() {
            to_hide.extend(self.set_active(mi, target));
        }
        for h in to_hide {
            self.hide(h);
        }
    }

    /// Shows workspace `target` on monitor `mi` and returns the windows of
    /// the one it replaces. The caller hides them after moving focus: hiding
    /// the foreground window first makes Windows hand focus to some other
    /// window, often one on another monitor.
    #[must_use]
    fn set_active(&mut self, mi: usize, target: usize) -> Vec<WindowHandle> {
        let mon = &self.monitors[mi];
        if target == mon.active || target >= mon.workspaces.len() {
            return Vec::new();
        }
        // Pinned windows come along to the new workspace.
        let pinned: Vec<_> = mon
            .workspace()
            .floating
            .iter()
            .copied()
            .filter(|h| self.pinned.contains(h))
            .collect();
        let mon = &mut self.monitors[mi];
        for &h in &pinned {
            let old = mon.workspace_mut();
            let was_focused = old.focused == Some(h);
            old.remove(h);
            let new = &mut mon.workspaces[target];
            new.add(h, true, InsertPos::End);
            if was_focused {
                new.focused = Some(h);
            }
        }
        let mon = &self.monitors[mi];
        let to_show: Vec<_> = mon.workspaces[target].windows().collect();
        let to_hide: Vec<_> = mon.workspace().windows().collect();
        // Show the new workspace before hiding the old one to avoid a flash
        // of desktop between the two.
        let mon = &mut self.monitors[mi];
        mon.last_active = Some(mon.active);
        mon.active = target;
        for h in to_show {
            self.show(h);
        }
        to_hide
    }

    /// Focuses `h`, or the desktop when there is nothing to focus, so that
    /// focus stays on this monitor instead of going wherever Windows picks.
    fn focus_or_desktop(&mut self, h: Option<WindowHandle>) {
        match h {
            Some(h) => self.focus_window(h),
            None => {
                self.desktop_focus_pending = true;
                if let Err(e) = self.platform.focus_desktop() {
                    self.desktop_focus_pending = false;
                    debug!(error = %e, "could not focus the desktop");
                }
            }
        }
    }

    fn focus_monitor_under_cursor(&mut self) {
        let Some((x, y)) = self.platform.cursor_pos() else {
            return;
        };
        if let Some(m) = self
            .monitors
            .iter()
            .position(|m| m.info.rect.contains_point(x, y))
        {
            self.focused_monitor = m;
            self.special_focused = false;
        }
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
            Target::Last => {
                let me = self.focused();
                self.focus_history.iter().rev().copied().find(|h| {
                    Some(*h) != me && self.locate(*h).is_some_and(|loc| self.is_visible(loc))
                })
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
            if let Target::Dir(dir) = target {
                self.nudge_floating(h, dir);
            }
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
        if ws.index_of(h).is_none() {
            return;
        }
        // In dwindle the first slot is the tree's first (biggest) window,
        // which needn't be first in the list.
        let order = if ws.layout == LayoutKind::Dwindle {
            ws.tree.leaves()
        } else {
            ws.tiled.clone()
        };
        let other = match order.iter().position(|x| *x == h) {
            Some(0) => order.get(1).copied(),
            Some(_) => order.first().copied(),
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
            if layout == LayoutKind::Dwindle {
                ws.tree.resize(ws.tiled[i], delta);
            } else {
                layout.resize(&mut ws.ratios, i, count, delta);
            }
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
            // The user wants it tiled: don't float it again if it looks fixed-size.
            self.force_tiled.insert(h);
        } else {
            ws.untile(h);
            ws.floating.push(h);
            self.force_tiled.remove(&h);
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

    /// Resolves a workspace of monitor `mi` by name or number, or one of
    /// `next`, `prev`, `last` (shown before the active one) and `empty`.
    fn workspace_index(&self, mi: usize, name: &str) -> Option<usize> {
        let mon = self.monitors.get(mi)?;
        if let Some(i) = mon.find_workspace(name) {
            return Some(i);
        }
        let n = mon.workspaces.len();
        match name {
            "next" => Some((mon.active + 1) % n),
            "prev" | "previous" => Some((mon.active + n - 1) % n),
            "last" | "back" => mon.last_active.filter(|i| *i < n),
            "empty" => mon.workspaces.iter().position(|w| w.is_empty()),
            _ => None,
        }
    }

    fn switch_workspace(&mut self, name: &str) {
        let mi = self.focused_monitor;
        let Some(mut target) = self.workspace_index(mi, name) else {
            warn!(workspace = name, "no such workspace");
            return;
        };
        if target == self.monitors[mi].active && self.settings.workspace_back_and_forth {
            if let Some(last) = self.monitors[mi].last_active {
                target = last;
            }
        }
        self.switch_to(mi, target);
    }

    /// Shows workspace `target` on monitor `mi` (every monitor in global
    /// mode) and focuses its focused window.
    fn switch_to(&mut self, mi: usize, target: usize) {
        self.focused_monitor = mi;
        self.special_focused = false;
        if target == self.monitors[mi].active {
            return;
        }
        let to_hide = match self.settings.workspace_mode {
            WorkspaceMode::PerMonitor => self.set_active(mi, target),
            WorkspaceMode::Global => (0..self.monitors.len())
                .flat_map(|m| self.set_active(m, target))
                .collect(),
        };
        let focus = self.monitors[mi].workspace().focused.or_else(|| {
            // Global mode: if this monitor's half is empty, focus another's.
            (self.settings.workspace_mode == WorkspaceMode::Global)
                .then(|| self.monitors.iter().find_map(|m| m.workspace().focused))
                .flatten()
        });
        self.focus_or_desktop(focus);
        for h in to_hide {
            self.hide(h);
        }
    }

    fn move_to_workspace(&mut self, name: &str, follow: bool) {
        let Some(h) = self.focused() else {
            return;
        };
        let mi = self.focused_monitor;
        let to = match self.workspace_index(mi, name) {
            Some(wi) if !name.starts_with(SPECIAL_PREFIX) => Some(Loc::Ws {
                monitor: mi,
                workspace: wi,
            }),
            _ => self.resolve(mi, name),
        };
        let (Some(from), Some(to)) = (self.locate(h), to) else {
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
        if let (true, false, Loc::Ws { monitor, workspace }) = (follow, self.is_visible(to), to) {
            self.switch_to(monitor, workspace);
            return;
        }
        if self.is_visible(to) {
            // Still on screen (e.g. out of a shown special workspace): keep it focused.
            self.show(h);
            self.focus_window(h);
        } else {
            let next = self.ws(from).focused;
            self.focus_or_desktop(next);
            self.hide(h);
        }
    }

    fn toggle_special(&mut self, name: &str) {
        let idx = self.special(name);
        let shown = self.special_shown;
        if let Some((current, _)) = shown {
            let to_hide: Vec<_> = self.specials[current].windows().collect();
            self.special_shown = None;
            self.special_focused = false;
            if current == idx {
                let back = self.monitors[self.focused_monitor].workspace().focused;
                self.focus_or_desktop(back);
            }
            for h in to_hide {
                self.hide(h);
            }
            if current == idx {
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

    /// `focus last`: the window focused before this one, switching to its
    /// workspace if it isn't shown.
    fn focus_last(&mut self) {
        let me = self.focused();
        let Some((h, loc)) = self
            .focus_history
            .iter()
            .rev()
            .filter(|h| Some(**h) != me)
            .find_map(|h| {
                let loc = self.locate(*h)?;
                let ws = self.ws(loc);
                (ws.tiled.contains(h) || ws.floating.contains(h)).then_some((*h, loc))
            })
        else {
            return;
        };
        if let (false, Loc::Ws { monitor, workspace }) = (self.is_visible(loc), loc) {
            self.ws_mut(loc).focused = Some(h);
            self.switch_to(monitor, workspace);
        }
        if self.is_visible(loc) {
            self.focus_window(h);
        }
    }

    /// `mouse_follows_focus`: puts the cursor in the middle of the focused
    /// window (or the focused monitor, if it has none) unless it is there.
    fn warp_cursor(&mut self) {
        if !self.settings.mouse_follows_focus {
            return;
        }
        let Some(target) = self.origin() else {
            return;
        };
        if let Some((x, y)) = self.platform.cursor_pos() {
            if target.contains_point(x, y) {
                return;
            }
        }
        let (x, y) = target.center();
        if let Err(e) = self.platform.set_cursor_pos(x, y) {
            debug!(error = %e, "could not move the cursor");
        }
    }

    /// The pixel step for keyboard moves and resizes on monitor `mi`.
    fn pixels(&self, mi: usize, px: i32) -> i32 {
        let scale = self.monitors.get(mi).map_or(1.0, |m| m.info.scale);
        ((px as f32 * scale).round() as i32).max(1)
    }

    /// Moves floating window `h` a step towards `dir`.
    fn nudge_floating(&mut self, h: WindowHandle, dir: Direction) {
        let Some(r) = self.platform.window_info(h).map(|w| w.rect) else {
            return;
        };
        let step = self.pixels(self.focused_monitor, DEFAULT_PIXEL_STEP);
        let (dx, dy) = match dir {
            Direction::Left => (-step, 0),
            Direction::Right => (step, 0),
            Direction::Up => (0, -step),
            Direction::Down => (0, step),
        };
        let rect = Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        if let Err(e) = self.platform.set_rects(&[(h, rect)]) {
            warn!(error = %e, "could not move floating window");
        }
    }

    /// `resize <dir> <px>`: a floating window grows on that side. A tiled
    /// window moves its edge that way: the edge facing `dir` if it has a
    /// neighbour there, else the opposite edge (so it shrinks), like dragging
    /// the divider between two tiles.
    fn resize_edge(&mut self, dir: Direction, px: i32) {
        let Some(h) = self.focused() else {
            return;
        };
        let Some(loc) = self.locate(h) else {
            return;
        };
        let px = self.pixels(self.focused_monitor, px);
        if self.ws(loc).is_floating(h) {
            let Some(r) = self.platform.window_info(h).map(|w| w.rect) else {
                return;
            };
            let rect = match dir {
                Direction::Left => Rect::new(r.x - px, r.y, r.w + px, r.h),
                Direction::Right => Rect::new(r.x, r.y, r.w + px, r.h),
                Direction::Up => Rect::new(r.x, r.y - px, r.w, r.h + px),
                Direction::Down => Rect::new(r.x, r.y, r.w, r.h + px),
            };
            if let Err(e) = self.platform.set_rects(&[(h, rect)]) {
                warn!(error = %e, "could not resize floating window");
            }
            return;
        }
        let (Some(slot), Some((area, gaps))) =
            (self.applied.get(&h).copied(), self.layout_area(loc))
        else {
            return;
        };
        let inner = area.inset(gaps.outer);
        // An edge within a gap's reach of the work area's edge has no neighbour.
        let reach = gaps.inner.max(2);
        let r = slot;
        let target = match dir {
            Direction::Right if r.right() + reach < inner.right() => {
                Rect::new(r.x, r.y, r.w + px, r.h)
            }
            Direction::Right => Rect::new(r.x + px, r.y, r.w - px, r.h),
            Direction::Left if r.x - reach > inner.x => Rect::new(r.x - px, r.y, r.w + px, r.h),
            Direction::Left => Rect::new(r.x, r.y, r.w - px, r.h),
            Direction::Down if r.bottom() + reach < inner.bottom() => {
                Rect::new(r.x, r.y, r.w, r.h + px)
            }
            Direction::Down => Rect::new(r.x, r.y + px, r.w, r.h - px),
            Direction::Up if r.y - reach > inner.y => Rect::new(r.x, r.y - px, r.w, r.h + px),
            Direction::Up => Rect::new(r.x, r.y, r.w, r.h - px),
        };
        if target.w > 0 && target.h > 0 {
            self.resize_to(h, loc, target);
        }
    }

    /// `swap-workspaces <dir>`: exchanges the windows of the focused
    /// monitor's active workspace with those of the monitor in `dir`. Focus
    /// goes with the focused window.
    fn swap_workspaces(&mut self, dir: Direction) {
        let a = self.focused_monitor;
        let Some(b) = self.monitor_towards(dir) else {
            return;
        };
        let (wa, wb) = (self.monitors[a].active, self.monitors[b].active);
        let (ra, rb) = (
            self.monitors[a].info.work_area,
            self.monitors[b].info.work_area,
        );
        let blank = || Workspace::new("", LayoutKind::default());
        let mut ws_a = std::mem::replace(&mut self.monitors[a].workspaces[wa], blank());
        let mut ws_b = std::mem::replace(&mut self.monitors[b].workspaces[wb], blank());
        // Names stay with their place on each monitor.
        std::mem::swap(&mut ws_a.name, &mut ws_b.name);
        // Floating windows keep their spot relative to the work area.
        let shift = |h: WindowHandle, from: Rect, to: Rect| {
            let r = self.platform.window_info(h)?.rect;
            let (w, hh) = (r.w.min(to.w), r.h.min(to.h));
            let x = (r.x - from.x + to.x).clamp(to.x, to.right() - w);
            let y = (r.y - from.y + to.y).clamp(to.y, to.bottom() - hh);
            Some((h, Rect::new(x, y, w, hh)))
        };
        let mut moves: Vec<_> = ws_a
            .floating
            .iter()
            .filter_map(|h| shift(*h, ra, rb))
            .collect();
        moves.extend(ws_b.floating.iter().filter_map(|h| shift(*h, rb, ra)));
        for h in ws_a.all().chain(ws_b.all()) {
            self.applied.remove(&h);
        }
        self.monitors[a].workspaces[wa] = ws_b;
        self.monitors[b].workspaces[wb] = ws_a;
        self.focused_monitor = b;
        self.special_focused = false;
        if !moves.is_empty() {
            if let Err(e) = self.platform.set_rects(&moves) {
                warn!(error = %e, "could not move floating windows");
            }
        }
    }

    /// `toggle-pin`: pins the focused window (floating it first) or unpins it.
    fn toggle_pin(&mut self) {
        let Some(h) = self.focused() else {
            return;
        };
        if self.pinned.remove(&h) {
            return;
        }
        if !self.ws(self.current_loc()).is_floating(h) {
            self.toggle_float();
        }
        self.pinned.insert(h);
    }

    /// Places `h` at `size` (at most the work area) in the middle of monitor `mi`.
    fn place_centred(&mut self, h: WindowHandle, mi: usize, size: (i32, i32)) {
        let area = self.monitors[mi].info.work_area;
        let (w, h_) = (size.0.min(area.w), size.1.min(area.h));
        let rect = Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h_) / 2, w, h_);
        if let Err(e) = self.platform.set_rects(&[(h, rect)]) {
            warn!(error = %e, "could not centre window");
        }
    }

    /// `stack <dir>`: moves the focused window into the tile of its
    /// neighbour in `dir`.
    fn stack(&mut self, dir: Direction) {
        let Some(h) = self.focused() else {
            return;
        };
        let loc = self.current_loc();
        if self.ws(loc).index_of(h).is_none() {
            return;
        }
        let Some(other) = self.neighbour(Target::Dir(dir)) else {
            return;
        };
        if self.locate(other) == Some(loc) && self.ws_mut(loc).stack_onto(h, other) {
            self.applied.remove(&h);
        }
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        info!(paused = self.paused, "tiling paused");
        if self.paused {
            return;
        }
        // Pick up windows that opened while paused.
        if let Ok(windows) = self.platform.windows() {
            for w in windows {
                if w.manageable && !w.minimized {
                    self.maybe_manage(w.handle);
                }
            }
        }
        self.applied.clear();
    }

    /// Shows windows that hold a visible tile but are hidden, e.g. the next
    /// window of a stack after the shown one closed or moved away.
    fn reveal_slots(&mut self) {
        let special = self.special_shown.map(|(i, _)| &self.specials[i]);
        let hidden: Vec<_> = self
            .monitors
            .iter()
            .map(|m| m.workspace())
            .chain(special)
            .flat_map(|ws| ws.tiled.iter().copied())
            .filter(|h| self.hidden_by_us.contains(h))
            .collect();
        for h in hidden {
            self.applied.remove(&h);
            self.show(h);
        }
    }

    /// Hides windows behind the shown member of their stack on visible
    /// workspaces.
    fn hide_stacked(&mut self) {
        let special = self.special_shown.map(|(i, _)| &self.specials[i]);
        let shown: Vec<_> = self
            .monitors
            .iter()
            .map(|m| m.workspace())
            .chain(special)
            .flat_map(|ws| ws.stacked_hidden())
            .filter(|h| !self.hidden_by_us.contains(h))
            .collect();
        for h in shown {
            self.hide(h);
        }
    }

    /// Makes unfocused windows translucent per `inactive_opacity`, and
    /// restores windows that should be opaque.
    fn update_opacity(&mut self) {
        let inactive = (self.settings.inactive_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        if inactive == 255 && self.opacity.is_empty() {
            return;
        }
        let focused = self.focused();
        let mut want: HashMap<WindowHandle, u8> = HashMap::new();
        if inactive < 255 && !self.paused {
            for h in self.managed() {
                if Some(h) != focused {
                    want.insert(h, inactive);
                }
            }
        }
        let current: Vec<_> = self.opacity.keys().copied().collect();
        for h in current {
            if !want.contains_key(&h) {
                self.opacity.remove(&h);
                if let Err(e) = self.platform.set_opacity(h, 255) {
                    debug!(handle = ?h, error = %e, "could not restore opacity");
                }
            }
        }
        for (h, alpha) in want {
            if self.opacity.get(&h) != Some(&alpha) {
                match self.platform.set_opacity(h, alpha) {
                    Ok(()) => {
                        self.opacity.insert(h, alpha);
                    }
                    Err(e) => debug!(handle = ?h, error = %e, "could not set opacity"),
                }
            }
        }
    }

    // ---- applying to the OS -------------------------------------------------

    /// Computes every visible workspace's layout and moves the windows whose
    /// slot changed since the last apply.
    pub fn apply(&mut self) {
        if self.paused {
            return;
        }
        // Show the new front of a stack before hiding the old one.
        self.reveal_slots();
        let mut slots = Vec::new();
        for m in &self.monitors {
            let gaps = self.gaps_for(m.workspace(), m.info.scale);
            slots.extend(arrange(
                m.workspace(),
                m.info.work_area,
                m.info.rect,
                gaps,
                &self.mins,
            ));
        }
        if let Some((i, mi)) = self.special_shown {
            if let Some(m) = self.monitors.get(mi) {
                let overlay = special_overlay(m.info.work_area);
                let gaps = self.gaps_for(&self.specials[i], m.info.scale);
                slots.extend(arrange(
                    &self.specials[i],
                    overlay,
                    overlay,
                    gaps,
                    &self.mins,
                ));
            }
        }
        let moves: Vec<_> = slots
            .into_iter()
            .filter(|(h, r)| self.applied.get(h) != Some(r))
            .collect();
        if !moves.is_empty() {
            for &(h, r) in &moves {
                self.applied.insert(h, r);
            }
            self.verify_pending = true;
            if let Err(e) = self.platform.set_rects(&moves) {
                warn!(error = %e, "some windows could not be moved");
            }
        }
        self.hide_stacked();
    }

    /// Hides a window and remembers that we did, so [`Wm::restore`] can undo it.
    pub fn hide(&mut self, handle: WindowHandle) {
        self.hidden_by_us.insert(handle);
        match self.platform.hide(handle) {
            Ok(()) => {
                self.hide_events_pending.insert(handle);
            }
            Err(e) => {
                warn!(?handle, error = %e, "hide failed");
                self.hidden_by_us.remove(&handle);
            }
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
        for (handle, _) in std::mem::take(&mut self.opacity) {
            if let Err(e) = self.platform.set_opacity(handle, 255) {
                debug!(?handle, error = %e, "could not restore opacity");
            }
        }
        for handle in std::mem::take(&mut self.hidden_by_us) {
            if let Err(e) = self.platform.show(handle) {
                warn!(?handle, error = %e, "restore failed");
            }
        }
    }
}

/// Where a shown special workspace is laid out: the middle of the work area.
fn special_overlay(area: Rect) -> Rect {
    let w = (area.w as f32 * SPECIAL_SCALE) as i32;
    let h = (area.h as f32 * SPECIAL_SCALE) as i32;
    Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h)
}

/// The target rect of every tiled (and fullscreen) window of a workspace.
fn arrange(
    ws: &Workspace,
    area: Rect,
    full: Rect,
    gaps: Gaps,
    mins: &Mins,
) -> Vec<(WindowHandle, Rect)> {
    let fullscreen = ws.fullscreen.filter(|h| ws.contains(*h));
    let maximized = ws
        .maximized
        .filter(|h| ws.windows().any(|x| x == *h) && Some(*h) != fullscreen);
    let tiled: Vec<_> = ws
        .tiled
        .iter()
        .copied()
        .filter(|h| Some(*h) != fullscreen && Some(*h) != maximized)
        .collect();
    let mut out = if ws.layout == LayoutKind::Dwindle {
        ws.tree.arrange(&tiled, area, gaps, mins)
    } else {
        let rects = ws.layout.arrange(tiled.len(), area, gaps, &ws.ratios);
        tiled.into_iter().zip(rects).collect()
    };
    if ws.flip_h || ws.flip_v {
        for (_, r) in &mut out {
            *r = layout::flip(*r, area, ws.flip_h, ws.flip_v);
        }
    }
    out.extend(maximized.map(|h| (h, area.inset(gaps.outer))));
    out.extend(fullscreen.map(|h| (h, full)));
    out
}
