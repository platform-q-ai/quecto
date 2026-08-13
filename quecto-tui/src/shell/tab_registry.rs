//! Local tab-agent registry sidecar (#1465 AC4 / ADR-0023).
//!
//! Durable records of detached/live tab agents (pid, socket, session key, name,
//! workspace id, timestamps/status) with atomic load/store and GC helpers.
//! No dependency on `quecto-runtime-manager`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::atomic_file::write_atomic;

/// On-disk schema version for the registry sidecar.
pub const REGISTRY_SCHEMA_VERSION: u32 = 1;

/// Default relative path under the TUI data root.
pub const DEFAULT_REGISTRY_FILE_NAME: &str = "tab-agent-registry.json";

/// Lifecycle status recorded for a registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabAgentStatus {
    Live,
    Dead,
    Unknown,
}

/// One tab-agent record in the local registry sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabAgentRecord {
    pub tab_id: u32,
    pub pid: Option<u32>,
    pub socket_path: PathBuf,
    pub session_key: Option<String>,
    pub tab_name: Option<String>,
    pub workspace_id: Option<String>,
    /// Unix seconds when the record was last written/updated.
    pub updated_unix_s: u64,
    pub status: TabAgentStatus,
}

/// Full registry document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabAgentRegistry {
    pub version: u32,
    pub agents: Vec<TabAgentRecord>,
}

impl Default for TabAgentRegistry {
    fn default() -> Self {
        Self {
            version: REGISTRY_SCHEMA_VERSION,
            agents: Vec::new(),
        }
    }
}

impl TabAgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from disk. Missing file → empty registry. Corrupt/partial JSON →
    /// empty registry (AC4: crashed writes must not poison restart).
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        match serde_json::from_slice::<TabAgentRegistry>(&bytes) {
            Ok(reg) if reg.version == REGISTRY_SCHEMA_VERSION => {
                // Keep placeholder rows (empty socket) so connecting tabs remain
                // durable; GC/liveness probes drop them when still unreachable.
                reg
            }
            _ => Self::default(),
        }
    }

    /// Atomically persist the registry document.
    pub fn store(&self, path: &Path) -> io::Result<()> {
        let mut out = self.clone();
        out.version = REGISTRY_SCHEMA_VERSION;
        let bytes = serde_json::to_vec_pretty(&out)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        write_atomic(path, &bytes)
    }

    /// Upsert by stable durable identity.
    ///
    /// `tab_id` is reused across TUI lifetimes (notably master tab 0), so it is
    /// not sufficient on its own: a fresh master must not overwrite a detached
    /// live master whose workspace manifest still points at `(tab_id,
    /// session_key)`. Prefer the manifest identity when present; otherwise fall
    /// back to the concrete socket identity for pre-session/placeholder rows.
    pub fn upsert(&mut self, record: TabAgentRecord) {
        if let Some(existing) = self
            .agents
            .iter_mut()
            .find(|a| Self::is_same_durable_identity(a, &record))
        {
            *existing = record;
        } else {
            self.agents.push(record);
        }
    }

    /// Is `candidate` the same durable owner row as `record`?
    fn is_same_durable_identity(candidate: &TabAgentRecord, record: &TabAgentRecord) -> bool {
        candidate.tab_id == record.tab_id
            && candidate.workspace_id == record.workspace_id
            && match (&candidate.session_key, &record.session_key) {
                (Some(a), Some(b)) => a == b,
                _ => candidate.socket_path == record.socket_path,
            }
    }

    /// Remove entries whose liveness probe reports dead, and entries already
    /// marked [`TabAgentStatus::Dead`].
    pub fn gc_dead<F>(&mut self, mut is_live: F)
    where
        F: FnMut(&TabAgentRecord) -> bool,
    {
        self.agents.retain(|a| {
            if a.status == TabAgentStatus::Dead {
                return false;
            }
            is_live(a)
        });
    }

    /// Mark status from a liveness probe without removing rows.
    pub fn refresh_status<F>(&mut self, mut is_live: F)
    where
        F: FnMut(&TabAgentRecord) -> bool,
    {
        let now = unix_now_s();
        for a in &mut self.agents {
            a.status = if is_live(a) {
                TabAgentStatus::Live
            } else {
                TabAgentStatus::Dead
            };
            a.updated_unix_s = now;
        }
    }
}

/// Default path: `$XDG_DATA_HOME/quecto/tui/tab-agent-registry.json` or
/// `~/.local/share/quecto/tui/...`.
pub fn default_registry_path() -> PathBuf {
    tui_data_dir().join(DEFAULT_REGISTRY_FILE_NAME)
}

pub(crate) fn tui_data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("quecto").join("tui");
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("share").join("quecto").join("tui")
}

pub(crate) fn unix_now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Best-effort liveness: process exists (when pid known) AND socket path is a
/// live unix socket file. Used by GC; callers may supply a stricter probe.
pub fn default_liveness_probe(record: &TabAgentRecord) -> bool {
    if let Some(pid) = record.pid {
        if !pid_exists(pid) {
            return false;
        }
    }
    socket_path_present(&record.socket_path)
}

fn pid_exists(pid: u32) -> bool {
    // Prefer the procfs node over kill(0) so this helper stays safe-Rust and
    // still answers "is this pid still around?" for GC.
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn socket_path_present(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}

/// True when `path` exists and is a unix-domain socket file (AC6 stale probe).
pub fn socket_path_is_live(path: &Path) -> bool {
    !path.as_os_str().is_empty() && socket_path_present(path)
}

#[cfg(test)]
#[path = "tab_registry_tests.rs"]
mod tab_registry_tests;
