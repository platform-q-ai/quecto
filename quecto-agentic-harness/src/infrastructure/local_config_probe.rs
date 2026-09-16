//! Filesystem adapter of the configuration capability's
//! [`LocalConfigProbe`] port (#1966).

use std::path::Path;

use crate::application::configuration::ports::{LocalConfigPresence, LocalConfigProbe};

/// Probes the real filesystem. `symlink_metadata` decides presence from the
/// directory entry itself, so a dangling symlink is *present* (and then
/// unreadable) rather than absent; `metadata` follows links so a symlink to
/// a regular file is usable.
#[derive(Debug, Default, Clone, Copy)]
pub struct FilesystemLocalConfigProbe;

impl LocalConfigProbe for FilesystemLocalConfigProbe {
    fn probe(&self, path: &Path) -> LocalConfigPresence {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return LocalConfigPresence::Absent;
            }
            Err(error) => return LocalConfigPresence::Unreadable(error.to_string()),
        }
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => LocalConfigPresence::RegularFile,
            Ok(_) => LocalConfigPresence::NotRegularFile,
            Err(error) => LocalConfigPresence::Unreadable(error.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "local_config_probe_tests.rs"]
mod tests;
