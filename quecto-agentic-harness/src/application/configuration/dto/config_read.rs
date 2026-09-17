//! Reading configuration values (#2024): one layer's raw document, or the
//! effective merge, optionally narrowed to a dotted key path.

use super::config_selection::ConfigSelection;
use super::effective_config::{ConfigSources, EffectiveConfigError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigReadScope {
    /// The merged document a run would load.
    Effective,
    /// The global (or explicit) file as written.
    Global,
    /// The overlay file as written, whether or not it is trusted.
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigReadRequest {
    pub selection: ConfigSelection,
    pub scope: ConfigReadScope,
    /// A dotted key path; `None` reads the whole document.
    pub key_path: Option<String>,
    /// Print secret-shaped leaves (API keys, tokens, passwords) as written.
    /// Off, they read as `"<redacted>"`: the output of a read lands in
    /// transcripts and model context.
    pub reveal_secrets: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigReadout {
    pub value: serde_json::Value,
    /// The layer report, for the effective scope only.
    pub sources: Option<ConfigSources>,
    /// How many leaves were redacted; zero when secrets were revealed or
    /// there were none.
    pub redacted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigReadError {
    Effective(EffectiveConfigError),
    /// The selection has no overlay location (unknown working directory or
    /// an explicit `--config`).
    NoOverlayLocation,
    Read {
        path: std::path::PathBuf,
        reason: String,
    },
    Parse {
        path: std::path::PathBuf,
        reason: String,
    },
    /// Nothing at `key_path` in the requested scope.
    NotSet(String),
}

impl std::fmt::Display for ConfigReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Effective(error) => write!(f, "{error}"),
            Self::NoOverlayLocation => {
                write!(
                    f,
                    "no repo-local overlay applies to this run (an explicit --config replaces both layers)"
                )
            }
            Self::Read { path, reason } => {
                write!(f, "failed to read config {}: {reason}", path.display())
            }
            Self::Parse { path, reason } => {
                write!(f, "failed to parse config {}: {reason}", path.display())
            }
            Self::NotSet(key_path) => write!(f, "`{key_path}` is not set"),
        }
    }
}

impl std::error::Error for ConfigReadError {}
