//! Persistent adapter of the configuration capability's
//! [`OverlayTrustStore`] port (#2024): a repo-local overlay is trusted by
//! canonical path and SHA-256 of its exact content, recorded in a per-user
//! record under the quecto base directory. On a miss the adapter may ask
//! an interactive user (stderr prompt, stdin answer) when composed to; the
//! non-interactive path is `quecto config trust`, which records an
//! approval through [`OverlayTrustStore::approve`].
//!
//! The record primitives are shared with the container-config overlay of
//! `repo_local_container_config` (same JSON shape, separate file).

use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::application::configuration::ports::{OverlayApproval, OverlayTrust, OverlayTrustStore};

/// The trust record's file name under the base directory.
pub const TRUST_RECORD_FILE_NAME: &str = "config-overlay-trust.json";

#[derive(Debug, Clone)]
pub struct PersistentOverlayTrustStore {
    record_path: PathBuf,
    prompt_on_miss: bool,
}

impl PersistentOverlayTrustStore {
    /// Trust recorded in `<base_dir>/config-overlay-trust.json`; with
    /// `prompt_on_miss` an unrecorded overlay is offered to the terminal
    /// user once (denied when stdin is not a terminal).
    pub fn for_base_dir(base_dir: &Path, prompt_on_miss: bool) -> Self {
        Self {
            record_path: base_dir.join(TRUST_RECORD_FILE_NAME),
            prompt_on_miss,
        }
    }

    pub fn record_path(&self) -> &Path {
        &self.record_path
    }
}

impl OverlayTrustStore for PersistentOverlayTrustStore {
    fn decide(&self, path: &Path, content: &[u8]) -> OverlayTrust {
        let identity = canonical_identity(path);
        let fingerprint = hex_sha256(content);
        if read_record(&self.record_path).is_approved(&identity, &fingerprint) {
            OverlayTrust::Trusted
        } else {
            OverlayTrust::Untrusted { fingerprint }
        }
    }

    /// Consent only: the caller validates the overlay and records the
    /// approval, so a "y" on a broken overlay trusts nothing.
    fn offer(&self, path: &Path, fingerprint: &str) -> bool {
        self.prompt_on_miss && prompt_approval(path, fingerprint)
    }

    fn approve(&self, path: &Path, content: &[u8]) -> Result<OverlayApproval, String> {
        let identity = canonical_identity(path);
        let fingerprint = hex_sha256(content);
        let mut record = read_record(&self.record_path);
        record.approve(identity.clone(), fingerprint.clone());
        write_record(&self.record_path, &record)
            .map_err(|error| format!("{}: {error}", self.record_path.display()))?;
        Ok(OverlayApproval {
            path: PathBuf::from(identity),
            fingerprint,
        })
    }
}

/// The path an approval is keyed by: canonical when the file resolves,
/// else the path made absolute against the working directory.
fn canonical_identity(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            }
        })
        .to_string_lossy()
        .into_owned()
}

fn prompt_approval(path: &Path, fingerprint: &str) -> bool {
    if !io::stdin().is_terminal() {
        return false;
    }
    eprint!(
        "Trust repo-local config overlay {} (sha256 {fingerprint})? [y/N] ",
        path.display()
    );
    let _ = io::stderr().flush();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map(|_| matches!(input.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
        .unwrap_or(false)
}

/// The one approved content hash per canonical path. A hand edit revokes
/// trust, and reverting to an earlier content does not restore it: only
/// the content approved last is trusted. Records written before #2024
/// held every approved hash per path; those keep matching any of them
/// until the next approval collapses the entry to one.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct TrustRecord {
    #[serde(default)]
    pub(crate) approved: HashMap<String, Fingerprint>,
}

/// One fingerprint, accepting the pre-#2024 list shape on read.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub(crate) enum Fingerprint {
    One(String),
    Legacy(Vec<String>),
}

impl Fingerprint {
    fn matches(&self, fingerprint: &str) -> bool {
        match self {
            Self::One(one) => one == fingerprint,
            Self::Legacy(list) => list.iter().any(|recorded| recorded == fingerprint),
        }
    }
}

impl TrustRecord {
    pub(crate) fn is_approved(&self, identity: &str, fingerprint: &str) -> bool {
        self.approved
            .get(identity)
            .is_some_and(|recorded| recorded.matches(fingerprint))
    }

    pub(crate) fn approve(&mut self, identity: String, fingerprint: String) {
        self.approved
            .insert(identity, Fingerprint::One(fingerprint));
    }
}

/// An unreadable or malformed record is empty: nothing is trusted until
/// approved again.
pub(crate) fn read_record(path: &Path) -> TrustRecord {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// The directory that must exist before writing `path`, or `None` when the
/// path is a bare filename and no directory needs creating. Split out so the
/// bare-filename case is testable without changing the process working
/// directory, which is global and breaks tests running in parallel.
pub(crate) fn record_parent_to_create(path: &Path) -> Option<&Path> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent),
        _ => None,
    }
}

/// Atomic, so a crash mid-write cannot leave a partial record (which would
/// read as "nothing trusted" — safe, but silently).
pub(crate) fn write_record(path: &Path, record: &TrustRecord) -> io::Result<()> {
    if let Some(parent) = record_parent_to_create(path) {
        std::fs::create_dir_all(parent)?;
    }
    crate::infrastructure::atomic_write::atomic_write(
        path,
        &serde_json::to_vec_pretty(record)?,
        Some(0o600),
    )
}

pub(crate) fn hex_sha256(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    format!("{digest:x}")
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
