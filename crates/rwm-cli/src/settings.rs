//! The menu's Settings screen: change common options without editing the
//! config by hand. Each change is written to rustwm.toml (keeping comments
//! and everything else as it was) and the running daemon reloads it.

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, bail};
use rwm_core::wm::Settings;
use toml_edit::{DocumentMut, value};

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// on/off; choosing it flips the value.
    Bool,
    /// One of these; choosing it moves to the next.
    Choice(&'static [&'static str]),
    /// A whole number in this range, typed in.
    Int(i64, i64),
    /// `#rrggbb`, typed in.
    Color,
    /// A command line, typed in; `-` removes it.
    Command,
}

#[derive(Debug, Clone, Copy)]
struct Setting {
    label: &'static str,
    section: &'static str,
    key: &'static str,
    kind: Kind,
}

const SETTINGS: &[Setting] = &[
    Setting {
        label: "Focus follows mouse",
        section: "general",
        key: "focus_follows_mouse",
        kind: Kind::Bool,
    },
    Setting {
        label: "Workspaces on several monitors",
        section: "general",
        key: "workspace_mode",
        kind: Kind::Choice(&["per-monitor", "global"]),
    },
    Setting {
        label: "Layout for new workspaces",
        section: "general",
        key: "default_layout",
        kind: Kind::Choice(&[
            "dwindle", "master", "monocle", "columns", "rows", "grid", "center",
        ]),
    },
    Setting {
        label: "Where new windows go",
        section: "general",
        key: "new_window",
        kind: Kind::Choice(&["after-focused", "end", "start"]),
    },
    Setting {
        label: "Mouse follows keyboard focus",
        section: "general",
        key: "mouse_follows_focus",
        kind: Kind::Bool,
    },
    Setting {
        label: "Workspace key twice goes back",
        section: "general",
        key: "workspace_back_and_forth",
        kind: Kind::Bool,
    },
    Setting {
        label: "Terminal (Win+Enter)",
        section: "general",
        key: "terminal",
        kind: Kind::Command,
    },
    Setting {
        label: "Focus border",
        section: "border",
        key: "enabled",
        kind: Kind::Bool,
    },
    Setting {
        label: "Border width",
        section: "border",
        key: "width",
        kind: Kind::Int(0, 50),
    },
    Setting {
        label: "Border colour",
        section: "border",
        key: "color",
        kind: Kind::Color,
    },
    Setting {
        label: "Gap between windows",
        section: "gaps",
        key: "inner",
        kind: Kind::Int(0, 200),
    },
    Setting {
        label: "Gap at screen edges",
        section: "gaps",
        key: "outer",
        kind: Kind::Int(0, 200),
    },
    Setting {
        label: "No gaps with a single window",
        section: "gaps",
        key: "smart",
        kind: Kind::Bool,
    },
];

/// The current value of `s` as shown in the menu.
fn current(s: &Setting, cfg: &Settings) -> String {
    let on_off = |b: bool| if b { "on" } else { "off" }.to_string();
    match (s.section, s.key) {
        ("general", "focus_follows_mouse") => on_off(cfg.focus_follows_mouse),
        ("general", "mouse_follows_focus") => on_off(cfg.mouse_follows_focus),
        ("general", "workspace_back_and_forth") => on_off(cfg.workspace_back_and_forth),
        ("general", "workspace_mode") => cfg.workspace_mode.to_string(),
        ("general", "default_layout") => cfg.default_layout.to_string(),
        ("general", "new_window") => match cfg.new_window {
            rwm_core::workspace::InsertPos::AfterFocused => "after-focused",
            rwm_core::workspace::InsertPos::End => "end",
            rwm_core::workspace::InsertPos::Start => "start",
        }
        .into(),
        ("general", "terminal") => cfg
            .terminal
            .clone()
            .unwrap_or_else(|| "default (wt.exe, then cmd.exe)".into()),
        ("border", "enabled") => on_off(cfg.border.enabled),
        ("border", "width") => cfg.border.width.to_string(),
        ("border", "color") => format!("#{:06x}", cfg.border.color),
        ("gaps", "inner") => cfg.gaps.inner.to_string(),
        ("gaps", "outer") => cfg.gaps.outer.to_string(),
        ("gaps", "smart") => on_off(cfg.gaps.smart),
        _ => String::new(),
    }
}

/// A new value for a setting: written as is, or removed to use the default.
#[derive(Debug, Clone, PartialEq)]
enum NewValue {
    Bool(bool),
    Int(i64),
    Str(String),
    Remove,
}

/// Returns `text` with `[section] key` changed, after checking the result is
/// a valid config. Comments and formatting elsewhere are kept.
fn set(text: &str, section: &str, key: &str, new: &NewValue) -> anyhow::Result<String> {
    let mut doc: DocumentMut = text.parse().context("the config is not valid TOML")?;
    if !doc.contains_key(section) {
        doc[section] = toml_edit::table();
    }
    let table = doc[section]
        .as_table_like_mut()
        .with_context(|| format!("[{section}] is not a table"))?;
    let item = match new {
        NewValue::Bool(b) => value(*b),
        NewValue::Int(i) => value(*i),
        NewValue::Str(s) => value(s.as_str()),
        NewValue::Remove => {
            table.remove(key);
            toml_edit::Item::None
        }
    };
    if !item.is_none() {
        // Replace just the value, so the comment above the key stays.
        match table.get_mut(key) {
            Some(existing) => {
                // Keep a trailing `# comment` on the same line too.
                let decor = existing.as_value().map(|v| v.decor().clone());
                *existing = item;
                if let (Some(d), Some(v)) = (decor, existing.as_value_mut()) {
                    *v.decor_mut() = d;
                }
            }
            None => {
                table.insert(key, item);
            }
        }
    }
    let out = doc.to_string();
    rwm_config::parse(&out)?;
    Ok(out)
}

/// Asks for a line; `None` if the user just pressed Enter or stdin closed.
fn prompt(stdin: &mut impl BufRead, question: &str) -> anyhow::Result<Option<String>> {
    print!("  {question}: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    stdin.read_line(&mut line)?;
    let line = line.trim();
    Ok((!line.is_empty()).then(|| line.to_string()))
}

/// Works out the new value for `s`, asking the user if it needs typing in.
/// `None` means leave it as it is.
fn new_value(
    s: &Setting,
    shown: &str,
    stdin: &mut impl BufRead,
) -> anyhow::Result<Option<NewValue>> {
    Ok(match s.kind {
        Kind::Bool => Some(NewValue::Bool(shown != "on")),
        Kind::Choice(options) => {
            let i = options.iter().position(|o| *o == shown).unwrap_or(0);
            Some(NewValue::Str(options[(i + 1) % options.len()].into()))
        }
        Kind::Int(min, max) => {
            match prompt(stdin, &format!("{} ({min}-{max}, Enter to keep)", s.label))? {
                None => None,
                Some(v) => {
                    let n: i64 = v
                        .parse()
                        .ok()
                        .filter(|n| (min..=max).contains(n))
                        .with_context(|| format!("`{v}` is not a number from {min} to {max}"))?;
                    Some(NewValue::Int(n))
                }
            }
        }
        Kind::Color => match prompt(stdin, "Colour as #rrggbb, e.g. #89b4fa (Enter to keep)")? {
            None => None,
            Some(v) => {
                let hex = v.trim_start_matches('#');
                if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    bail!("`{v}` is not a #rrggbb colour");
                }
                Some(NewValue::Str(format!("#{}", hex.to_lowercase())))
            }
        },
        Kind::Command => match prompt(
            stdin,
            "Command, e.g. wt.exe or alacritty.exe (- for the default, Enter to keep)",
        )? {
            None => None,
            Some(v) if v == "-" => Some(NewValue::Remove),
            Some(v) => Some(NewValue::Str(v)),
        },
    })
}

/// Shows the Settings screen until the user goes back.
pub fn run(config: &Path, stdin: &mut impl BufRead) -> anyhow::Result<()> {
    if rwm_config::ensure_default(config)? {
        println!("  Wrote the default config to {}", config.display());
    }
    let mut message = String::new();
    loop {
        super::menu::clear_screen();
        let text = std::fs::read_to_string(config)
            .with_context(|| format!("cannot read {}", config.display()))?;
        let loaded = rwm_config::parse(&text).with_context(|| {
            format!(
                "fix {} first (menu: Check config for errors)",
                config.display()
            )
        })?;
        println!();
        println!("  Settings");
        println!("  ----------------------------------------------");
        println!("  Saved to {}.", config.display());
        println!("  A running rustWM picks changes up straight away.");
        println!();
        let shown: Vec<String> = SETTINGS
            .iter()
            .map(|s| current(s, &loaded.settings))
            .collect();
        let width = SETTINGS.iter().map(|s| s.label.len()).max().unwrap_or(0);
        for (i, (s, v)) in SETTINGS.iter().zip(&shown).enumerate() {
            println!("  {:>2}  {:width$}  {v}", i + 1, s.label);
        }
        println!("   0  Back");
        println!();
        if !message.is_empty() {
            println!("  {message}");
            println!();
            message.clear();
        }
        let Some(choice) = prompt(stdin, "Choose a setting to change")? else {
            continue;
        };
        if matches!(choice.as_str(), "0" | "q" | "b" | "back") {
            return Ok(());
        }
        let Some(i) = choice
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .filter(|n| *n < SETTINGS.len())
        else {
            continue;
        };
        let s = &SETTINGS[i];
        let result = new_value(s, &shown[i], stdin).and_then(|new| {
            let Some(new) = new else {
                return Ok(None);
            };
            let out = set(&text, s.section, s.key, &new)?;
            std::fs::write(config, out)
                .with_context(|| format!("cannot write {}", config.display()))?;
            Ok(Some(()))
        });
        message = match result {
            Ok(Some(())) => format!("Saved: {}", s.label),
            Ok(None) => String::new(),
            Err(e) => format!("Not changed: {e:#}"),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = rwm_config::DEFAULT_CONFIG;

    #[test]
    fn every_setting_is_known_to_the_config() {
        let defaults = rwm_config::parse(DEFAULT).unwrap().settings;
        for s in SETTINGS {
            assert!(!current(s, &defaults).is_empty(), "{}", s.key);
        }
    }

    #[test]
    fn edits_keep_comments_and_apply() {
        let out = set(
            DEFAULT,
            "general",
            "focus_follows_mouse",
            &NewValue::Bool(true),
        )
        .unwrap();
        assert!(out.contains("# Focus a window just by moving the mouse"));
        assert!(
            rwm_config::parse(&out)
                .unwrap()
                .settings
                .focus_follows_mouse
        );

        let out = set(&out, "gaps", "inner", &NewValue::Int(20)).unwrap();
        assert!(out.contains("inner = 20      # between windows"), "{out}");

        let out = set(&out, "border", "color", &NewValue::Str("#ff0000".into())).unwrap();
        assert_eq!(
            rwm_config::parse(&out).unwrap().settings.border.color,
            0xff0000
        );

        let out = set(
            &out,
            "general",
            "terminal",
            &NewValue::Str("alacritty.exe".into()),
        )
        .unwrap();
        let s = rwm_config::parse(&out).unwrap().settings;
        assert_eq!(s.terminal.as_deref(), Some("alacritty.exe"));
        let out = set(&out, "general", "terminal", &NewValue::Remove).unwrap();
        assert_eq!(rwm_config::parse(&out).unwrap().settings.terminal, None);
    }

    #[test]
    fn missing_sections_are_added() {
        let out = set("", "gaps", "inner", &NewValue::Int(20)).unwrap();
        assert_eq!(rwm_config::parse(&out).unwrap().settings.gaps.inner, 20);
        let out = set(
            &out,
            "general",
            "workspace_mode",
            &NewValue::Str("global".into()),
        )
        .unwrap();
        assert_eq!(
            rwm_config::parse(&out)
                .unwrap()
                .settings
                .workspace_mode
                .to_string(),
            "global"
        );
    }

    #[test]
    fn invalid_results_are_refused() {
        assert!(
            set(
                DEFAULT,
                "general",
                "workspace_mode",
                &NewValue::Str("both".into())
            )
            .is_err()
        );
        assert!(set("not = [toml", "gaps", "inner", &NewValue::Int(1)).is_err());
    }

    #[test]
    fn toggles_and_cycles() {
        let mut no_input = std::io::empty();
        let ffm = &SETTINGS[0];
        assert_eq!(
            new_value(ffm, "off", &mut no_input).unwrap(),
            Some(NewValue::Bool(true))
        );
        let layout = &SETTINGS[2];
        assert_eq!(
            new_value(layout, "monocle", &mut no_input).unwrap(),
            Some(NewValue::Str("columns".into()))
        );
        assert_eq!(
            new_value(layout, "center", &mut no_input).unwrap(),
            Some(NewValue::Str("dwindle".into()))
        );
        let mut typed = std::io::Cursor::new("12\n");
        let width = &SETTINGS[8];
        assert_eq!(
            new_value(width, "3", &mut typed).unwrap(),
            Some(NewValue::Int(12))
        );
        let mut typed = std::io::Cursor::new("99\n");
        assert!(new_value(width, "3", &mut typed).is_err());
    }
}
