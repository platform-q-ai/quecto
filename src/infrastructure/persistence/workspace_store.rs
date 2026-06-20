use std::path::PathBuf;

use crate::domain::error::DomainError;
use crate::domain::workspace::OnboardStore;

#[derive(Debug, Clone)]
pub struct FileOnboardStore {
    base_dir: PathBuf,
}

impl FileOnboardStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
}

impl OnboardStore for FileOnboardStore {
    fn config_path(&self) -> PathBuf {
        self.base_dir.join("config.json")
    }

    fn workspace_path(&self) -> PathBuf {
        self.base_dir.join("workspace")
    }

    fn config_exists(&self) -> Result<bool, DomainError> {
        Ok(self.config_path().exists())
    }

    fn initialize(&self) -> Result<(), DomainError> {
        std::fs::create_dir_all(&self.base_dir)
            .map_err(|e| DomainError::Other(format!("failed to create base dir: {}", e)))?;

        let config_path = self.config_path();
        std::fs::write(&config_path, "{}\n").map_err(|e| {
            DomainError::Other(format!(
                "failed to write default config '{}': {}",
                config_path.display(),
                e
            ))
        })?;

        let workspace_path = self.workspace_path();
        std::fs::create_dir_all(&workspace_path).map_err(|e| {
            DomainError::Other(format!(
                "failed to create workspace '{}': {}",
                workspace_path.display(),
                e
            ))
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn paths_are_under_base_dir() {
        let tmp = TempDir::new().unwrap();
        let store = FileOnboardStore::new(tmp.path());
        assert_eq!(store.config_path(), tmp.path().join("config.json"));
        assert_eq!(store.workspace_path(), tmp.path().join("workspace"));
    }

    #[test]
    fn initialize_creates_config_and_workspace() {
        let tmp = TempDir::new().unwrap();
        let store = FileOnboardStore::new(tmp.path());
        assert!(!store.config_exists().unwrap(), "no config before init");

        store.initialize().unwrap();

        assert!(store.config_exists().unwrap(), "config exists after init");
        assert!(store.workspace_path().is_dir(), "workspace dir created");
        // The default config is valid JSON the config layer can parse.
        let content = std::fs::read_to_string(store.config_path()).unwrap();
        let _: serde_json::Value = serde_json::from_str(&content).unwrap();
    }
}
