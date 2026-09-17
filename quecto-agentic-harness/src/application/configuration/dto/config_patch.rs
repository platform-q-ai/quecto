//! A single-path patch of one configuration document (#2024).

use std::path::PathBuf;

/// Which layer a document belongs to; decides which validations apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLayer {
    /// The global file (or an explicit `--config` file): any section.
    Global,
    /// The repo-local overlay: overlay sections only, trust re-recorded
    /// after every write.
    Overlay,
}

/// The document to patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigTarget {
    pub layer: ConfigLayer,
    pub path: PathBuf,
}

/// Set `key_path` (dotted, e.g. `agents.defaults.model`) to `value` in the
/// target document, creating intermediate objects as needed.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigPatch {
    pub target: ConfigTarget,
    pub key_path: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPatchReceipt {
    pub path: PathBuf,
    /// Whether the file did not exist before this patch.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigPatchError {
    /// The key path is empty or has an empty segment.
    InvalidKeyPath(String),
    /// The top-level key is global-only and the target is the overlay.
    GlobalOnlyKey {
        path: PathBuf,
        key: String,
    },
    /// The overlay exists but its current content is not approved; patching
    /// it would silently trust that content.
    UntrustedOverlay {
        path: PathBuf,
        fingerprint: String,
    },
    Read {
        path: PathBuf,
        reason: String,
    },
    Parse {
        path: PathBuf,
        reason: String,
    },
    /// The document, or an intermediate value on the key path, is not an
    /// object.
    NotAnObject {
        path: PathBuf,
        at: String,
    },
    /// The patched document does not pass schema validation; nothing was
    /// written.
    Invalid {
        path: PathBuf,
        reason: String,
    },
    Write {
        path: PathBuf,
        reason: String,
    },
    /// The overlay was written but its new content could not be recorded
    /// as trusted.
    Trust {
        path: PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for ConfigPatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKeyPath(key_path) => {
                write!(
                    f,
                    "invalid config path `{key_path}`: expected dotted keys such as agents.defaults.model"
                )
            }
            Self::GlobalOnlyKey { path, key } => write!(
                f,
                "cannot set `{key}` in {}: `{key}` is global-only; use --global",
                path.display()
            ),
            Self::UntrustedOverlay { path, fingerprint } => write!(
                f,
                "overlay {} is not trusted (sha256 {fingerprint}); review it and run `quecto config trust` first",
                path.display()
            ),
            Self::Read { path, reason } => {
                write!(f, "failed to read config {}: {reason}", path.display())
            }
            Self::Parse { path, reason } => {
                write!(f, "failed to parse config {}: {reason}", path.display())
            }
            Self::NotAnObject { path, at } => write!(
                f,
                "cannot set a key under `{at}` in {}: it is not a JSON object",
                path.display()
            ),
            Self::Invalid { path, reason } => write!(
                f,
                "refusing to write {}: the result is not a valid configuration: {reason}",
                path.display()
            ),
            Self::Write { path, reason } => {
                write!(f, "failed to write config {}: {reason}", path.display())
            }
            Self::Trust { path, reason } => write!(
                f,
                "wrote {} but could not record it as trusted: {reason}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ConfigPatchError {}
