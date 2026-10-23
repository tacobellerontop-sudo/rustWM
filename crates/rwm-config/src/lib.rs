//! rustWM configuration: the TOML schema, validation, defaults, and conversion
//! into the core's [`Settings`] plus the key bindings.

pub mod keys;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rwm_core::Command;
use rwm_core::layout::{Gaps, LayoutKind};
use rwm_core::rules::{Rule, RuleAction};
use rwm_core::wm::{DEFAULT_MODE, Settings};
use rwm_core::workspace::InsertPos;
use serde::Deserialize;

pub use keys::{Chord, KeyError};

/// The commented config written on first run. Its `[bindings]` table is the
/// built-in key map.
pub const DEFAULT_CONFIG: &str = include_str!("default.toml");

/// A binding value that removes a default binding.
const UNBIND: &str = "none";

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub gaps: GapsConfig,
    pub bindings: BTreeMap<String, String>,
    /// Extra binding modes: mode name -> chord -> command.
    pub modes: BTreeMap<String, BTreeMap<String, String>>,
    pub rules: Vec<RuleConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub default_layout: LayoutKind,
    pub new_window: InsertPos,
    pub workspaces: Vec<String>,
    /// Command line launched by the `terminal` command. `None` falls back to
    /// `wt.exe`, then `cmd.exe`.
    pub terminal: Option<String>,
}

impl Default for General {
    fn default() -> Self {
        let s = Settings::default();
        Self {
            default_layout: s.default_layout,
            new_window: s.new_window,
            workspaces: s.workspaces,
            terminal: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GapsConfig {
    pub inner: i32,
    pub outer: i32,
    pub smart: bool,
}

impl Default for GapsConfig {
    fn default() -> Self {
        let g = Gaps::default();
        Self {
            inner: g.inner,
            outer: g.outer,
            smart: g.smart,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    #[serde(rename = "match")]
    pub matcher: MatchConfig,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchConfig {
    pub exe: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
}

/// A validated config, ready to hand to the window manager.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub settings: Settings,
    /// Bindings of the default mode.
    pub bindings: Vec<(Chord, String)>,
    /// Bindings of every other mode, by mode name.
    pub modes: Vec<(String, Vec<(Chord, String)>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// TOML syntax or type error; the message includes line and column.
    #[error("{0}")]
    Parse(String),
    #[error("invalid config:\n  {}", .0.join("\n  "))]
    Invalid(Vec<String>),
}

fn default_config() -> Config {
    toml::from_str(DEFAULT_CONFIG).expect("default config parses")
}

/// The built-in key map, as written in [`DEFAULT_CONFIG`].
pub fn default_bindings() -> Vec<(Chord, String)> {
    default_config()
        .bindings
        .iter()
        .map(|(k, c)| (k.parse().expect("default binding parses"), c.clone()))
        .collect()
}

/// Parses one table of bindings, reporting problems under `section`.
fn parse_bindings(
    section: &str,
    table: &BTreeMap<String, String>,
    into: &mut BTreeMap<Chord, String>,
    errors: &mut Vec<String>,
) {
    for (key, command) in table {
        let chord = match key.parse::<Chord>() {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("{section}.\"{key}\": {e}"));
                continue;
            }
        };
        if command.trim() == UNBIND {
            into.remove(&chord);
            continue;
        }
        match command.parse::<Command>() {
            Ok(_) => {
                into.insert(chord, command.trim().to_string());
            }
            Err(e) => errors.push(format!("{section}.\"{key}\": {e}")),
        }
    }
}

/// `%USERPROFILE%\.config\rustwm\rustwm.toml` (or `$HOME/...` elsewhere).
pub fn default_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config").join("rustwm").join("rustwm.toml")
}

/// Writes [`DEFAULT_CONFIG`] to `path` if nothing is there yet. Returns
/// whether it wrote the file.
pub fn ensure_default(path: &Path) -> std::io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, DEFAULT_CONFIG)?;
    Ok(true)
}

/// Reads, parses and validates the config at `path`.
pub fn load(path: &Path) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse(&text)
}

/// Parses and validates config text.
pub fn parse(text: &str) -> Result<Loaded, ConfigError> {
    let config: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
    config.validate()
}

impl Config {
    /// Checks everything serde cannot and builds the runtime settings. All
    /// problems are reported at once.
    pub fn validate(self) -> Result<Loaded, ConfigError> {
        let mut errors = Vec::new();

        let ws = &self.general.workspaces;
        if ws.is_empty() || ws.len() > 9 {
            errors.push(format!(
                "general.workspaces: need 1 to 9 names, got {}",
                ws.len()
            ));
        }
        for (i, name) in ws.iter().enumerate() {
            if name.is_empty() || name.contains(char::is_whitespace) {
                errors.push(format!("general.workspaces: `{name}` must be one word"));
            } else if ws[..i].contains(name) {
                errors.push(format!("general.workspaces: `{name}` appears twice"));
            }
        }
        if self
            .general
            .terminal
            .as_deref()
            .is_some_and(|t| t.trim().is_empty())
        {
            errors
                .push("general.terminal: must not be empty (remove it to use the default)".into());
        }
        if self.gaps.inner < 0 || self.gaps.outer < 0 {
            errors.push("gaps: inner and outer must not be negative".into());
        }

        let mut bindings: BTreeMap<Chord, String> = default_bindings().into_iter().collect();
        parse_bindings("bindings", &self.bindings, &mut bindings, &mut errors);

        // User modes replace built-in modes of the same name.
        let mut mode_tables = default_config().modes;
        mode_tables.extend(self.modes);
        let mut modes = Vec::new();
        for (name, table) in &mode_tables {
            let section = format!("modes.{name}");
            if name == DEFAULT_MODE {
                errors.push(format!("{section}: the default mode is [bindings]"));
                continue;
            }
            let mut map = BTreeMap::new();
            parse_bindings(&section, table, &mut map, &mut errors);
            if !map.is_empty() && !map.values().any(|c| c == &format!("mode {DEFAULT_MODE}")) {
                errors.push(format!(
                    "{section}: needs a binding to \"mode {DEFAULT_MODE}\""
                ));
            }
            modes.push((name.clone(), map.into_iter().collect::<Vec<_>>()));
        }
        // Every `mode <name>` must name a mode that exists.
        let all_commands = bindings
            .values()
            .chain(modes.iter().flat_map(|(_, b)| b.iter().map(|(_, c)| c)));
        for command in all_commands {
            if let Ok(Command::Mode(m)) = command.parse::<Command>() {
                if m != DEFAULT_MODE && !modes.iter().any(|(n, _)| *n == m) {
                    errors.push(format!("`{command}`: there is no [modes.{m}]"));
                }
            }
        }

        let mut rules = Vec::new();
        for (i, r) in self.rules.iter().enumerate() {
            let n = i + 1;
            let actions: Vec<RuleAction> = r
                .actions
                .iter()
                .filter_map(|a| match RuleAction::parse(a) {
                    Ok(a) => Some(a),
                    Err(e) => {
                        errors.push(format!("rules #{n}: {e}"));
                        None
                    }
                })
                .collect();
            if r.actions.is_empty() {
                errors.push(format!("rules #{n}: needs at least one action"));
            }
            let m = &r.matcher;
            match Rule::new(
                m.exe.as_deref(),
                m.class.as_deref(),
                m.title.as_deref(),
                actions,
            ) {
                Ok(rule) => rules.push(rule),
                Err(e) => errors.push(format!("rules #{n}: {e}")),
            }
        }

        if !errors.is_empty() {
            return Err(ConfigError::Invalid(errors));
        }
        Ok(Loaded {
            settings: Settings {
                gaps: Gaps {
                    inner: self.gaps.inner,
                    outer: self.gaps.outer,
                    smart: self.gaps.smart,
                },
                new_window: self.general.new_window,
                default_layout: self.general.default_layout,
                workspaces: self.general.workspaces,
                terminal: self.general.terminal.map(|t| t.trim().to_string()),
                rules,
            },
            bindings: bindings.into_iter().collect(),
            modes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(text: &str) -> Vec<String> {
        match parse(text) {
            Err(ConfigError::Invalid(e)) => e,
            other => panic!("expected validation errors, got {other:?}"),
        }
    }

    #[test]
    fn default_config_is_valid() {
        let loaded = parse(DEFAULT_CONFIG).unwrap();
        assert_eq!(loaded.settings.workspaces.len(), 9);
        assert_eq!(loaded.settings.rules.len(), 3);
        assert_eq!(loaded.bindings.len(), default_bindings().len());
    }

    #[test]
    fn default_bindings_are_valid_commands() {
        for (chord, cmd) in default_bindings() {
            cmd.parse::<Command>()
                .unwrap_or_else(|e| panic!("{chord}: {e}"));
        }
    }

    #[test]
    fn empty_config_uses_defaults() {
        let loaded = parse("").unwrap();
        assert_eq!(loaded.settings.default_layout, LayoutKind::Dwindle);
        assert_eq!(loaded.settings.terminal, None);
        assert_eq!(loaded.settings.gaps, Gaps::default());
        assert_eq!(loaded.bindings.len(), default_bindings().len());
    }

    #[test]
    fn full_example() {
        let loaded = parse(
            r#"
            [general]
            default_layout = "master"
            new_window = "end"
            workspaces = ["web", "code"]
            terminal = "wezterm-gui.exe"

            [gaps]
            inner = 4
            outer = 0
            smart = false

            [bindings]
            "win+t" = "exec wezterm-gui.exe start --cwd ~"
            "win+h" = "none"
            "win+enter" = "exec alacritty.exe"

            [[rules]]
            match = { exe = "Spotify.exe", class = "Chrome_WidgetWin_1" }
            actions = ["workspace code", "float"]
            "#,
        )
        .unwrap();
        let s = &loaded.settings;
        assert_eq!(s.default_layout, LayoutKind::Master);
        assert_eq!(s.new_window, InsertPos::End);
        assert_eq!(s.workspaces, vec!["web", "code"]);
        assert_eq!(s.terminal.as_deref(), Some("wezterm-gui.exe"));
        assert_eq!(
            s.gaps,
            Gaps {
                inner: 4,
                outer: 0,
                smart: false
            }
        );
        assert_eq!(
            s.rules[0].actions,
            vec![RuleAction::Workspace("code".into()), RuleAction::Float]
        );

        let find = |k: &str| {
            let chord: Chord = k.parse().unwrap();
            loaded
                .bindings
                .iter()
                .find(|(c, _)| *c == chord)
                .map(|(_, cmd)| cmd.as_str())
        };
        assert_eq!(find("win+t"), Some("exec wezterm-gui.exe start --cwd ~"));
        assert_eq!(find("win+h"), None);
        assert_eq!(find("win+enter"), Some("exec alacritty.exe"));
        assert_eq!(find("win+j"), Some("focus down"));
    }

    #[test]
    fn syntax_errors_report_line_and_column() {
        let Err(ConfigError::Parse(msg)) = parse("[general]\ndefault_layout = \"spiral\"\n") else {
            panic!("expected a parse error");
        };
        assert!(msg.contains("line 2"), "{msg}");

        let Err(ConfigError::Parse(msg)) = parse("[general]\nbogus = 1\n") else {
            panic!("unknown keys are errors");
        };
        assert!(msg.contains("bogus"), "{msg}");
    }

    #[test]
    fn semantic_errors_are_all_reported() {
        let errors = invalid(
            r#"
            [general]
            workspaces = ["a", "a", "b c"]
            terminal = " "

            [gaps]
            inner = -1

            [bindings]
            "win+banana" = "focus left"
            "win+x" = "fly away"

            [[rules]]
            match = {}
            actions = ["explode"]
            "#,
        );
        let all = errors.join("\n");
        for needle in [
            "twice",
            "one word",
            "terminal",
            "negative",
            "banana",
            "fly",
            "explode",
            "at least one of",
        ] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }
    }

    #[test]
    fn default_resize_mode() {
        let loaded = parse("").unwrap();
        let (name, binds) = &loaded.modes[0];
        assert_eq!(name, "resize");
        assert!(
            binds
                .iter()
                .any(|(c, cmd)| c.to_string() == "escape" && cmd == "mode default")
        );
    }

    #[test]
    fn custom_modes_are_validated() {
        let loaded = parse(
            r#"
            [bindings]
            "win+m" = "mode move"
            [modes.move]
            "h" = "move left"
            "escape" = "mode default"
            "#,
        )
        .unwrap();
        assert_eq!(loaded.modes.len(), 2);

        let errors = invalid(
            r#"
            [bindings]
            "win+m" = "mode nowhere"
            [modes.trap]
            "h" = "focus left"
            [modes.default]
            "x" = "close"
            "#,
        )
        .join("\n");
        assert!(errors.contains("no [modes.nowhere]"), "{errors}");
        assert!(errors.contains("modes.trap: needs a binding"), "{errors}");
        assert!(errors.contains("default mode is [bindings]"), "{errors}");
    }

    #[test]
    fn too_many_workspaces() {
        let names: Vec<_> = (1..=10).map(|i| format!("\"{i}\"")).collect();
        let errors = invalid(&format!("[general]\nworkspaces = [{}]", names.join(",")));
        assert!(errors[0].contains("1 to 9"));
    }

    #[test]
    fn ensure_default_writes_once() {
        let dir = std::env::temp_dir().join(format!("rwm-config-test-{}", std::process::id()));
        let path = dir.join("rustwm.toml");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ensure_default(&path).unwrap());
        assert!(!ensure_default(&path).unwrap());
        assert!(load(&path).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
