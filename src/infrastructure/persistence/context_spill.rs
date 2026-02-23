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
            .join(sanitize_filename(session_key))
            .join("spill.jsonl")
    }
}

/// Sanitize a session key for use as a directory name.
/// Replaces characters that are invalid in filenames.
fn sanitize_filename(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

/// Read and parse all spill records from a JSONL file.
async fn read_spill_records(path: &Path) -> Result<Vec<SpillRecord>, DomainError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => {
            return Err(DomainError::Session(format!(
                "failed to read spill file: {}",
                e
            )));
        }
    };

    let mut records = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<SpillRecord>(trimmed) {
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
    Ok(records)
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

            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|e| DomainError::Session(format!("failed to open spill file: {}", e)))?;

            tokio::fs::write(&path, {
                // Read existing content and append
                let mut existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                existing.push_str(&line);
                existing
            })
            .await
            .map_err(|e| DomainError::Session(format!("failed to write spill entry: {}", e)))?;

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
            let records = read_spill_records(&path).await?;
            Ok(records.iter().map(SpillIndex::from).collect())
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

        // Session keys with special characters should work
        store.append("telegram:12345", &entry).await.unwrap();

        let recalled = store
            .recall("telegram:12345", "turn1:bash:0")
            .await
            .unwrap();
        assert!(recalled.is_some());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("simple"), "simple");
        assert_eq!(sanitize_filename("telegram:12345"), "telegram_12345");
        assert_eq!(sanitize_filename("a/b\\c"), "a_b_c");
    }
}
