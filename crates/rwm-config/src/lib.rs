//! rustWM configuration.
//!
//! The full TOML schema, validation and hot reload land in M3. For now this
//! holds the settings and default key bindings the earlier milestones need.

pub mod keys;

use serde::{Deserialize, Serialize};

pub use keys::{Chord, KeyError};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub default_layout: String,
    /// Command line launched by the `terminal` command. `None` falls back to
    /// `wt.exe`, then `cmd.exe`.
    pub terminal: Option<String>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            default_layout: "dwindle".into(),
            terminal: None,
        }
    }
}

/// The built-in key map: vim-style focus and movement on Win+H/J/K/L (and the
/// arrow keys), Win+Enter for a terminal.
pub const DEFAULT_BINDINGS: &[(&str, &str)] = &[
    ("win+h", "focus left"),
    ("win+j", "focus down"),
    ("win+k", "focus up"),
    ("win+l", "focus right"),
    ("win+left", "focus left"),
    ("win+down", "focus down"),
    ("win+up", "focus up"),
    ("win+right", "focus right"),
    ("win+shift+h", "move left"),
    ("win+shift+j", "move down"),
    ("win+shift+k", "move up"),
    ("win+shift+l", "move right"),
    ("win+shift+left", "move left"),
    ("win+shift+down", "move down"),
    ("win+shift+up", "move up"),
    ("win+shift+right", "move right"),
    ("win+n", "focus next"),
    ("win+p", "focus prev"),
    ("win+enter", "terminal"),
    ("win+shift+enter", "promote"),
    ("win+shift+q", "close"),
    ("win+shift+r", "retile"),
    ("win+ctrl+shift+q", "quit"),
];

/// [`DEFAULT_BINDINGS`] parsed into chords.
pub fn default_bindings() -> Vec<(Chord, String)> {
    DEFAULT_BINDINGS
        .iter()
        .map(|(k, c)| {
            (
                k.parse().expect("built-in binding parses"),
                (*c).to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_parse_and_are_unique() {
        let b = default_bindings();
        assert_eq!(b.len(), DEFAULT_BINDINGS.len());
        let mut chords: Vec<_> = b.iter().map(|(c, _)| c.to_string()).collect();
        chords.sort();
        chords.dedup();
        assert_eq!(chords.len(), b.len());
    }

    #[test]
    fn default_binding_commands_are_valid() {
        for (_, cmd) in DEFAULT_BINDINGS {
            cmd.parse::<rwm_core::Command>().unwrap();
        }
    }
}
