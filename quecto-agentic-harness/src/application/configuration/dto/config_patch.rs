//! A single-path patch of one configuration document (#2024).

use std::path::PathBuf;

use super::config_selection::ConfigSelection;
use super::effective_config::EffectiveConfigError;

/// Which layer a document belongs to; decides which validations apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLayer {
    /// The global file (or an explicit `--config` file): any section.
    Global,
    /// The repo-local overlay: overlay sections only, trust re-recorded
    /// after every write.
    Overlay,
}

/// Set `key_path` (dotted, e.g. `agents.defaults.model`) to `value` in
/// one layer of `selection`, creating intermediate objects as needed. The
/// selection is what the patched layer is validated *against*: the result
/// must be a configuration the run in this directory would load.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigPatch {
    pub selection: ConfigSelection,
    pub layer: ConfigLayer,
    pub key_path: String,
    pub value: serde_json::Value,
}

/// Remove `key_path` from one layer of `selection` (#2024 S2): the
/// rollback of a `set`. A key that is not set in that layer is an error,
/// never a silent no-op (the caller may be looking at the other layer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigUnset {
    pub selection: ConfigSelection,
    pub layer: ConfigLayer,
    pub key_path: String,
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
    /// The selection has no overlay location (unknown working directory or
    /// an explicit `--config`), so there is no overlay to patch.
    NoOverlayLocation,
    /// The store's overlay policy refuses the entry at the overlay
    /// location (a symbolic link on the way to it); nothing is written
    /// through it.
    Refused {
        path: PathBuf,
        reason: String,
    },
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
    /// An unset of a key the layer does not set (or a file that does not
    /// exist); nothing to remove.
    NotSet {
        path: PathBuf,
        key_path: String,
    },
    /// The patched document does not pass schema validation; nothing was
    /// written.
    Invalid {
        path: PathBuf,
        reason: String,
    },
    /// The patched layer is valid on its own, but the effective
    /// configuration it would produce with the other layer is not; nothing
    /// was written.
    InvalidMerge {
        path: PathBuf,
        reason: EffectiveConfigError,
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
            Self::NoOverlayLocation => write!(
                f,
                "no repo-local overlay applies to this run (an explicit --config replaces both layers, and an unknown working directory has none)"
            ),
            Self::Refused { path, reason } => {
                write!(f, "refusing to write {}: {reason}", path.display())
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
            Self::NotAnObject { path, at } if at.is_empty() => write!(
                f,
                "cannot address a key in {}: the document is not a JSON object",
                path.display()
            ),
            Self::NotAnObject { path, at } => write!(
                f,
                "cannot address a key under `{at}` in {}: it is not a JSON object",
                path.display()
            ),
            Self::NotSet { path, key_path } => write!(
                f,
                "`{key_path}` is not set in {}; nothing to unset (the other layer may set it: check `quecto config get --global` / `--local`)",
                path.display()
            ),
            Self::Invalid { path, reason } => write!(
                f,
                "refusing to write {}: the result is not a valid configuration: {reason}",
                path.display()
            ),
            Self::InvalidMerge { path, reason } => write!(
                f,
                "refusing to write {}: the configuration this directory would load is not valid: {reason}",
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
