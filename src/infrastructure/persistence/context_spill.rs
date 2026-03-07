// FileContextSpillStore: JSONL-based spill file for context pruning.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::domain::error::DomainError;
use crate::domain::session::{ContextSpillStore, SpillEntry, SpillIndex};

/// JSONL-based spill store for context pruning.
/// Stores spilled tool outputs as one JSON object per line, append-only.
pub struct FileContextSpillStore {
    base_dir: PathBuf,
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
        Self { base_dir }
    }

    fn spill_path(&self, session_key: &str) -> PathBuf {
        self.base_dir
            .join("sessions")
            .join(super::filename::sanitize_session_key(session_key))
            .join("spill.jsonl")
    }
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

/// Read and parse all spill records from a JSONL file (full content).
async fn read_spill_records(path: &Path) -> Result<Vec<SpillRecord>, DomainError> {
    let content = read_spill_content(path).await?;
    Ok(parse_jsonl::<SpillRecord>(&content))
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

            Ok(())
        })
    }

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let path = self.spill_path(session_key);
        let id = id.to_string();
        Box::pin(async move {
            let records = read_spill_records(&path).await?;
            Ok(records.into_iter().find(|r| r.id == id).map(Into::into))
        })
    }

    fn list_entries(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SpillIndex>, DomainError>> + Send + '_>> {
        let path = self.spill_path(session_key);
        Box::pin(async move {
            let records = read_spill_index_records(&path).await?;
            Ok(records
                .into_iter()
                .map(|r| SpillIndex {
                    id: r.id,
                    tool: r.tool,
                    input_preview: r.input_preview,
                    tokens: r.tokens,
                })
                .collect())
        })
    }

    fn clear(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.spill_path(session_key);
        Box::pin(async move {
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
    async fn test_clear_nonexistent_file_is_noop() {
        let tmp = TempDir::new().unwrap();
        let store = FileContextSpillStore::new(tmp.path().to_path_buf());

        // Clearing a non-existent session's spill file should not error
        let result = store.clear("ghost-session").await;
        assert!(result.is_ok());
    }
}
