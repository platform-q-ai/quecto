use crate::domain::error::DomainError;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct FilesystemScope;

impl FilesystemScope {
    pub fn canonicalize(&self, path: &Path) -> Result<PathBuf, DomainError> {
        let directory = path.canonicalize().map_err(|error| {
            DomainError::Session(format!("workspace canonicalization failed: {error}"))
        })?;
        let metadata = directory.metadata().map_err(|error| {
            DomainError::Session(format!("workspace metadata unavailable: {error}"))
        })?;
        if metadata.is_dir() {
            // Reading the directory verifies access rather than treating inaccessible paths as empty.
            std::fs::read_dir(&directory).map_err(|error| {
                DomainError::Session(format!("workspace directory unavailable: {error}"))
            })?;
            assert!(directory.is_absolute());
            Ok(directory)
        } else {
            Err(DomainError::Session("workspace must be a directory".into()))
        }
    }
}

#[cfg(test)]
#[path = "filesystem_scope_tests.rs"]
mod tests;
