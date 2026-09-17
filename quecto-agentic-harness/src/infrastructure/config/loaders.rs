//! Filesystem adapter of the configuration capability's
//! [`ConfigDocumentStore`] port (#2024): reads report a present-but-broken
//! entry as an error (never as absence), so selection never falls back
//! past a file that exists; the overlay read states the one overlay
//! policy — no symbolic link below the working directory.

use std::path::Path;

use crate::application::configuration::ports::config_document_store::OVERLAY_RELATIVE_PATH;
use crate::application::configuration::ports::{ConfigDocumentStore, OverlayDocument};

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

    /// The overlay policy, stated once: the entries `path` names below the
    /// working directory (as many trailing components as
    /// `OVERLAY_RELATIVE_PATH` has — the `.quecto` directory and the file)
    /// are each inspected as a directory entry, and the first symbolic link
    /// among them, from the file up, is the refusal. The working directory
    /// itself and everything above it is selection's business (it is
    /// canonicalised before the candidate is named), not the overlay's.
    fn read_overlay(&self, path: &Path) -> Result<OverlayDocument, String> {
        let mut entry = path;
        for _ in Path::new(OVERLAY_RELATIVE_PATH).components() {
            if is_symlink(entry) {
                return Ok(OverlayDocument::Refused {
                    reason: symlink_refusal(entry),
                });
            }
            entry = entry.parent().ok_or_else(|| {
                format!(
                    "{} is not an overlay location (expected <working directory>/{OVERLAY_RELATIVE_PATH})",
                    path.display()
                )
            })?;
        }
        Ok(match self.read(path)? {
            Some(bytes) => OverlayDocument::Present(bytes),
            None => OverlayDocument::Absent,
        })
    }
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// Why a symbolic link on the overlay's stretch never applies and is never
/// written through: trust is keyed by the file's identity, which a link
/// would borrow from its target, and a write would land in the target.
fn symlink_refusal(link: &Path) -> String {
    format!(
        "{} is a symbolic link, and a repo-local overlay must be a regular file in a regular `.quecto` directory (replace the link with a copy)",
        link.display()
    )
}

#[cfg(test)]
#[path = "loaders_tests.rs"]
mod tests;
