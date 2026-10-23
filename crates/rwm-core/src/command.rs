//! The command language shared by hotkey bindings, the CLI and IPC.
//!
//! Every binding in the config maps to one of these strings, e.g.
//! `"focus left"`, `"move right"` or `"exec wezterm-gui.exe"`.

use std::fmt;
use std::str::FromStr;

use crate::geometry::Direction;

#[derive(Debug, Clone, PartialEq, Eq)]
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
        assert!(matches!(
            "focus sideways".parse::<Command>(),
            Err(ParseError::BadArgument { .. })
        ));
        assert!(matches!(
            "promote now".parse::<Command>(),
            Err(ParseError::BadArgument { .. })
        ));
        assert!(matches!(
            "exec".parse::<Command>(),
            Err(ParseError::BadArgument { .. })
        ));
    }
}
