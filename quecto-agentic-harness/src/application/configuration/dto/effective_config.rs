//! The effective configuration of a run (#2024): the merged JSON document
//! and a report of the layers that produced it.

use std::path::PathBuf;

/// The merged document plus where each part came from.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveConfig {
    /// The global (or explicit) document with a trusted overlay merged in.
    pub document: serde_json::Value,
    pub sources: ConfigSources,
}

/// Which files contributed to the effective configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSources {
    /// The base file: `--config <path>` or the global file.
    pub base: PathBuf,
    /// Whether `base` was an explicit `--config` selection (no overlay
    /// applies then).
    pub explicit: bool,
    /// The overlay candidate and what became of it; `None` when the
    /// selection had no overlay location.
    pub overlay: Option<OverlayReport>,
    /// A retired `<cwd>/config.json` that exists but is no longer loaded.
    pub legacy_local: Option<PathBuf>,
}

impl ConfigSources {
    /// The overlay path when one was applied.
    pub fn applied_overlay(&self) -> Option<&PathBuf> {
        self.overlay
            .as_ref()
            .filter(|report| report.state == OverlayState::Applied)
            .map(|report| &report.path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayReport {
    pub path: PathBuf,
    pub state: OverlayState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayState {
    /// No file at the overlay location.
    Absent,
    /// Present, trusted, valid, merged.
    Applied,
    /// Present but not approved: reported and not applied.
    Untrusted { fingerprint: String },
}

/// Why the effective configuration could not be produced. Every variant
/// names the file at fault so the user can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveConfigError {
    /// An explicit `--config` file that does not exist.
    Missing(PathBuf),
    Read {
        path: PathBuf,
        reason: String,
    },
    Parse {
        path: PathBuf,
        reason: String,
    },
    NotAnObject(PathBuf),
    /// The overlay carries a section only the global file may define.
    GlobalOnlyKey {
        path: PathBuf,
        key: String,
    },
    /// The document (one layer, or the merge) fails schema validation.
    Invalid {
        path: PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for EffectiveConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "config not found: {}", path.display()),
            Self::Read { path, reason } => {
                write!(f, "failed to load config {}: {reason}", path.display())
            }
            Self::Parse { path, reason } => {
                write!(
                    f,
                    "failed to load config {}: failed to parse config: {reason}",
                    path.display()
                )
            }
            Self::NotAnObject(path) => write!(
                f,
                "failed to load config {}: the document must be a JSON object",
                path.display()
            ),
            Self::GlobalOnlyKey { path, key } => write!(
                f,
                "failed to load config {}: `{key}` is global-only and cannot be set in a repo-local overlay; define it in the global config.json",
                path.display()
            ),
            Self::Invalid { path, reason } => {
                write!(f, "failed to load config {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for EffectiveConfigError {}
