//! Session-scoped registry of script-managed environments.
//!
//! Per ADR-0021 composition builds exactly one registry per session and
//! injects it into the launch services. It is the authority for minting
//! never-reused `C1`-style environment refs and for recording which
//! environments this session has committed. Slice 2 extends it with
//! membership, status, and kill/list control operations.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One committed script-managed environment known to this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentRecord {
    /// Session-local, never-reused ref (e.g. `C1`) minted by this registry.
    pub environment_ref: String,
    /// Script/runtime-owned environment identity from the create result.
    pub environment_id: String,
    /// Workspace path reported by the create result.
    pub workspace_path: PathBuf,
    /// Name of the configured container script set that created it.
    pub script_name: String,
}

#[derive(Debug, Default)]
struct EnvironmentRegistryState {
    next_ref: u64,
    entries: BTreeMap<String, EnvironmentRecord>,
}

/// Cloneable handle to one session's environment registry.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentRegistry {
    state: Arc<Mutex<EnvironmentRegistryState>>,
}

impl EnvironmentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint the next `CN` ref. Refs are monotonic and never reused within a
    /// session, even when the launch they were minted for later fails.
    pub fn mint_ref(&self) -> String {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.next_ref += 1;
        format!("C{}", state.next_ref)
    }

    /// Commit a created environment under its minted ref.
    pub fn commit(&self, record: EnvironmentRecord) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.insert(record.environment_ref.clone(), record);
    }

    /// Remove a committed environment (launch rollback/uncommit).
    pub fn remove(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.remove(environment_ref)
    }

    pub fn get(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.get(environment_ref).cloned()
    }

    pub fn entries(&self) -> Vec<EnvironmentRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.values().cloned().collect()
    }
}

#[cfg(test)]
#[path = "environment_registry_tests.rs"]
mod tests;
