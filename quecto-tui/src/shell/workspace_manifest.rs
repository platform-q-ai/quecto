//! Atomic workspace manifest for multi-tab restore (#1465 AC4 / AC5 prep).
//!
//! Maps a workspace id → ordered tabs (session key + name) + active index.
//! Partial/crashed writes must not leave corrupt authoritative state.

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
    /// Human-recognizable conversation snippet for `/resume` (#1466 fix pass
    /// item 3): the tab's last user message (or session title), so workspace
    /// rows are identifiable by content rather than opaque labels.
    #[serde(default)]
    pub summary: Option<String>,
}

/// One workspace's durable tab set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    /// Stable workspace identity (#1466): a UUID, never cwd-derived.
    pub workspace_id: String,
    /// Human label (#1466): auto-generated at creation, renameable; `/resume`
    /// lists workspaces by label, never by raw id.
    #[serde(default)]
    pub label: String,
    /// Unix seconds when the workspace was last active (#1466): shown in
    /// `/resume` alongside the label.
    #[serde(default)]
    pub last_active_unix_s: u64,
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
    #[cfg_attr(not(test), allow(dead_code))]
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

    /// GC orphaned workspaces (#1466): a workspace is orphaned when none of
    /// its tabs carries a resumable session key AND no registry record exists
    /// for it. Returns the removed workspace ids.
    pub fn gc_orphaned(
        &mut self,
        registry: &crate::shell::tab_registry::TabAgentRegistry,
    ) -> Vec<String> {
        let referenced: std::collections::HashSet<&str> = registry
            .agents
            .iter()
            .filter_map(|a| a.workspace_id.as_deref())
            .collect();
        let mut removed = Vec::new();
        self.workspaces.retain(|ws| {
            let has_session = ws.tabs.iter().any(|t| {
                t.session_key
                    .as_deref()
                    .is_some_and(|k| !k.trim().is_empty())
            });
            let keep = has_session || referenced.contains(ws.workspace_id.as_str());
            if !keep {
                removed.push(ws.workspace_id.clone());
            }
            keep
        });
        removed
    }

    /// Remove a workspace row (lifecycle cleanup / tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn remove(&mut self, workspace_id: &str) -> bool {
        let before = self.workspaces.len();
        self.workspaces.retain(|w| w.workspace_id != workspace_id);
        self.workspaces.len() != before
    }
}

impl WorkspaceManifest {
    /// Label to show in `/resume` (#1466 decision 1): the human label, with a
    /// non-UUID fallback for pre-#1466 manifests persisted without one.
    pub fn display_label(&self) -> String {
        let trimmed = self.label.trim();
        if trimmed.is_empty() {
            "unnamed workspace".to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Best-known last-active instant: the explicit #1466 field, falling back
    /// to the manifest write time for legacy rows.
    pub fn last_active_or_updated_s(&self) -> u64 {
        if self.last_active_unix_s > 0 {
            self.last_active_unix_s
        } else {
            self.updated_unix_s
        }
    }
}

/// Mint a fresh workspace identity (#1466): a hyphenated UUIDv4 string.
pub fn generate_workspace_id() -> String {
    let b = random_bytes_16();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-4{:01x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6] & 0x0f,
        b[7],
        (b[8] & 0x3f) | 0x80,
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// Auto-generate a human workspace label (#1466 decision 1): adjective-noun
/// pairs, friendly and non-UUID; renaming can come later without migration.
pub fn generate_workspace_label() -> String {
    const ADJECTIVES: [&str; 16] = [
        "amber", "brisk", "calm", "deft", "eager", "fresh", "gentle", "hardy", "keen", "lively",
        "mellow", "nimble", "quiet", "steady", "tidy", "vivid",
    ];
    const NOUNS: [&str; 16] = [
        "aspen", "brook", "cedar", "dune", "fjord", "glade", "harbor", "inlet", "juniper", "knoll",
        "lagoon", "meadow", "orchard", "prairie", "ridge", "summit",
    ];
    let b = random_bytes_16();
    format!(
        "{}-{}",
        ADJECTIVES[(b[0] as usize) % ADJECTIVES.len()],
        NOUNS[(b[1] as usize) % NOUNS.len()]
    )
}

/// 16 bytes of randomness: `/dev/urandom` when available, else a time/pid/
/// counter mix (uniqueness-grade, not cryptographic — ids are local keys).
fn random_bytes_16() -> [u8; 16] {
    use std::io::Read;
    let mut buf = [0u8; 16];
    if fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok()
    {
        return buf;
    }
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seed = nanos
        ^ (u64::from(std::process::id()) << 32)
        ^ COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut x = seed;
    for chunk in buf.chunks_mut(8) {
        // splitmix64 step — well distributed even from adjacent seeds.
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
    }
    buf
}

/// Humanised "how long ago" for `/resume` rows (#1466 decision 1).
pub fn relative_age_label(now_unix_s: u64, then_unix_s: u64) -> String {
    let secs = now_unix_s.saturating_sub(then_unix_s);
    match secs {
        0..=59 => "moments ago".to_string(),
        60..=3_599 => format!("{}m ago", secs / 60),
        3_600..=86_399 => format!("{}h ago", secs / 3_600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

pub fn default_manifest_path() -> PathBuf {
    tui_data_dir().join(DEFAULT_MANIFEST_FILE_NAME)
}

#[cfg(test)]
#[path = "workspace_manifest_tests.rs"]
mod workspace_manifest_tests;
