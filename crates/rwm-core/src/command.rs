//! The command language of the CLI and IPC, and so of hotkey daemons like
//! kerys, which send these strings over the pipe.
//!
//! For example `"focus left"`, `"move right"` or `"exec wezterm-gui.exe"`.

use std::fmt;
use std::str::FromStr;

use crate::geometry::Direction;
use crate::layout::{LayoutKind, Side};
use crate::workspace::WorkspaceMode;

/// The special workspace `toggle-special` uses when no name is given.
pub const DEFAULT_SPECIAL: &str = "scratch";

/// Prefix that names a special workspace in `move-to-workspace` and rules,
/// e.g. `special:scratch`.
pub const SPECIAL_PREFIX: &str = "special:";

/// How much `resize grow`/`resize shrink` changes a ratio by default.
pub const DEFAULT_RESIZE_STEP: f32 = 0.05;

/// How far `resize <dir>` moves an edge, and `move <dir>` nudges a floating
/// window, by default: logical pixels at 100% scale.
pub const DEFAULT_PIXEL_STEP: i32 = 50;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `focus left|right|up|down|next|prev|last`
    Focus(Target),
    /// `move left|right|up|down|next|prev`: swap with that neighbour. A
    /// floating window is nudged that way instead.
    Move(Target),
    /// Swap the focused window into the first (largest) slot.
    Promote,
    /// Ask the focused window to close, like clicking its X.
    Close,
    /// Launch the configured terminal.
    Terminal,
    /// `exec <command line>`
    Exec(String),
    /// Recompute and reapply every layout.
    Retile,
    /// `workspace <name|number>`: switch the focused monitor (or, in global
    /// mode, every monitor) to a workspace. Besides names it takes `next`,
    /// `prev`, `last` (the one shown before) and `empty` (the first empty one).
    Workspace(String),
    /// `workspace-mode global|per-monitor|toggle`; `None` toggles.
    WorkspaceMode(Option<WorkspaceMode>),
    /// `move-to-workspace <name|number> [follow]`: send the focused window
    /// there, and with `follow` switch to that workspace too.
    MoveToWorkspace { name: String, follow: bool },
    /// `swap-workspaces <dir>`: exchange the active workspace's windows with
    /// those of the monitor in that direction.
    SwapWorkspaces(Direction),
    /// `focus-monitor <dir>`
    FocusMonitor(Direction),
    /// `move-to-monitor <dir>`
    MoveToMonitor(Direction),
    /// `layout dwindle|master|monocle|columns|rows|grid|center`
    Layout(LayoutKind),
    /// `cycle-layout [next|prev]`: switch the focused workspace to the next
    /// (or previous) layout of the configured cycle.
    CycleLayout(isize),
    /// `flip-layout horizontal|vertical`: mirror the workspace's layout
    /// along that axis (again to undo).
    FlipLayout(Axis),
    /// Dwindle: turn the split holding the focused window between
    /// side-by-side and stacked (Hyprland's `togglesplit`).
    ToggleSplit,
    /// Forget all resize ratios on the focused workspace.
    LayoutReset,
    /// `master-count +1|-1|<n>`: how many windows share the master area.
    MasterCount(Count),
    /// `master-side left|right|top|bottom|next`: where the master area is;
    /// `None` (`next`) goes round the sides.
    MasterSide(Option<Side>),
    /// `resize grow|shrink [amount]`: grow (positive) or shrink the focused
    /// window's share of its split.
    Resize(f32),
    /// `resize left|right|up|down [pixels]`: move the focused window's edge
    /// between it and its neighbour that way (a floating window's edge on
    /// that side grows outwards).
    ResizeEdge(Direction, i32),
    /// Float a tiled window or tile a floating one.
    ToggleFloat,
    /// Make the focused window cover its whole monitor, or back.
    ToggleFullscreen,
    /// Make the focused window fill the work area (gaps and taskbar stay),
    /// or back.
    ToggleMaximize,
    /// Keep the focused window on screen on every workspace of its monitor
    /// (floats it first).
    TogglePin,
    /// Centre the focused floating window on its monitor.
    Center,
    /// Minimize the focused window (it stops being managed until restored).
    Minimize,
    /// `focus-window <handle|exe:regex|title:regex|class:regex>`: focus a
    /// window anywhere, switching workspace or stack tab to show it. With
    /// several matches, the next one after the focused window.
    FocusWindow(String),
    /// `stack <dir>`: put the focused window in the same tile as its
    /// neighbour; only one window of a stack shows at a time.
    Stack(Direction),
    /// Take the focused window out of its stack into a tile of its own.
    Unstack,
    /// `cycle-stack next|prev`: show the next window of the focused stack.
    CycleStack(isize),
    /// Stack every tiled window of the workspace into one tile.
    StackAll,
    /// Break up every stack of the workspace.
    UnstackAll,
    /// Stop (or resume) tiling and managing windows.
    TogglePause,
    /// Start managing the focused window even if rules or heuristics skip it.
    Manage,
    /// Stop managing the focused window; it is left where it is.
    Unmanage,
    /// Re-read the config file.
    Reload,
    /// `toggle-special [name]`: show or hide a special (scratchpad) workspace
    /// over the focused monitor. The name defaults to `scratch`.
    ToggleSpecial(String),
    /// Show every hidden window without quitting.
    Restore,
    /// Restore all windows and exit.
    Quit,
}

/// A new value for a count: relative or absolute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    By(i32),
    To(usize),
}

impl Count {
    /// Applies the change to `n`, staying at least 1.
    pub fn apply(self, n: usize) -> usize {
        match self {
            Count::By(d) => (n as i64 + d as i64).max(1) as usize,
            Count::To(v) => v.max(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Dir(Direction),
    Next,
    Prev,
    /// The window focused before the current one (Alt+Tab style).
    Last,
}

/// An axis of the screen, for `flip-layout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Mirror left and right.
    Horizontal,
    /// Mirror top and bottom.
    Vertical,
}

impl Axis {
    pub fn name(self) -> &'static str {
        match self {
            Axis::Horizontal => "horizontal",
            Axis::Vertical => "vertical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("empty command")]
    Empty,
    #[error("unknown command `{0}`")]
    Unknown(String),
    #[error("`{cmd}` expects {expected}, got `{got}`")]
    BadArgument {
        cmd: &'static str,
        expected: &'static str,
        got: String,
    },
}

fn target(cmd: &'static str, arg: &str) -> Result<Target, ParseError> {
    match arg {
        "last" if cmd == "focus" => Ok(Target::Last),
        "next" => Ok(Target::Next),
        "prev" | "previous" => Ok(Target::Prev),
        other => other
            .parse()
            .map(Target::Dir)
            .map_err(|()| ParseError::BadArgument {
                cmd,
                expected: "left, right, up, down, next or prev",
                got: arg.into(),
            }),
    }
}

fn direction(cmd: &'static str, arg: &str) -> Result<Direction, ParseError> {
    arg.parse().map_err(|()| ParseError::BadArgument {
        cmd,
        expected: "left, right, up or down",
        got: arg.into(),
    })
}

fn name_arg(cmd: &'static str, arg: &str) -> Result<String, ParseError> {
    if arg.is_empty() || arg.contains(char::is_whitespace) {
        Err(ParseError::BadArgument {
            cmd,
            expected: "a workspace name or number",
            got: arg.into(),
        })
    } else {
        Ok(arg.into())
    }
}

fn step(cmd: &'static str, rest: &str) -> Result<isize, ParseError> {
    match rest {
        "" | "next" => Ok(1),
        "prev" | "previous" => Ok(-1),
        _ => Err(ParseError::BadArgument {
            cmd,
            expected: "next or prev",
            got: rest.into(),
        }),
    }
}

fn resize(rest: &str) -> Result<Command, ParseError> {
    let bad = || ParseError::BadArgument {
        cmd: "resize",
        expected: "grow or shrink with an optional amount like 0.1, \
                   or a direction with optional pixels like `left 50`",
        got: rest.into(),
    };
    let mut parts = rest.split_whitespace();
    let first = parts.next().unwrap_or_default();
    if let Ok(dir) = first.parse::<Direction>() {
        let px = match parts.next() {
            None => DEFAULT_PIXEL_STEP,
            Some(p) => p
                .parse::<i32>()
                .ok()
                .filter(|p| (1..=10_000).contains(p))
                .ok_or_else(bad)?,
        };
        if parts.next().is_some() {
            return Err(bad());
        }
        return Ok(Command::ResizeEdge(dir, px));
    }
    let sign = match first {
        "grow" => 1.0,
        "shrink" => -1.0,
        _ => return Err(bad()),
    };
    let amount = match parts.next() {
        None => DEFAULT_RESIZE_STEP,
        Some(a) => a
            .parse::<f32>()
            .ok()
            .filter(|a| *a > 0.0 && *a < 1.0)
            .ok_or_else(bad)?,
    };
    if parts.next().is_some() {
        return Err(bad());
    }
    Ok(Command::Resize(sign * amount))
}

fn count(cmd: &'static str, rest: &str) -> Result<Count, ParseError> {
    let parsed = if rest.starts_with(['+', '-']) {
        rest.parse::<i32>().ok().filter(|d| *d != 0).map(Count::By)
    } else {
        rest.parse::<usize>()
            .ok()
            .filter(|n| *n >= 1)
            .map(Count::To)
    };
    parsed.ok_or_else(|| ParseError::BadArgument {
        cmd,
        expected: "+N, -N or a number of at least 1",
        got: rest.into(),
    })
}

/// Checks a `focus-window` query: a window handle, or `exe:`, `title:` or
/// `class:` followed by a pattern.
fn window_query(rest: &str) -> Result<String, ParseError> {
    let ok = rest.parse::<isize>().is_ok()
        || ["exe:", "title:", "class:"]
            .iter()
            .any(|p| rest.strip_prefix(p).is_some_and(|re| !re.is_empty()));
    if ok {
        Ok(rest.into())
    } else {
        Err(ParseError::BadArgument {
            cmd: "focus-window",
            expected: "a window handle, exe:<pattern>, title:<pattern> or class:<pattern>",
            got: rest.into(),
        })
    }
}

fn no_arg(cmd: &'static str, rest: &str, c: Command) -> Result<Command, ParseError> {
    if rest.is_empty() {
        Ok(c)
    } else {
        Err(ParseError::BadArgument {
            cmd,
            expected: "no argument",
            got: rest.into(),
        })
    }
}

impl FromStr for Command {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (name, rest) = s.split_once(char::is_whitespace).unwrap_or((s, ""));
        let rest = rest.trim();
        match name {
            "" => Err(ParseError::Empty),
            "focus" => target("focus", rest).map(Command::Focus),
            "move" => target("move", rest).map(Command::Move),
            "promote" => no_arg("promote", rest, Command::Promote),
            "close" => no_arg("close", rest, Command::Close),
            "terminal" => no_arg("terminal", rest, Command::Terminal),
            "retile" => no_arg("retile", rest, Command::Retile),
            "restore" => no_arg("restore", rest, Command::Restore),
            "quit" => no_arg("quit", rest, Command::Quit),
            "workspace" => name_arg("workspace", rest).map(Command::Workspace),
            "workspace-mode" if rest == "toggle" => Ok(Command::WorkspaceMode(None)),
            "workspace-mode" => rest
                .parse()
                .map(|m| Command::WorkspaceMode(Some(m)))
                .map_err(|()| ParseError::BadArgument {
                    cmd: "workspace-mode",
                    expected: "global, per-monitor or toggle",
                    got: rest.into(),
                }),
            "move-to-workspace" => {
                let (name, follow) = match rest.rsplit_once(char::is_whitespace) {
                    Some((name, "follow")) => (name.trim(), true),
                    _ => (rest, false),
                };
                name_arg("move-to-workspace", name)
                    .map(|name| Command::MoveToWorkspace { name, follow })
            }
            "swap-workspaces" => direction("swap-workspaces", rest).map(Command::SwapWorkspaces),
            "focus-monitor" => direction("focus-monitor", rest).map(Command::FocusMonitor),
            "move-to-monitor" => direction("move-to-monitor", rest).map(Command::MoveToMonitor),
            "layout" if rest == "reset" => Ok(Command::LayoutReset),
            "layout" => rest
                .parse()
                .map(Command::Layout)
                .map_err(|()| ParseError::BadArgument {
                    cmd: "layout",
                    expected: "dwindle, master, monocle or reset",
                    got: rest.into(),
                }),
            "cycle-layout" => step("cycle-layout", rest).map(Command::CycleLayout),
            "flip-layout" => match rest {
                "horizontal" => Ok(Command::FlipLayout(Axis::Horizontal)),
                "vertical" => Ok(Command::FlipLayout(Axis::Vertical)),
                _ => Err(ParseError::BadArgument {
                    cmd: "flip-layout",
                    expected: "horizontal or vertical",
                    got: rest.into(),
                }),
            },
            "master-count" => count("master-count", rest).map(Command::MasterCount),
            "master-side" if rest == "next" => Ok(Command::MasterSide(None)),
            "master-side" => rest
                .parse()
                .map(|s| Command::MasterSide(Some(s)))
                .map_err(|()| ParseError::BadArgument {
                    cmd: "master-side",
                    expected: "left, right, top, bottom or next",
                    got: rest.into(),
                }),
            "minimize" => no_arg("minimize", rest, Command::Minimize),
            "focus-window" => window_query(rest).map(Command::FocusWindow),
            "toggle-split" => no_arg("toggle-split", rest, Command::ToggleSplit),
            "resize" => resize(rest),
            "toggle-float" => no_arg("toggle-float", rest, Command::ToggleFloat),
            "toggle-fullscreen" => no_arg("toggle-fullscreen", rest, Command::ToggleFullscreen),
            "toggle-maximize" => no_arg("toggle-maximize", rest, Command::ToggleMaximize),
            "toggle-pin" => no_arg("toggle-pin", rest, Command::TogglePin),
            "center" => no_arg("center", rest, Command::Center),
            "stack" => direction("stack", rest).map(Command::Stack),
            "unstack" => no_arg("unstack", rest, Command::Unstack),
            "cycle-stack" => step("cycle-stack", rest).map(Command::CycleStack),
            "stack-all" => no_arg("stack-all", rest, Command::StackAll),
            "unstack-all" => no_arg("unstack-all", rest, Command::UnstackAll),
            "toggle-pause" => no_arg("toggle-pause", rest, Command::TogglePause),
            "manage" => no_arg("manage", rest, Command::Manage),
            "unmanage" => no_arg("unmanage", rest, Command::Unmanage),
            "reload" => no_arg("reload", rest, Command::Reload),
            "toggle-special" if rest.is_empty() => {
                Ok(Command::ToggleSpecial(DEFAULT_SPECIAL.into()))
            }
            "toggle-special" => name_arg("toggle-special", rest).map(Command::ToggleSpecial),
            "exec" if rest.is_empty() => Err(ParseError::BadArgument {
                cmd: "exec",
                expected: "a command line",
                got: String::new(),
            }),
            "exec" => Ok(Command::Exec(rest.into())),
            other => Err(ParseError::Unknown(other.into())),
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Dir(d) => d.fmt(f),
            Target::Next => f.write_str("next"),
            Target::Prev => f.write_str("prev"),
            Target::Last => f.write_str("last"),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Focus(t) => write!(f, "focus {t}"),
            Command::Move(t) => write!(f, "move {t}"),
            Command::Promote => f.write_str("promote"),
            Command::Close => f.write_str("close"),
            Command::Terminal => f.write_str("terminal"),
            Command::Exec(c) => write!(f, "exec {c}"),
            Command::Retile => f.write_str("retile"),
            Command::Workspace(n) => write!(f, "workspace {n}"),
            Command::WorkspaceMode(Some(m)) => write!(f, "workspace-mode {m}"),
            Command::WorkspaceMode(None) => f.write_str("workspace-mode toggle"),
            Command::MoveToWorkspace {
                name,
                follow: false,
            } => write!(f, "move-to-workspace {name}"),
            Command::MoveToWorkspace { name, follow: true } => {
                write!(f, "move-to-workspace {name} follow")
            }
            Command::SwapWorkspaces(d) => write!(f, "swap-workspaces {d}"),
            Command::FocusMonitor(d) => write!(f, "focus-monitor {d}"),
            Command::MoveToMonitor(d) => write!(f, "move-to-monitor {d}"),
            Command::Layout(l) => write!(f, "layout {l}"),
            Command::CycleLayout(s) if *s < 0 => f.write_str("cycle-layout prev"),
            Command::CycleLayout(_) => f.write_str("cycle-layout"),
            Command::FlipLayout(a) => write!(f, "flip-layout {}", a.name()),
            Command::ToggleSplit => f.write_str("toggle-split"),
            Command::MasterCount(Count::By(d)) => write!(f, "master-count {d:+}"),
            Command::MasterCount(Count::To(n)) => write!(f, "master-count {n}"),
            Command::MasterSide(Some(s)) => write!(f, "master-side {s}"),
            Command::MasterSide(None) => f.write_str("master-side next"),
            Command::Minimize => f.write_str("minimize"),
            Command::FocusWindow(q) => write!(f, "focus-window {q}"),
            Command::LayoutReset => f.write_str("layout reset"),
            Command::Resize(d) if *d >= 0.0 => write!(f, "resize grow {d}"),
            Command::Resize(d) => write!(f, "resize shrink {}", -d),
            Command::ToggleFloat => f.write_str("toggle-float"),
            Command::ResizeEdge(d, px) => write!(f, "resize {d} {px}"),
            Command::ToggleFullscreen => f.write_str("toggle-fullscreen"),
            Command::ToggleMaximize => f.write_str("toggle-maximize"),
            Command::TogglePin => f.write_str("toggle-pin"),
            Command::Center => f.write_str("center"),
            Command::Stack(d) => write!(f, "stack {d}"),
            Command::Unstack => f.write_str("unstack"),
            Command::CycleStack(s) if *s < 0 => f.write_str("cycle-stack prev"),
            Command::CycleStack(_) => f.write_str("cycle-stack next"),
            Command::StackAll => f.write_str("stack-all"),
            Command::UnstackAll => f.write_str("unstack-all"),
            Command::TogglePause => f.write_str("toggle-pause"),
            Command::Manage => f.write_str("manage"),
            Command::Unmanage => f.write_str("unmanage"),
            Command::Reload => f.write_str("reload"),
            Command::ToggleSpecial(n) => write!(f, "toggle-special {n}"),
            Command::Restore => f.write_str("restore"),
            Command::Quit => f.write_str("quit"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Command {
        s.parse().unwrap()
    }

    #[test]
    fn parses_and_round_trips() {
        for s in [
            "focus left",
            "focus next",
            "move down",
            "move prev",
            "promote",
            "close",
            "terminal",
            "retile",
            "restore",
            "quit",
            "exec wezterm-gui.exe start --cwd ~",
            "workspace 3",
            "workspace web",
            "move-to-workspace 9",
            "workspace-mode global",
            "workspace-mode per-monitor",
            "workspace-mode toggle",
            "focus-monitor left",
            "move-to-monitor right",
            "layout master",
            "layout monocle",
            "layout reset",
            "cycle-layout",
            "resize grow 0.05",
            "resize shrink 0.1",
            "toggle-float",
            "toggle-fullscreen",
            "reload",
            "toggle-special music",
            "move-to-workspace special:scratch",
            "focus last",
            "workspace last",
            "move-to-workspace 4 follow",
            "swap-workspaces left",
            "cycle-layout prev",
            "layout columns",
            "layout rows",
            "layout grid",
            "layout center",
            "flip-layout horizontal",
            "flip-layout vertical",
            "toggle-split",
            "resize left 50",
            "resize down 120",
            "toggle-maximize",
            "toggle-pin",
            "center",
            "stack right",
            "unstack",
            "cycle-stack next",
            "cycle-stack prev",
            "stack-all",
            "unstack-all",
            "toggle-pause",
            "manage",
            "unmanage",
            "master-count +1",
            "master-count -1",
            "master-count 2",
            "master-side top",
            "master-side next",
            "minimize",
            "focus-window 1234",
            "focus-window exe:firefox\\.exe",
            "focus-window title:^Inbox",
        ] {
            assert_eq!(parse(s).to_string(), s);
        }
    }

    #[test]
    fn tolerates_extra_whitespace() {
        assert_eq!(
            parse("  focus   up "),
            Command::Focus(Target::Dir(Direction::Up))
        );
        assert_eq!(
            parse("exec  wt.exe -w new"),
            Command::Exec("wt.exe -w new".into())
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!("".parse::<Command>(), Err(ParseError::Empty));
        assert!(matches!(
            "fly away".parse::<Command>(),
            Err(ParseError::Unknown(_))
        ));
        for bad in [
            "focus sideways",
            "promote now",
            "exec",
            "workspace",
            "workspace a b",
            "layout spiral",
            "resize",
            "resize grow 2",
            "resize sideways",
            "focus-monitor next",
            "workspace-mode",
            "workspace-mode both",
            "move last",
            "flip-layout",
            "flip-layout diagonal",
            "resize left 0",
            "resize left x",
            "stack next",
            "cycle-stack sideways",
            "swap-workspaces",
            "master-count",
            "master-count 0",
            "master-count +0",
            "master-count many",
            "master-side middle",
            "focus-window",
            "focus-window firefox",
            "focus-window exe:",
        ] {
            assert!(
                matches!(bad.parse::<Command>(), Err(ParseError::BadArgument { .. })),
                "{bad}"
            );
        }
    }

    #[test]
    fn defaults_for_optional_arguments() {
        assert_eq!(
            parse("resize shrink"),
            Command::Resize(-DEFAULT_RESIZE_STEP)
        );
        assert_eq!(
            parse("toggle-special"),
            Command::ToggleSpecial("scratch".into())
        );
        assert_eq!(
            parse("resize right"),
            Command::ResizeEdge(Direction::Right, DEFAULT_PIXEL_STEP)
        );
        assert_eq!(parse("cycle-layout"), Command::CycleLayout(1));
        assert_eq!(parse("cycle-stack"), Command::CycleStack(1));
        assert_eq!(
            parse("move-to-workspace 3"),
            Command::MoveToWorkspace {
                name: "3".into(),
                follow: false
            }
        );
    }
}
