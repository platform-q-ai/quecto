// Onboarding: create config file, workspace directory, template files.

use std::path::PathBuf;

use crate::domain::error::DomainError;
use crate::domain::workspace::OnboardStore;

/// Result of the onboard operation.
#[derive(Debug)]
pub struct OnboardResult {
    pub config_path: PathBuf,
    pub workspace_path: PathBuf,
    pub already_existed: bool,
}

/// Run the onboarding process using an injected store adapter.
pub fn run_onboard(store: &dyn OnboardStore) -> Result<OnboardResult, DomainError> {
    let config_path = store.config_path();
    let workspace_path = store.workspace_path();

    if store.config_exists()? {
        return Ok(OnboardResult {
            config_path,
            workspace_path,
            already_existed: true,
        });
    }

    let templates = [
        (
            "AGENTS.md",
            "# Agents\n\nDefine your agent configurations here.\n",
        ),
        (
            "IDENTITY.md",
            "# Identity\n\nDescribe who this AI assistant is.\n",
        ),
        (
            "SOUL.md",
            "# Soul\n\nDefine the personality and values of your assistant.\n",
        ),
        (
            "TOOLS.md",
            "# Tools\n\nList of available tools and their usage.\n",
        ),
        ("USER.md", "# User\n\nInformation about the user.\n"),
    ];

    store.initialize(&templates)?;

    Ok(OnboardResult {
        config_path,
        workspace_path,
        already_existed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::persistence::workspace_store::FileOnboardStore;
    use tempfile::TempDir;

    #[test]
    fn test_onboard_creates_config_and_workspace() {
        let tmp = TempDir::new().unwrap();
        let store = FileOnboardStore::new(tmp.path());
        let result = run_onboard(&store).unwrap();
        assert!(!result.already_existed);
        assert!(result.config_path.exists());
        assert!(result.workspace_path.exists());
        assert!(result.workspace_path.is_dir());
    }

    #[test]
    fn test_onboard_creates_template_files() {
        let tmp = TempDir::new().unwrap();
        let store = FileOnboardStore::new(tmp.path());
        run_onboard(&store).unwrap();
        let ws = tmp.path().join("workspace");
        for name in &["AGENTS.md", "IDENTITY.md", "SOUL.md", "TOOLS.md", "USER.md"] {
            assert!(ws.join(name).exists(), "{} should exist", name);
        }
    }

    #[test]
    fn test_onboard_existing_config_reports_already_existed() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
        let store = FileOnboardStore::new(tmp.path());
        let result = run_onboard(&store).unwrap();
        assert!(result.already_existed);
    }

    #[test]
    fn test_onboard_default_config_has_sensible_defaults() {
        let tmp = TempDir::new().unwrap();
        let store = FileOnboardStore::new(tmp.path());
        run_onboard(&store).unwrap();
        let content = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let config: crate::infrastructure::config::Config = serde_json::from_str(&content).unwrap();
        assert_eq!(config.agents.defaults.model, "gpt-5.4");
        assert_eq!(config.agents.defaults.max_tokens, 8192);
        assert!((config.agents.defaults.temperature - 0.7).abs() < f32::EPSILON);
        assert!(config.agents.defaults.restrict_to_workspace);
    }
}
