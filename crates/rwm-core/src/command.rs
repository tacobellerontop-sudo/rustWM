//! The command language of the CLI and IPC, and so of hotkey daemons like
//! kerys, which send these strings over the pipe.
//!
//! For example `"focus left"`, `"move right"` or `"exec wezterm-gui.exe"`.

use std::fmt;
use std::str::FromStr;

use crate::geometry::Direction;
use crate::layout::LayoutKind;
use crate::workspace::WorkspaceMode;

/// The special workspace `toggle-special` uses when no name is given.
pub const DEFAULT_SPECIAL: &str = "scratch";

/// Prefix that names a special workspace in `move-to-workspace` and rules,
/// e.g. `special:scratch`.
pub const SPECIAL_PREFIX: &str = "special:";

/// How much `resize grow`/`resize shrink` changes a ratio by default.
pub const DEFAULT_RESIZE_STEP: f32 = 0.05;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `focus left|right|up|down|next|prev`
    Focus(Target),
    /// `move left|right|up|down|next|prev`: swap with that neighbour.
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
    /// mode, every monitor) to a workspace.
    Workspace(String),
    /// `workspace-mode global|per-monitor|toggle`; `None` toggles.
    WorkspaceMode(Option<WorkspaceMode>),
    /// `move-to-workspace <name|number>`: send the focused window there.
    MoveToWorkspace(String),
    /// `focus-monitor <dir>`
    FocusMonitor(Direction),
    /// `move-to-monitor <dir>`
    MoveToMonitor(Direction),
    /// `layout dwindle|master|monocle`
    Layout(LayoutKind),
    /// Switch the focused workspace to the next layout.
    CycleLayout,
    /// Forget all resize ratios on the focused workspace.
    LayoutReset,
    /// `resize grow|shrink [amount]`: grow (positive) or shrink the focused
    /// window's share of its split.
    Resize(f32),
    /// Float a tiled window or tile a floating one.
    ToggleFloat,
    /// Make the focused window cover its whole monitor, or back.
    ToggleFullscreen,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Dir(Direction),
    Next,
    Prev,
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

fn resize(rest: &str) -> Result<Command, ParseError> {
    let bad = || ParseError::BadArgument {
        cmd: "resize",
        expected: "grow or shrink, optionally followed by an amount like 0.1",
        got: rest.into(),
    };
    let mut parts = rest.split_whitespace();
    let sign = match parts.next() {
        Some("grow") => 1.0,
        Some("shrink") => -1.0,
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
                name_arg("move-to-workspace", rest).map(Command::MoveToWorkspace)
            }
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
            "cycle-layout" => no_arg("cycle-layout", rest, Command::CycleLayout),
            "resize" => resize(rest),
            "toggle-float" => no_arg("toggle-float", rest, Command::ToggleFloat),
            "toggle-fullscreen" => no_arg("toggle-fullscreen", rest, Command::ToggleFullscreen),
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
            Command::MoveToWorkspace(n) => write!(f, "move-to-workspace {n}"),
            Command::FocusMonitor(d) => write!(f, "focus-monitor {d}"),
            Command::MoveToMonitor(d) => write!(f, "move-to-monitor {d}"),
            Command::Layout(l) => write!(f, "layout {l}"),
            Command::CycleLayout => f.write_str("cycle-layout"),
            Command::LayoutReset => f.write_str("layout reset"),
            Command::Resize(d) if *d >= 0.0 => write!(f, "resize grow {d}"),
            Command::Resize(d) => write!(f, "resize shrink {}", -d),
            Command::ToggleFloat => f.write_str("toggle-float"),
            Command::ToggleFullscreen => f.write_str("toggle-fullscreen"),
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
            "resize up",
            "focus-monitor next",
            "workspace-mode",
            "workspace-mode both",
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
    }
}
