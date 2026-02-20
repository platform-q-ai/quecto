// Onboarding: create config file, workspace directory, template files.

use std::path::{Path, PathBuf};

/// Result of the onboard operation.
#[derive(Debug)]
pub struct OnboardResult {
    pub config_path: PathBuf,
    pub workspace_path: PathBuf,
    pub already_existed: bool,
}

/// Run the onboarding process: create config file, workspace dir, and template files.
///
/// `base_dir` is the base directory (e.g. `~/.quecto` or a temp dir in tests).
pub fn run_onboard(base_dir: &Path) -> Result<OnboardResult, OnboardError> {
    let config_path = base_dir.join("config.json");
    let workspace_path = base_dir.join("workspace");

    // Check if config already exists
    if config_path.exists() {
        return Ok(OnboardResult {
            config_path,
            workspace_path,
            already_existed: true,
        });
    }

    // Create base directory if needed
    std::fs::create_dir_all(base_dir).map_err(|e| OnboardError::Io("base dir".into(), e))?;

    // Write default config (empty JSON — all fields use serde defaults)
    std::fs::write(&config_path, "{}\n")
        .map_err(|e| OnboardError::Io(config_path.display().to_string(), e))?;

    // Create workspace directory
    std::fs::create_dir_all(&workspace_path)
        .map_err(|e| OnboardError::Io(workspace_path.display().to_string(), e))?;

    // Write template files
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

    for (filename, content) in &templates {
        let file_path = workspace_path.join(filename);
        std::fs::write(&file_path, content)
            .map_err(|e| OnboardError::Io(file_path.display().to_string(), e))?;
    }

    Ok(OnboardResult {
        config_path,
        workspace_path,
        already_existed: false,
    })
}

/// Returns the default quecto base directory (`~/.quecto`).
pub fn default_base_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".quecto"))
}

#[derive(Debug)]
pub enum OnboardError {
    Io(String, std::io::Error),
}

impl std::fmt::Display for OnboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnboardError::Io(path, err) => write!(f, "I/O error for '{}': {}", path, err),
        }
    }
}

impl std::error::Error for OnboardError {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_onboard_creates_config_and_workspace() {
        let tmp = TempDir::new().unwrap();
        let result = run_onboard(tmp.path()).unwrap();
        assert!(!result.already_existed);
        assert!(result.config_path.exists());
        assert!(result.workspace_path.exists());
        assert!(result.workspace_path.is_dir());
    }

    #[test]
    fn test_onboard_creates_template_files() {
        let tmp = TempDir::new().unwrap();
        run_onboard(tmp.path()).unwrap();
        let ws = tmp.path().join("workspace");
        for name in &["AGENTS.md", "IDENTITY.md", "SOUL.md", "TOOLS.md", "USER.md"] {
            assert!(ws.join(name).exists(), "{} should exist", name);
        }
    }

    #[test]
    fn test_onboard_existing_config_reports_already_existed() {
        let tmp = TempDir::new().unwrap();
        // Create a config file first
        std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
        let result = run_onboard(tmp.path()).unwrap();
        assert!(result.already_existed);
    }

    #[test]
    fn test_onboard_default_config_has_sensible_defaults() {
        let tmp = TempDir::new().unwrap();
        run_onboard(tmp.path()).unwrap();
        let content = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let config: crate::infrastructure::config::Config = serde_json::from_str(&content).unwrap();
        assert_eq!(config.agents.defaults.model, "gpt-4");
        assert_eq!(config.agents.defaults.max_tokens, 8192);
        assert!((config.agents.defaults.temperature - 0.7).abs() < f32::EPSILON);
        assert!(config.agents.defaults.restrict_to_workspace);
    }
}
