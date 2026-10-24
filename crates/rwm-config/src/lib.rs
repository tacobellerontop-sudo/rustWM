//! rustWM configuration: the TOML schema, validation, defaults, and conversion
//! into the core's [`Settings`].
//!
//! Key bindings are not here: they live in kerys's own config.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rwm_core::layout::{Gaps, LayoutKind};
use rwm_core::rules::{Rule, RuleAction};
use rwm_core::wm::{BorderSettings, Settings, WorkspaceOverride};
use rwm_core::workspace::{InsertPos, WorkspaceMode};
use serde::Deserialize;

/// The commented config written on first run.
pub const DEFAULT_CONFIG: &str = include_str!("default.toml");

/// The hotkey daemon rustWM starts unless `general.hotkeys` says otherwise.
pub const DEFAULT_HOTKEYS: &str = "kerys.exe";

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub gaps: GapsConfig,
    pub border: BorderConfig,
    /// Per-workspace overrides: `[workspace.<name>]`.
    pub workspace: BTreeMap<String, WorkspaceConfig>,
    pub rules: Vec<RuleConfig>,
    /// Key bindings from before they moved to kerys. Accepted so old configs
    /// still load (kerys copies them on its first run), but unused.
    pub bindings: Option<toml::Value>,
    pub modes: Option<toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub default_layout: LayoutKind,
    pub new_window: InsertPos,
    pub workspaces: Vec<String>,
    pub workspace_mode: WorkspaceMode,
    pub focus_follows_mouse: bool,
    pub mouse_follows_focus: bool,
    pub workspace_back_and_forth: bool,
    /// Layouts `cycle-layout` goes through.
    pub layout_cycle: Vec<LayoutKind>,
    pub inactive_opacity: f32,
    /// Command line launched by the `terminal` command. `None` falls back to
    /// `wt.exe`, then `cmd.exe`.
    pub terminal: Option<String>,
    /// Hotkey daemon started with rustWM and stopped when it quits. Empty to
    /// start none.
    pub hotkeys: String,
    pub exec_on_start: Vec<String>,
}

impl Default for General {
    fn default() -> Self {
        let s = Settings::default();
        Self {
            default_layout: s.default_layout,
            new_window: s.new_window,
            workspaces: s.workspaces,
            workspace_mode: s.workspace_mode,
            focus_follows_mouse: s.focus_follows_mouse,
            mouse_follows_focus: s.mouse_follows_focus,
            workspace_back_and_forth: s.workspace_back_and_forth,
            layout_cycle: s.layout_cycle,
            inactive_opacity: s.inactive_opacity,
            terminal: None,
            hotkeys: DEFAULT_HOTKEYS.into(),
            exec_on_start: Vec::new(),
        }
    }
}

/// `[workspace.<name>]`: settings for one workspace.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub layout: Option<LayoutKind>,
    pub gaps_inner: Option<i32>,
    pub gaps_outer: Option<i32>,
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
#[serde(default, deny_unknown_fields)]
pub struct BorderConfig {
    pub enabled: bool,
    pub width: i32,
    /// `#rrggbb`.
    pub color: String,
}

impl Default for BorderConfig {
    fn default() -> Self {
        let b = BorderSettings::default();
        Self {
            enabled: b.enabled,
            width: b.width,
            color: format!("#{:06x}", b.color),
        }
    }
}

/// Parses `#rrggbb` (the `#` is optional) into 0xRRGGBB.
fn parse_color(s: &str) -> Option<u32> {
    let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
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
    /// Command line of the hotkey daemon to start, if any.
    pub hotkeys: Option<String>,
    /// The file still has `[bindings]` or `[modes]`, which rustWM ignores now
    /// that kerys handles keys.
    pub has_old_bindings: bool,
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
        if self.general.layout_cycle.is_empty() {
            errors.push("general.layout_cycle: needs at least one layout".into());
        }
        let opacity = self.general.inactive_opacity;
        if !(0.1..=1.0).contains(&opacity) {
            errors.push(format!(
                "general.inactive_opacity: must be 0.1 to 1.0, got {opacity}"
            ));
        }
        if self
            .general
            .exec_on_start
            .iter()
            .any(|c| c.trim().is_empty())
        {
            errors.push("general.exec_on_start: entries must not be empty".into());
        }
        let mut workspace_overrides = HashMap::new();
        for (name, w) in &self.workspace {
            if !self.general.workspaces.contains(name) {
                errors.push(format!(
                    "workspace.{name}: no workspace called `{name}` in general.workspaces"
                ));
            }
            if w.gaps_inner.is_some_and(|g| g < 0) || w.gaps_outer.is_some_and(|g| g < 0) {
                errors.push(format!("workspace.{name}: gaps must not be negative"));
            }
            let gaps = (w.gaps_inner.is_some() || w.gaps_outer.is_some()).then(|| Gaps {
                inner: w.gaps_inner.unwrap_or(self.gaps.inner),
                outer: w.gaps_outer.unwrap_or(self.gaps.outer),
                smart: self.gaps.smart,
            });
            workspace_overrides.insert(
                name.clone(),
                WorkspaceOverride {
                    layout: w.layout,
                    gaps,
                },
            );
        }
        if !(0..=50).contains(&self.border.width) {
            errors.push(format!(
                "border.width: must be 0 to 50, got {}",
                self.border.width
            ));
        }
        let border_color = parse_color(&self.border.color).unwrap_or_else(|| {
            errors.push(format!(
                "border.color: `{}` is not a #rrggbb colour",
                self.border.color
            ));
            BorderSettings::default().color
        });

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
                workspace_mode: self.general.workspace_mode,
                focus_follows_mouse: self.general.focus_follows_mouse,
                mouse_follows_focus: self.general.mouse_follows_focus,
                workspace_back_and_forth: self.general.workspace_back_and_forth,
                layout_cycle: self.general.layout_cycle,
                inactive_opacity: opacity,
                exec_on_start: self
                    .general
                    .exec_on_start
                    .iter()
                    .map(|c| c.trim().to_string())
                    .collect(),
                workspace_overrides,
                border: BorderSettings {
                    enabled: self.border.enabled,
                    width: self.border.width,
                    color: border_color,
                },
                terminal: self.general.terminal.map(|t| t.trim().to_string()),
                rules,
            },
            hotkeys: Some(self.general.hotkeys.trim().to_string()).filter(|h| !h.is_empty()),
            has_old_bindings: self.bindings.is_some() || self.modes.is_some(),
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
        assert_eq!(loaded.hotkeys.as_deref(), Some(DEFAULT_HOTKEYS));
        assert!(!loaded.has_old_bindings);
    }

    #[test]
    fn empty_config_uses_defaults() {
        let loaded = parse("").unwrap();
        assert_eq!(loaded.settings.default_layout, LayoutKind::Dwindle);
        assert_eq!(loaded.settings.terminal, None);
        assert_eq!(loaded.settings.gaps, Gaps::default());
        assert_eq!(loaded.hotkeys.as_deref(), Some(DEFAULT_HOTKEYS));
    }

    #[test]
    fn full_example() {
        let loaded = parse(
            r#"
            [general]
            default_layout = "master"
            new_window = "end"
            workspaces = ["web", "code"]
            workspace_mode = "global"
            focus_follows_mouse = true
            terminal = "wezterm-gui.exe"
            hotkeys = ""

            [border]
            width = 5
            color = '#FF8800'

            [gaps]
            inner = 4
            outer = 0
            smart = false

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
        assert_eq!(s.workspace_mode, WorkspaceMode::Global);
        assert!(s.focus_follows_mouse);
        assert_eq!(
            s.border,
            BorderSettings {
                enabled: true,
                width: 5,
                color: 0xff8800
            }
        );
        assert!(parse("[border]\ncolor = \"blue\"").is_err());
        assert!(parse("[border]\nwidth = -1").is_err());
        assert!(parse("[general]\nworkspace_mode = \"both\"").is_err());
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
        assert_eq!(loaded.hotkeys, None);
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
            "explode",
            "at least one of",
        ] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }
    }

    #[test]
    fn old_bindings_are_accepted_and_ignored() {
        let loaded = parse(
            r#"
            [bindings]
            "win+h" = "focus left"
            "win+m" = "mode move"
            [modes.move]
            "escape" = "mode default"
            "#,
        )
        .unwrap();
        assert!(loaded.has_old_bindings);
        assert!(
            parse("[general]\nhotkeys = \"whkd.exe\"").unwrap().hotkeys == Some("whkd.exe".into())
        );
    }

    #[test]
    fn komorebi_and_hyprland_style_settings() {
        let loaded = parse(
            r#"
            [general]
            workspaces = ["web", "code"]
            mouse_follows_focus = true
            workspace_back_and_forth = true
            layout_cycle = ["dwindle", "center", "grid"]
            inactive_opacity = 0.85
            exec_on_start = ["yasb.exe"]

            [workspace.code]
            layout = "columns"
            gaps_inner = 0

            [[rules]]
            match = { exe = "mpv.exe" }
            actions = ["pin", "monitor 2"]
            "#,
        )
        .unwrap();
        let s = &loaded.settings;
        assert!(s.mouse_follows_focus && s.workspace_back_and_forth);
        assert_eq!(
            s.layout_cycle,
            vec![LayoutKind::Dwindle, LayoutKind::Center, LayoutKind::Grid]
        );
        assert_eq!(s.inactive_opacity, 0.85);
        assert_eq!(s.exec_on_start, vec!["yasb.exe"]);
        let code = &s.workspace_overrides["code"];
        assert_eq!(code.layout, Some(LayoutKind::Columns));
        assert_eq!(code.gaps.map(|g| (g.inner, g.outer)), Some((0, 8)));
        assert_eq!(
            s.rules[0].actions,
            vec![RuleAction::Pin, RuleAction::Monitor(2)]
        );

        let errors = invalid(
            r#"
            [general]
            layout_cycle = []
            inactive_opacity = 0.0
            [workspace.nope]
            layout = "grid"
            "#,
        )
        .join("\n");
        for needle in ["layout_cycle", "inactive_opacity", "workspace.nope"] {
            assert!(errors.contains(needle), "missing `{needle}` in:\n{errors}");
        }
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
