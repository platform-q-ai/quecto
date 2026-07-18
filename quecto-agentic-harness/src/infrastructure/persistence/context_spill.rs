// FileContextSpillStore: JSONL-based spill file for context pruning.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::domain::error::DomainError;
use crate::domain::session::{
    ContextSpillStore, SpillEntry, SpillIndex, SpillIndexList, SpillPresence,
};

/// JSONL-based spill store for context pruning.
///
/// Stores spilled tool outputs as one JSON object per line, append-only.
/// Maintains an in-memory index cache keyed by session to avoid re-reading
/// and re-parsing the JSONL file on every `list_entries()` call (#375).
///
/// The cache uses `Arc<Vec<SpillIndex>>` so that `list_entries()` returns
/// a cheap `Arc::clone()` instead of deep-cloning every `SpillIndex`.
pub struct FileContextSpillStore {
    base_dir: PathBuf,
    /// In-memory index cache: session_key → cached SpillIndex entries.
    /// Populated incrementally on `append()` and seeded from disk on
    /// cold-start `list_entries()`. Invalidated on `clear()`.
    index_cache: RwLock<HashMap<String, Arc<Vec<SpillIndex>>>>,
}

impl std::fmt::Debug for FileContextSpillStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileContextSpillStore")
            .field("base_dir", &self.base_dir)
            .finish()
    }
}

/// On-disk representation of a spill entry (serde-compatible).
#[derive(Serialize, Deserialize)]
struct SpillRecord {
    id: String,
    tool: String,
    input_preview: String,
    tokens: usize,
    content: String,
}

impl From<&SpillEntry> for SpillRecord {
    fn from(e: &SpillEntry) -> Self {
        Self {
            id: e.id.clone(),
            tool: e.tool.clone(),
            input_preview: e.input_preview.clone(),
            tokens: e.tokens,
            content: e.content.clone(),
        }
    }
}

impl From<SpillRecord> for SpillEntry {
    fn from(r: SpillRecord) -> Self {
        Self {
            id: r.id,
            tool: r.tool,
            input_preview: r.input_preview,
            tokens: r.tokens,
            content: r.content,
        }
    }
}

impl From<&SpillRecord> for SpillIndex {
    fn from(r: &SpillRecord) -> Self {
        Self {
            id: r.id.clone(),
            tool: r.tool.clone(),
            input_preview: r.input_preview.clone(),
            tokens: r.tokens,
        }
    }
}

impl FileContextSpillStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            index_cache: RwLock::new(HashMap::new()),
        }
    }

    fn spill_path(&self, session_key: &str) -> PathBuf {
        spill_path_for(&self.base_dir, session_key)
    }

    /// Best-effort synchronous removal of a session's on-disk spill file
    /// (plus its parent directory when that leaves it empty).
    ///
    /// Privacy scrub for ephemeral runs (PR #1048 security review): the spill
    /// writers deliberately persist `--no-session` content under the sanitized
    /// empty-key path so in-run `recall()` stubs stay resolvable, so ephemeral
    /// interface paths (one-shot CLI, UDS, REPL) call this at run end to
    /// guarantee that content does not outlive the run. Synchronous and
    /// instance-free so exit paths without a live runtime or store handle can
    /// scrub. Best-effort only: all ephemeral runs share the empty-key path,
    /// so a concurrent ephemeral run's entries may be scrubbed early (the same
    /// pre-existing shared-file caveat as the writers themselves).
    pub fn scrub_session_spill_sync(base_dir: &Path, session_key: &str) {
        let path = spill_path_for(base_dir, session_key);
        let _ = std::fs::remove_file(&path);
        if let Some(dir) = path.parent() {
            // Only succeeds when the directory is now empty — never removes
            // unrelated session files.
            let _ = std::fs::remove_dir(dir);
        }
    }
}

fn spill_path_for(base_dir: &Path, session_key: &str) -> PathBuf {
    base_dir
        .join("sessions")
        .join(super::filename::sanitize_session_key(session_key))
        .join("spill.jsonl")
}

/// Lightweight index record for list_entries — avoids deserializing content.
#[derive(Deserialize)]
struct SpillIndexRecord {
    id: String,
    tool: String,
    input_preview: String,
    tokens: usize,
    // content field is intentionally omitted to skip deserialization
}

/// Read the raw JSONL content from a spill file.
async fn read_spill_content(path: &Path) -> Result<String, DomainError> {
    match tokio::fs::read_to_string(path).await {
        Ok(c) => Ok(c),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(DomainError::Session(format!(
            "failed to read spill file: {}",
            e
        ))),
    }
}

/// Read and parse spill index records from a JSONL file (no content field).
async fn read_spill_index_records(path: &Path) -> Result<Vec<SpillIndexRecord>, DomainError> {
    let content = read_spill_content(path).await?;
    Ok(parse_jsonl::<SpillIndexRecord>(&content))
}

/// Parse JSONL content into a vector of deserialized records.
fn parse_jsonl<T: serde::de::DeserializeOwned>(content: &str) -> Vec<T> {
    let mut records = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(trimmed) {
            Ok(rec) => records.push(rec),
            Err(e) => {
                tracing::warn!(
                    target: "context_prune",
                    error = %e,
                    "skipping corrupt spill entry"
                );
            }
        }
    }
    records
}

impl ContextSpillStore for FileContextSpillStore {
    fn append(
        &self,
        session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.spill_path(session_key);
        let record = SpillRecord::from(entry);
        let session_key = session_key.to_string();
        Box::pin(async move {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    DomainError::Session(format!("failed to create spill directory: {}", e))
                })?;
            }

            let mut line =
                serde_json::to_string(&record).map_err(|e| DomainError::Session(e.to_string()))?;
            line.push('\n');

            // True append: open in append mode and write directly.
            // No read-modify-write cycle, no TOCTOU race.
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|e| DomainError::Session(format!("failed to open spill file: {}", e)))?;

            file.write_all(line.as_bytes())
                .await
                .map_err(|e| DomainError::Session(format!("failed to write spill entry: {}", e)))?;

            file.flush()
                .await
                .map_err(|e| DomainError::Session(format!("failed to flush spill file: {}", e)))?;

            // Update in-memory index cache (only if already populated).
            // If no cache entry exists, we skip — the next list_entries()
            // call will seed the cache from disk including this new entry.
            // This avoids creating a partial cache that misses prior entries.
            let index = SpillIndex {
                id: record.id,
                tool: record.tool,
                input_preview: record.input_preview,
                tokens: record.tokens,
            };
            let mut cache = self.index_cache.write().await;
            if let Some(existing) = cache.get_mut(&session_key) {
                Arc::make_mut(existing).push(index);
            }

            Ok(())
        })
    }

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let path = self.spill_path(session_key);
        let session_key_owned = session_key.to_string();
        let id = id.to_string();
        Box::pin(async move {
            // Quick check: if index cache is populated and doesn't contain
            // this ID, skip disk I/O entirely.
            if let Some(cached) = self.index_cache.read().await.get(&session_key_owned) {
                if !cached.iter().any(|e| e.id == id) {
                    return Ok(None);
                }
            }

            let content = read_spill_content(&path).await?;
            // Line-by-line scan with early exit: only deserialize lines
            // that contain the target ID as a substring (cheap string check
            // before expensive JSON parse).  Stops at the first match.
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Quick substring filter — avoids deserializing lines that
                // obviously don't contain the target ID.
                if !trimmed.contains(&id) {
                    continue;
                }
                match serde_json::from_str::<SpillRecord>(trimmed) {
                    Ok(rec) if rec.id == id => return Ok(Some(rec.into())),
                    Ok(_) => {} // substring matched in content, not id
                    Err(e) => {
                        tracing::warn!(
                            target: "context_prune",
                            error = %e,
                            "skipping corrupt spill entry during recall"
                        );
                    }
                }
            }
            Ok(None)
        })
    }

    fn list_entries(&self, session_key: &str) -> SpillIndexList<'_> {
        let path = self.spill_path(session_key);
        let session_key = session_key.to_string();
        Box::pin(async move {
            // Fast path: return cached index if available (cheap Arc clone)
            {
                let cache = self.index_cache.read().await;
                if let Some(cached) = cache.get(&session_key) {
                    return Ok(cached.clone());
                }
            }

            // Cold start: read from disk and populate cache.
            // Hold write lock for the full operation to prevent TOCTOU
            // races with concurrent cold-start callers or append().
            let records = read_spill_index_records(&path).await?;
            let entries: Vec<SpillIndex> = records
                .into_iter()
                .map(|r| SpillIndex {
                    id: r.id,
                    tool: r.tool,
                    input_preview: r.input_preview,
                    tokens: r.tokens,
                })
                .collect();

            let arc = Arc::new(entries);
            self.index_cache
                .write()
                .await
                .entry(session_key)
                .or_insert_with(|| arc.clone());

            Ok(arc)
        })
    }

    fn has_entries(&self, session_key: &str) -> SpillPresence<'_> {
        let path = self.spill_path(session_key);
        Box::pin(async move {
            // Presence must use the same corrupt-line-skipping semantics as
            // list_entries(), otherwise a torn append can advertise memory
            // that recall("list") cannot discover.
            Ok(!read_spill_index_records(&path).await?.is_empty())
        })
    }

    fn clear(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.spill_path(session_key);
        let session_key = session_key.to_string();
        Box::pin(async move {
            // Invalidate cache regardless of disk state
            self.index_cache.write().await.remove(&session_key);

            // Check if the file exists — if not, nothing to clear.
            match tokio::fs::metadata(&path).await {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => {
                    return Err(DomainError::Session(format!(
                        "failed to stat spill file: {}",
                        e
                    )));
                }
                Ok(_) => {}
            }

            // Atomic clear: write empty content to a temp file then rename over the target.
            // This avoids a race window where a concurrent append() could interleave with
            // a truncate-in-place (which O_TRUNC would cause).
            let parent = path.parent().ok_or_else(|| {
                DomainError::Session("spill path has no parent directory".to_string())
            })?;

            // Use the same directory so rename() is atomic (same filesystem).
            let tmp_path = parent.join(format!(".spill-clear-{}.tmp", uuid::Uuid::new_v4()));

            tokio::fs::write(&tmp_path, b"").await.map_err(|e| {
                DomainError::Session(format!("failed to write temp clear file: {}", e))
            })?;

            tokio::fs::rename(&tmp_path, &path).await.map_err(|e| {
                DomainError::Session(format!("failed to atomically clear spill file: {}", e))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_entry() -> SpillEntry {
        SpillEntry {
            id: "turn1:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "echo hello".to_string(),
            tokens: 100,
            content: "hello\n".to_string(),
        }
    }

    #[test]
    fn debug_names_base_dir_without_dumping_cache_contents() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().join("spill-base"));
        let debug = format!("{store:?}");
        assert!(
            debug.contains("spill-base"),
            "debug should identify the store base dir: {debug}"
        );
        assert!(
            !debug.contains("index_cache"),
            "debug must not dump cached spill ids/content: {debug}"
        );
    }

    #[tokio::test]
    async fn list_entries_seeds_empty_cache_and_append_extends_it() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());
        let initial = store.list_entries("new-session").await.unwrap();
        assert!(initial.is_empty());
        let mut entry = test_entry();
        entry.id = "turn9:read:0".into();
        store.append("new-session", &entry).await.unwrap();
        let entries = store.list_entries("new-session").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "turn9:read:0");
        assert_eq!(entries[0].tool, "bash");
        assert_eq!(entries[0].tokens, 100);
    }

    #[tokio::test]
    async fn recall_cache_miss_short_circuits_even_if_disk_contains_id() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());
        store.append("session", &test_entry()).await.unwrap();
        assert_eq!(store.list_entries("session").await.unwrap().len(), 1);
        let mut hidden = test_entry();
        hidden.id = "turn-hidden:bash:0".into();
        let path = store.spill_path("session");
        let mut line = serde_json::to_string(&SpillRecord::from(&hidden)).unwrap();
        line.push('\n');
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .unwrap();
        file.write_all(line.as_bytes()).await.unwrap();
        file.flush().await.unwrap();
        let recalled = store.recall("session", "turn-hidden:bash:0").await.unwrap();
        assert!(
            recalled.is_none(),
            "populated index cache should avoid scanning IDs not present in it"
        );
    }

    #[tokio::test]
    async fn test_append_and_recall() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());
        let entry = test_entry();

        store.append("test-session", &entry).await.unwrap();

        let recalled = store.recall("test-session", "turn1:bash:0").await.unwrap();
        assert!(recalled.is_some());
        let recalled = recalled.unwrap();
        assert_eq!(recalled.id, "turn1:bash:0");
        assert_eq!(recalled.content, "hello\n");
    }

    #[tokio::test]
    async fn test_recall_not_found() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        let recalled = store.recall("test-session", "nonexistent").await.unwrap();
        assert!(recalled.is_none());
    }

    #[tokio::test]
    async fn test_list_entries() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        let entry1 = SpillEntry {
            id: "turn1:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "echo hello".to_string(),
            tokens: 100,
            content: "hello\n".to_string(),
        };
        let entry2 = SpillEntry {
            id: "turn2:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "ls -la".to_string(),
            tokens: 200,
            content: "total 0\n".to_string(),
        };

        store.append("test-session", &entry1).await.unwrap();
        store.append("test-session", &entry2).await.unwrap();

        let entries = store.list_entries("test-session").await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "turn1:bash:0");
        assert_eq!(entries[1].id, "turn2:bash:0");
        // Index entries should not have content
    }

    #[tokio::test]
    async fn test_sanitize_session_key() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());
        let entry = test_entry();

        // Session keys with special characters should work (colon is sanitized to _)
        store.append("telegram:12345", &entry).await.unwrap();

        let recalled = store
            .recall("telegram:12345", "turn1:bash:0")
            .await
            .unwrap();
        assert!(recalled.is_some());
    }

    #[tokio::test]
    async fn test_clear_truncates_spill_file() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());
        let entry = test_entry();

        store.append("test-session", &entry).await.unwrap();

        // Verify entry is present before clearing
        let entries = store.list_entries("test-session").await.unwrap();
        assert_eq!(entries.len(), 1);

        // Clear the spill
        store.clear("test-session").await.unwrap();

        // Verify the file is now empty
        let entries_after = store.list_entries("test-session").await.unwrap();
        assert!(entries_after.is_empty());
    }

    #[tokio::test]
    async fn test_has_entries_ignores_a_torn_write_with_no_parseable_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileContextSpillStore::new(dir.path().to_path_buf());
        let path = store.spill_path("session");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, br#"{"id":"turn1:bash:0""#)
            .await
            .unwrap();

        assert!(store.list_entries("session").await.unwrap().is_empty());
        assert!(
            !store.has_entries("session").await.unwrap(),
            "presence must agree with the parse-based index"
        );
    }

    #[tokio::test]
    async fn test_has_entries_tracks_append_and_clear_without_loading_index() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileContextSpillStore::new(dir.path().to_path_buf());
        assert!(!store.has_entries("session").await.unwrap());
        store
            .append(
                "session",
                &SpillEntry {
                    id: "turn1:bash:0".into(),
                    tool: "bash".into(),
                    input_preview: "echo hi".into(),
                    tokens: 2,
                    content: "hi".into(),
                },
            )
            .await
            .unwrap();
        assert!(store.has_entries("session").await.unwrap());
        store.clear("session").await.unwrap();
        assert!(!store.has_entries("session").await.unwrap());
    }

    #[tokio::test]
    async fn test_clear_nonexistent_file_is_noop() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        // Clearing a non-existent session's spill file should not error
        let result = store.clear("ghost-session").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_entries_uses_cache_after_append() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        let entry = test_entry();
        store.append("cached-session", &entry).await.unwrap();

        // Seed the cache via list_entries (simulates agent loop startup)
        let initial = store.list_entries("cached-session").await.unwrap();
        assert_eq!(initial.len(), 1);

        // Append a second entry (updates cache)
        let entry2 = SpillEntry {
            id: "turn2:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "ls".to_string(),
            tokens: 50,
            content: "file.txt\n".to_string(),
        };
        store.append("cached-session", &entry2).await.unwrap();

        // Delete the spill file behind the store's back
        let spill_path = store.spill_path("cached-session");
        tokio::fs::remove_file(&spill_path).await.unwrap();

        // list_entries should still return both entries from cache
        let entries = store.list_entries("cached-session").await.unwrap();
        assert_eq!(
            entries.len(),
            2,
            "list_entries should return cached entries even after disk file is deleted"
        );
        assert_eq!(entries[0].id, "turn1:bash:0");
        assert_eq!(entries[1].id, "turn2:bash:0");
    }

    #[tokio::test]
    async fn test_clear_invalidates_cache() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        let entry = test_entry();
        store.append("clear-test", &entry).await.unwrap();

        // Verify cached
        let entries = store.list_entries("clear-test").await.unwrap();
        assert_eq!(entries.len(), 1);

        // Clear should invalidate cache
        store.clear("clear-test").await.unwrap();

        let entries = store.list_entries("clear-test").await.unwrap();
        assert!(entries.is_empty(), "cache should be cleared after clear()");
    }

    #[tokio::test]
    async fn test_recall_finds_entry_among_many() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        // Append 10 entries with distinct content
        for i in 0..10 {
            let entry = SpillEntry {
                id: format!("turn{}:bash:0", i + 1),
                tool: "bash".to_string(),
                input_preview: format!("cmd-{}", i + 1),
                tokens: 100,
                content: format!("output-{}-{}", i + 1, "x".repeat(1000)),
            };
            store.append("recall-test", &entry).await.unwrap();
        }

        // Recall the 5th entry
        let recalled = store.recall("recall-test", "turn5:bash:0").await.unwrap();
        assert!(recalled.is_some(), "should find turn5:bash:0");
        let entry = recalled.unwrap();
        assert_eq!(entry.id, "turn5:bash:0");
        assert!(
            entry.content.starts_with("output-5-"),
            "content should match the 5th entry"
        );

        // Recall nonexistent
        let missing = store.recall("recall-test", "turn99:bash:0").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn scrub_session_spill_sync_removes_ephemeral_spill_file_and_dir() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());
        // Ephemeral runs spill under the sanitized empty key.
        store.append("", &test_entry()).await.unwrap();
        let path = store.spill_path("");
        assert!(path.exists(), "positive control: the spill file must exist");

        FileContextSpillStore::scrub_session_spill_sync(tmp.path(), "");

        assert!(
            !path.exists(),
            "ephemeral spill content must not outlive the run (PR #1048 security review)"
        );
        assert!(
            !path.parent().unwrap().exists(),
            "the emptied ephemeral session dir must be removed too"
        );
    }

    #[test]
    fn scrub_session_spill_sync_is_a_noop_when_nothing_was_spilled() {
        let tmp = TempDir::new().unwrap();
        // Must not panic or create anything when no spill file exists.
        FileContextSpillStore::scrub_session_spill_sync(tmp.path(), "");
        assert!(!tmp.path().join("sessions").exists());
    }

    #[tokio::test]
    async fn test_recall_handles_id_substring_in_content() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        // Entry whose content contains another entry's ID as a substring
        let entry1 = SpillEntry {
            id: "turn1:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "echo".to_string(),
            tokens: 50,
            content: "Use recall(\"turn2:bash:0\") to see the other output".to_string(),
        };
        let entry2 = SpillEntry {
            id: "turn2:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "ls".to_string(),
            tokens: 50,
            content: "actual output".to_string(),
        };
        store.append("substr-test", &entry1).await.unwrap();
        store.append("substr-test", &entry2).await.unwrap();

        // Recalling turn2 should return entry2, not entry1 (even though
        // entry1's content contains "turn2:bash:0" as a substring)
        let recalled = store.recall("substr-test", "turn2:bash:0").await.unwrap();
        assert!(recalled.is_some());
        let entry = recalled.unwrap();
        assert_eq!(entry.id, "turn2:bash:0");
        assert_eq!(entry.content, "actual output");
    }
}
