// Memory store: read/write MEMORY.md for long-term agent memory.

use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;

/// Manages the long-term memory file (MEMORY.md) in the workspace.
#[derive(Debug)]
pub struct MemoryStore {
    memory_dir: PathBuf,
}

impl MemoryStore {
    /// Create a new memory store rooted at the given workspace directory.
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            memory_dir: workspace.as_ref().join("memory"),
        }
    }

    /// Path to the MEMORY.md file.
    fn memory_path(&self) -> PathBuf {
        self.memory_dir.join("MEMORY.md")
    }

    /// Read the long-term memory contents.
    /// Returns an empty string if the file doesn't exist.
    pub async fn read(&self) -> Result<String, DomainError> {
        let path = self.memory_path();
        if !path.exists() {
            return Ok(String::new());
        }
        tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| DomainError::Session(format!("failed to read MEMORY.md: {}", e)))
    }

    /// Append a note to the long-term memory file.
    /// Creates the file and parent directories if they don't exist.
    pub async fn append(&self, note: &str) -> Result<(), DomainError> {
        tokio::fs::create_dir_all(&self.memory_dir)
            .await
            .map_err(|e| DomainError::Session(format!("failed to create memory dir: {}", e)))?;

        let path = self.memory_path();
        let existing = if path.exists() {
            tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| DomainError::Session(format!("failed to read MEMORY.md: {}", e)))?
        } else {
            String::new()
        };

        let new_content = if existing.is_empty() {
            format!("# Memory\n\n{}\n", note)
        } else {
            format!("{}\n{}\n", existing.trim_end(), note)
        };

        tokio::fs::write(&path, new_content.as_bytes())
            .await
            .map_err(|e| DomainError::Session(format!("failed to write MEMORY.md: {}", e)))
    }

    /// Write (overwrite) the entire memory file.
    pub async fn write(&self, content: &str) -> Result<(), DomainError> {
        tokio::fs::create_dir_all(&self.memory_dir)
            .await
            .map_err(|e| DomainError::Session(format!("failed to create memory dir: {}", e)))?;

        tokio::fs::write(&self.memory_path(), content.as_bytes())
            .await
            .map_err(|e| DomainError::Session(format!("failed to write MEMORY.md: {}", e)))
    }

    /// Check if the memory file exists.
    pub fn exists(&self) -> bool {
        self.memory_path().exists()
    }
}

/// Load IDENTITY.md from the workspace directory.
/// Returns the content, or an empty string if the file doesn't exist.
pub async fn load_identity(workspace: impl AsRef<Path>) -> Result<String, DomainError> {
    let path = workspace.as_ref().join("IDENTITY.md");
    if !path.exists() {
        return Ok(String::new());
    }
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read IDENTITY.md: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_memory_append_creates_file() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path());

        assert!(!store.exists());
        store.append("First note").await.unwrap();
        assert!(store.exists());

        let content = store.read().await.unwrap();
        assert!(content.contains("First note"));
    }

    #[tokio::test]
    async fn test_memory_append_multiple() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path());

        store.append("Note 1").await.unwrap();
        store.append("Note 2").await.unwrap();

        let content = store.read().await.unwrap();
        assert!(content.contains("Note 1"));
        assert!(content.contains("Note 2"));
    }

    #[tokio::test]
    async fn test_memory_read_empty() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path());

        let content = store.read().await.unwrap();
        assert!(content.is_empty());
    }

    #[tokio::test]
    async fn test_memory_write_overwrites() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path());

        store.append("Old note").await.unwrap();
        store.write("New content only").await.unwrap();

        let content = store.read().await.unwrap();
        assert_eq!(content, "New content only");
        assert!(!content.contains("Old note"));
    }

    #[tokio::test]
    async fn test_load_identity_exists() {
        let tmp = TempDir::new().unwrap();
        let identity_content = "You are Quecto, a helpful assistant";
        std::fs::write(tmp.path().join("IDENTITY.md"), identity_content).unwrap();

        let loaded = load_identity(tmp.path()).await.unwrap();
        assert_eq!(loaded, identity_content);
    }

    #[tokio::test]
    async fn test_load_identity_missing() {
        let tmp = TempDir::new().unwrap();
        let loaded = load_identity(tmp.path()).await.unwrap();
        assert!(loaded.is_empty());
    }
}
