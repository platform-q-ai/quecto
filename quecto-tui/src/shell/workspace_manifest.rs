//! Atomic workspace manifest for multi-tab restore (#1465 AC4 / AC5 prep).
//!
//! Maps a workspace id → ordered tabs (session key + name) + active index.
//! Partial/crashed writes must not leave corrupt authoritative state.

#![allow(dead_code)] // P2 manifest library; resume UX is P4
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::atomic_file::write_atomic;
use super::tab_registry::tui_data_dir;

/// On-disk schema version.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

pub const DEFAULT_MANIFEST_FILE_NAME: &str = "workspace-manifests.json";

/// One tab row inside a workspace snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTabEntry {
    pub tab_id: u32,
    pub session_key: Option<String>,
    pub name: Option<String>,
}

/// One workspace's durable tab set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    pub workspace_id: String,
    /// Index into `tabs` for the focused tab at last save.
    pub active_index: usize,
    pub tabs: Vec<WorkspaceTabEntry>,
    pub updated_unix_s: u64,
}

/// Document holding all known workspaces (map-like list for stable JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceManifestStore {
    pub version: u32,
    pub workspaces: Vec<WorkspaceManifest>,
}

impl Default for WorkspaceManifestStore {
    fn default() -> Self {
        Self {
            version: MANIFEST_SCHEMA_VERSION,
            workspaces: Vec::new(),
        }
    }
}

impl WorkspaceManifestStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from disk. Missing or corrupt → empty store (AC4).
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        match serde_json::from_slice::<WorkspaceManifestStore>(&bytes) {
            Ok(mut store) if store.version == MANIFEST_SCHEMA_VERSION => {
                store.workspaces.retain(|w| !w.workspace_id.is_empty());
                for w in &mut store.workspaces {
                    if w.active_index >= w.tabs.len() && !w.tabs.is_empty() {
                        w.active_index = 0;
                    }
                    if w.tabs.is_empty() {
                        w.active_index = 0;
                    }
                }
                store
            }
            _ => Self::default(),
        }
    }

    pub fn store(&self, path: &Path) -> io::Result<()> {
        let mut out = self.clone();
        out.version = MANIFEST_SCHEMA_VERSION;
        let bytes = serde_json::to_vec_pretty(&out)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        write_atomic(path, &bytes)
    }

    pub fn upsert(&mut self, manifest: WorkspaceManifest) {
        if let Some(existing) = self
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == manifest.workspace_id)
        {
            *existing = manifest;
        } else {
            self.workspaces.push(manifest);
        }
    }

    pub fn get(&self, workspace_id: &str) -> Option<&WorkspaceManifest> {
        self.workspaces
            .iter()
            .find(|w| w.workspace_id == workspace_id)
    }

    pub fn remove(&mut self, workspace_id: &str) -> bool {
        let before = self.workspaces.len();
        self.workspaces.retain(|w| w.workspace_id != workspace_id);
        self.workspaces.len() != before
    }
}

pub fn default_manifest_path() -> PathBuf {
    tui_data_dir().join(DEFAULT_MANIFEST_FILE_NAME)
}

#[cfg(test)]
#[path = "workspace_manifest_tests.rs"]
mod workspace_manifest_tests;
