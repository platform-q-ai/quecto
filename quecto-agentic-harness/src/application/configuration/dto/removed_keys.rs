//! #2414 review H1: the files that still set keys the watermark context
//! removed, and the message that names every one with the command that
//! removes it.

use crate::application::configuration::removed_keys::{WHY_REMOVED, removed_keys_set};
use serde_json::{Map, Value};
use std::path::Path;

/// How a file's removed keys can be taken out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// `quecto config unset` with this layer flag (`--global`, or `--local`
    /// for a trusted overlay).
    Unset(&'static str),
    /// An untrusted overlay: no write goes through it (a write records
    /// trust), so it is edited by hand and then trusted (review round 2 M1).
    EditThenTrust,
}

/// One file's removed keys and how they are taken out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedKeysIn {
    pub path: std::path::PathBuf,
    pub repair: Repair,
    pub keys: Vec<&'static str>,
}

impl RemovedKeysIn {
    /// `None` when `document` sets none.
    pub fn of(path: &Path, repair: Repair, document: &Map<String, Value>) -> Option<Self> {
        let keys = removed_keys_set(document);
        (!keys.is_empty()).then(|| Self {
            path: path.to_path_buf(),
            repair,
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
        match file.repair {
            Repair::Unset(flag) => {
                message.push_str(&format!("\n  {}:", file.path.display()));
                for key in &file.keys {
                    message.push_str(&format!(
                        "\n    quecto config unset agents.defaults.{key} {flag}"
                    ));
                }
            }
            Repair::EditThenTrust => {
                message.push_str(&format!(
                    "\n  remove these keys from {} by editing it, then run `quecto config trust`:",
                    file.path.display()
                ));
                for key in &file.keys {
                    message.push_str(&format!("\n    agents.defaults.{key}"));
                }
            }
        }
    }
    message
}

#[cfg(test)]
#[path = "removed_keys_tests.rs"]
mod tests;
