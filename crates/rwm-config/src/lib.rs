//! rustWM configuration.
//!
//! The full TOML schema, validation and hot reload land in M3. For now this
//! holds the settings the earlier milestones already need, with defaults.

use serde::{Deserialize, Serialize};

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
