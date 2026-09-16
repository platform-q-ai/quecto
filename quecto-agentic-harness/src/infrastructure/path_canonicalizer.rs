//! Filesystem path canonicalizer adapter for #2001 D2.
//! Implements [`PathCanonicalizer`] over `std::fs::canonicalize`.

use crate::application::sessions::ports::PathCanonicalizer;
use crate::domain::error::DomainError;
use crate::domain::session_home_scope::CanonicalExecutionLocation;

/// Real filesystem canonicalizer (resolves symlinks to absolute paths).
#[derive(Debug, Default, Clone, Copy)]
pub struct FsPathCanonicalizer;

impl FsPathCanonicalizer {
    pub fn new() -> Self {
        Self
    }
}

impl PathCanonicalizer for FsPathCanonicalizer {
    fn canonicalize(&self, path: &str) -> Result<CanonicalExecutionLocation, DomainError> {
        if path.is_empty() {
            return Err(DomainError::Session(
                "path to canonicalize must be non-empty".to_string(),
            ));
        }
        let canonical = std::fs::canonicalize(path).map_err(|e| {
            DomainError::Session(format!("failed to canonicalize path {path:?}: {e}"))
        })?;
        let text = canonical.to_string_lossy().into_owned();
        CanonicalExecutionLocation::try_from_canonical_path(text)
    }
}

#[cfg(test)]
#[path = "path_canonicalizer_tests.rs"]
mod tests;
