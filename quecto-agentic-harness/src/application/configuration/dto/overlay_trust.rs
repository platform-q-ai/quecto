//! Explicit, non-interactive approval of the repo-local overlay (#2024).

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayTrustRequest {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayTrustError {
    Missing(PathBuf),
    /// A symbolic link is never trusted: trust is keyed by the file's
    /// identity, which a link would borrow from its target.
    NotARegularFile(PathBuf),
    Read {
        path: PathBuf,
        reason: String,
    },
    Parse {
        path: PathBuf,
        reason: String,
    },
    NotAnObject(PathBuf),
    /// Approving would let the overlay smuggle a global-only section.
    GlobalOnlyKey {
        path: PathBuf,
        key: String,
    },
    Invalid {
        path: PathBuf,
        reason: String,
    },
    Store {
        path: PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for OverlayTrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "no overlay to trust at {}", path.display()),
            Self::NotARegularFile(path) => write!(
                f,
                "refusing to trust {}: it is a symbolic link, and a repo-local overlay must be a regular file (replace the link with a copy)",
                path.display()
            ),
            Self::Read { path, reason } => {
                write!(f, "failed to read overlay {}: {reason}", path.display())
            }
            Self::Parse { path, reason } => {
                write!(f, "failed to parse overlay {}: {reason}", path.display())
            }
            Self::NotAnObject(path) => {
                write!(f, "overlay {} must be a JSON object", path.display())
            }
            Self::GlobalOnlyKey { path, key } => write!(
                f,
                "refusing to trust {}: `{key}` is global-only and cannot be set in a repo-local overlay",
                path.display()
            ),
            Self::Invalid { path, reason } => write!(
                f,
                "refusing to trust {}: not a valid overlay: {reason}",
                path.display()
            ),
            Self::Store { path, reason } => {
                write!(f, "could not record trust for {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for OverlayTrustError {}
