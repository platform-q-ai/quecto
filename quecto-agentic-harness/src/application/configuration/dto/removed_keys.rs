//! #2414 review H1: the files that still set keys the watermark context
//! removed, and the message that names every one with the command that
//! removes it.

use crate::domain::conversation::removed_keys::{WHY_REMOVED, removed_keys_set};
use serde_json::{Map, Value};
use std::path::Path;

/// One file's removed keys and the `quecto config unset` layer flag
/// (`--global` or `--local`) that addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedKeysIn {
    pub path: std::path::PathBuf,
    pub flag: &'static str,
    pub keys: Vec<&'static str>,
}

impl RemovedKeysIn {
    /// `None` when `document` sets none.
    pub fn of(path: &Path, flag: &'static str, document: &Map<String, Value>) -> Option<Self> {
        let keys = removed_keys_set(document);
        (!keys.is_empty()).then(|| Self {
            path: path.to_path_buf(),
            flag,
            keys,
        })
    }
}

/// The message naming every removed key of every file, with the command
/// that removes each.
pub fn removed_keys_message(files: &[RemovedKeysIn]) -> String {
    assert!(
        files.iter().all(|file| !file.keys.is_empty()),
        "a file is named only for keys it sets"
    );
    let mut message = format!("configuration keys were {WHY_REMOVED}; remove each one:");
    for file in files {
        message.push_str(&format!("\n  {}:", file.path.display()));
        for key in &file.keys {
            message.push_str(&format!(
                "\n    quecto config unset agents.defaults.{key} {}",
                file.flag
            ));
        }
    }
    message
}

#[cfg(test)]
#[path = "removed_keys_tests.rs"]
mod tests;
