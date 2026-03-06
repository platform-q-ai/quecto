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

    fn write_workspace_file(&self, filename: &str, content: &str) -> Result<(), DomainError> {
        let file_name = std::path::Path::new(filename)
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                DomainError::Other(format!("invalid workspace filename: {}", filename))
            })?;
        if file_name != filename {
            return Err(DomainError::Other(format!(
                "workspace filename must be a basename: {}",
                filename
            )));
        }

        let file_path = self.workspace_path().join(filename);
        std::fs::write(&file_path, content).map_err(|e| {
            DomainError::Other(format!(
                "failed to write workspace file '{}': {}",
                file_path.display(),
                e
            ))
        })
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

    fn initialize(&self, templates: &[(&str, &str)]) -> Result<(), DomainError> {
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

        for (filename, content) in templates {
            self.write_workspace_file(filename, content)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_onboard_store_rejects_non_basename_workspace_file() {
        let tmp = TempDir::new().unwrap();
        let store = FileOnboardStore::new(tmp.path());
        store.initialize(&[]).unwrap();

        let err = store
            .write_workspace_file("nested/file.txt", "data")
            .unwrap_err();
        assert!(err.to_string().contains("basename"));
    }
}
