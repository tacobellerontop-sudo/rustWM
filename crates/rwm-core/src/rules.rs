//! Window rules: match a window by exe, class or title and decide how it is
//! managed.

use regex::{Regex, RegexBuilder};

use crate::WindowInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    /// Manage the window as floating instead of tiling it.
    Float,
    /// Never manage the window.
    Ignore,
    /// Always tile the window, even if it looks fixed-size.
    Tile,
    /// Open the window on the named workspace of its monitor.
    Workspace(String),
    /// Open the window on this monitor (1-based, in the order Windows lists
    /// them, primary first).
    Monitor(usize),
    /// Float the window and keep it on screen on every workspace.
    Pin,
    /// Float the window at this size (logical pixels), centred.
    Size(i32, i32),
}

#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    #[error("a rule must match on at least one of exe, class or title")]
    NoMatcher,
    #[error("invalid {field} pattern: {source}")]
    Pattern {
        field: &'static str,
        #[source]
        source: regex::Error,
    },
    #[error(
        "unknown rule action `{0}` (expected float, tile, ignore, pin, size <w>x<h>, workspace <name> or monitor <n>)"
    )]
    Action(String),
}

/// A rule's match: every field given must match.
///
/// `exe` and `class` must match the whole value, case-insensitively, so
/// `code.exe` does not match `vscode.exe`. `title` is searched anywhere in the
/// title (anchor it with `^...$` for an exact match).
#[derive(Debug, Clone)]
pub struct Rule {
    pub exe: Option<Regex>,
    pub class: Option<Regex>,
    pub title: Option<Regex>,
    pub actions: Vec<RuleAction>,
}

fn whole(field: &'static str, pattern: &str) -> Result<Regex, RuleError> {
    RegexBuilder::new(&format!("^(?:{pattern})$"))
        .case_insensitive(true)
        .build()
        .map_err(|source| RuleError::Pattern { field, source })
}

impl RuleAction {
    pub fn parse(s: &str) -> Result<Self, RuleError> {
        let s = s.trim();
        match s.split_once(char::is_whitespace) {
            None if s == "float" => Ok(Self::Float),
            None if s == "ignore" => Ok(Self::Ignore),
            None if s == "tile" => Ok(Self::Tile),
            None if s == "pin" => Ok(Self::Pin),
            Some(("monitor", n)) => n
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .map(Self::Monitor)
                .ok_or_else(|| RuleError::Action(s.into())),
            Some(("size", size)) => size
                .trim()
                .split_once('x')
                .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                .filter(|&(w, h): &(i32, i32)| w > 0 && h > 0)
                .map(|(w, h)| Self::Size(w, h))
                .ok_or_else(|| RuleError::Action(s.into())),
            Some(("workspace", name)) if !name.trim().is_empty() => {
                Ok(Self::Workspace(name.trim().into()))
            }
            _ => Err(RuleError::Action(s.into())),
        }
    }
}

impl Rule {
    pub fn new(
        exe: Option<&str>,
        class: Option<&str>,
        title: Option<&str>,
        actions: Vec<RuleAction>,
    ) -> Result<Self, RuleError> {
        if exe.is_none() && class.is_none() && title.is_none() {
            return Err(RuleError::NoMatcher);
        }
        Ok(Self {
            exe: exe.map(|p| whole("exe", p)).transpose()?,
            class: class.map(|p| whole("class", p)).transpose()?,
            title: title
                .map(|p| {
                    Regex::new(p).map_err(|source| RuleError::Pattern {
                        field: "title",
                        source,
                    })
                })
                .transpose()?,
            actions,
        })
    }

    pub fn matches(&self, w: &WindowInfo) -> bool {
        let ok = |re: &Option<Regex>, value: &str| re.as_ref().is_none_or(|re| re.is_match(value));
        ok(&self.exe, &w.exe) && ok(&self.class, &w.class) && ok(&self.title, &w.title)
    }
}

/// What all matching rules together say about a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub ignore: bool,
    pub float: bool,
    pub tile: bool,
    pub pin: bool,
    pub workspace: Option<String>,
    /// 0-based monitor index.
    pub monitor: Option<usize>,
    /// Size to float the window at, in logical pixels.
    pub size: Option<(i32, i32)>,
}

pub fn evaluate(rules: &[Rule], w: &WindowInfo) -> Verdict {
    let mut v = Verdict::default();
    for rule in rules.iter().filter(|r| r.matches(w)) {
        for action in &rule.actions {
            match action {
                RuleAction::Float => v.float = true,
                RuleAction::Ignore => v.ignore = true,
                RuleAction::Tile => v.tile = true,
                RuleAction::Workspace(name) => v.workspace = Some(name.clone()),
                RuleAction::Monitor(n) => v.monitor = Some(n - 1),
                RuleAction::Pin => {
                    v.pin = true;
                    v.float = true;
                }
                RuleAction::Size(w, h) => {
                    v.size = Some((*w, *h));
                    v.float = true;
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, WindowHandle};

    fn win(exe: &str, class: &str, title: &str) -> WindowInfo {
        WindowInfo {
            handle: WindowHandle(1),
            title: title.into(),
            class: class.into(),
            exe: exe.into(),
            rect: Rect::default(),
            minimized: false,
            manageable: true,
            resizable: true,
        }
    }

    #[test]
    fn exe_matches_whole_name_case_insensitively() {
        let r = Rule::new(Some("code.exe"), None, None, vec![]).unwrap();
        assert!(r.matches(&win("Code.exe", "", "")));
        assert!(!r.matches(&win("vscode.exe", "", "")));
    }

    #[test]
    fn title_is_a_search() {
        let r = Rule::new(None, None, Some("Picture-in-Picture"), vec![]).unwrap();
        assert!(r.matches(&win("firefox.exe", "", "Picture-in-Picture")));
        let exact = Rule::new(None, None, Some("^PiP$"), vec![]).unwrap();
        assert!(!exact.matches(&win("", "", "PiP mode")));
    }

    #[test]
    fn all_fields_must_match() {
        let r = Rule::new(
            Some("Spotify.exe"),
            Some("Chrome_WidgetWin_1"),
            None,
            vec![],
        )
        .unwrap();
        assert!(r.matches(&win("Spotify.exe", "Chrome_WidgetWin_1", "Spotify")));
        assert!(!r.matches(&win("chrome.exe", "Chrome_WidgetWin_1", "Spotify")));
    }

    #[test]
    fn evaluate_combines_actions() {
        let rules = vec![
            Rule::new(Some("calc.exe"), None, None, vec![RuleAction::Float]).unwrap(),
            Rule::new(
                None,
                None,
                Some("calc"),
                vec![RuleAction::Workspace("3".into())],
            )
            .unwrap(),
        ];
        let v = evaluate(&rules, &win("calc.exe", "", "calc"));
        assert_eq!(
            v,
            Verdict {
                float: true,
                workspace: Some("3".into()),
                ..Verdict::default()
            }
        );
        assert_eq!(evaluate(&rules, &win("x.exe", "", "y")), Verdict::default());
    }

    #[test]
    fn parse_actions_and_errors() {
        assert_eq!(RuleAction::parse("float").unwrap(), RuleAction::Float);
        assert_eq!(
            RuleAction::parse("workspace  2").unwrap(),
            RuleAction::Workspace("2".into())
        );
        assert!(RuleAction::parse("workspace").is_err());
        assert_eq!(
            RuleAction::parse("monitor 2").unwrap(),
            RuleAction::Monitor(2)
        );
        assert_eq!(RuleAction::parse("pin").unwrap(), RuleAction::Pin);
        assert!(RuleAction::parse("monitor 0").is_err());
        assert!(RuleAction::parse("monitor x").is_err());
        assert!(RuleAction::parse("explode").is_err());
        assert_eq!(
            RuleAction::parse("size 800x600").unwrap(),
            RuleAction::Size(800, 600)
        );
        assert!(RuleAction::parse("size 800").is_err());
        assert!(RuleAction::parse("size 0x600").is_err());
        assert!(matches!(
            Rule::new(None, None, None, vec![]),
            Err(RuleError::NoMatcher)
        ));
        assert!(matches!(
            Rule::new(Some("("), None, None, vec![]),
            Err(RuleError::Pattern { field: "exe", .. })
        ));
    }
}
