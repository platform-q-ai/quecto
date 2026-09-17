//! Filesystem adapter of the configuration capability's
//! [`ConfigDocumentStore`] port (#2024): reads report a present-but-broken
//! entry as an error (never as absence), so selection never falls back
//! past a file that exists.

use std::path::Path;

use crate::application::configuration::ports::ConfigDocumentStore;

#[derive(Debug, Default, Clone, Copy)]
pub struct FilesystemConfigDocumentStore;

impl ConfigDocumentStore for FilesystemConfigDocumentStore {
    /// `symlink_metadata` decides presence from the directory entry itself,
    /// so a dangling symlink is *present* (and then unreadable) rather than
    /// absent; a directory is present and not readable as a file.
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        }
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err("not a regular file; move it aside or pass --config".into()),
            Err(error) => return Err(format!("cannot be read: {error}")),
        }
        std::fs::read(path)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn is_present(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }
}

#[cfg(test)]
#[path = "loaders_tests.rs"]
mod tests;
