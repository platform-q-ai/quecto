//! A named container config (#1410), split from `config.rs` for its line cap.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ContainerConfig {
    /// Marks the config `container: true` selects. The label travels with
    /// the entry when copied between config files.
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub create: Vec<String>,
    #[serde(default)]
    pub cleanup: Vec<String>,
    /// Argv for joining an existing environment (#1369 slice 2).
    #[serde(default)]
    pub exec: Vec<String>,
    /// Argv for stopping an environment (#1369 slice 2).
    #[serde(default)]
    pub kill: Vec<String>,
    /// Argv for the post-mortem inspect of a dead member (#1369 slice 3).
    #[serde(default)]
    pub inspect: Vec<String>,
}
