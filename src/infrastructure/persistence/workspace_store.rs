use std::path::PathBuf;

use crate::domain::error::DomainError;
use crate::domain::workspace::{HeartbeatTaskSource, OnboardStore};

#[derive(Debug, Clone)]
pub struct FileHeartbeatTaskSource {
    workspace: PathBuf,
}

impl FileHeartbeatTaskSource {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }
}

impl HeartbeatTaskSource for FileHeartbeatTaskSource {
    fn read_heartbeat_md(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<String>, DomainError>> + Send + '_>,
    > {
        Box::pin(async move {
            let path = self.workspace.join("HEARTBEAT.md");
            match tokio::fs::read_to_string(&path).await {
                Ok(content) => Ok(Some(content)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(DomainError::Other(format!(
                    "failed to read HEARTBEAT.md: {}",
                    e
                ))),
            }
        })
    }
}

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

    fn create_base_dir(&self) -> Result<(), DomainError> {
        std::fs::create_dir_all(&self.base_dir)
            .map_err(|e| DomainError::Other(format!("failed to create base dir: {}", e)))
    }

    fn write_default_config(&self) -> Result<(), DomainError> {
        let config_path = self.config_path();
        std::fs::write(&config_path, "{}\n").map_err(|e| {
            DomainError::Other(format!(
                "failed to write default config '{}': {}",
                config_path.display(),
                e
            ))
        })
    }

    fn create_workspace_dir(&self) -> Result<(), DomainError> {
        let workspace_path = self.workspace_path();
        std::fs::create_dir_all(&workspace_path).map_err(|e| {
            DomainError::Other(format!(
                "failed to create workspace '{}': {}",
                workspace_path.display(),
                e
            ))
        })
    }

    fn write_workspace_file(&self, filename: &str, content: &str) -> Result<(), DomainError> {
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
