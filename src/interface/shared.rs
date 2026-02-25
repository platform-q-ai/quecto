//! Shared utility functions used by CLI, REPL, and gateway modules.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::application::coding_coordinator::{CodingCoordinator, CoordinatorPolicy};
use crate::domain::coding_ports::CodingJobService;
use crate::domain::skill::SkillLoader;
use crate::infrastructure::auth::credential_store::Credential;
use crate::infrastructure::coding::runtime_adapters::{
    WorkspaceRepoValidator, WorkspaceSkillResolver,
};
use crate::infrastructure::persistence::skill_loader::FileSkillLoader;
use crate::infrastructure::tools::coding_job::CodingJobTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

/// Load all workspace skills and concatenate their non-empty body content.
///
/// Skills without valid YAML frontmatter are silently skipped.
pub fn load_skill_prompt(base_dir: &Path) -> String {
    let workspace = base_dir.join("workspace");
    let loader = FileSkillLoader::new(&workspace);
    let skills = match loader.list() {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    skills
        .iter()
        .filter(|s| !s.content.is_empty())
        .map(|s| s.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Merge skill content with an optional user-provided system prompt.
pub fn merge_prompts(skill_prompt: &str, user_prompt: &Option<String>) -> String {
    match user_prompt {
        Some(up) if !up.is_empty() => format!("{}\n\n{}", skill_prompt, up),
        _ => skill_prompt.to_string(),
    }
}

/// Resolve an API key for a provider from a credential snapshot.
///
/// The credential store snapshot takes priority over the config-file key.
/// Expired credentials are ignored (falls back to config key).
/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
pub fn resolve_api_key(
    config_key: &str,
    creds: &HashMap<String, Credential>,
    provider: &str,
) -> String {
    if let Some(cred) = creds.get(provider) {
        if !cred.is_expired() {
            return cred.token.clone();
        }
    }
    config_key.to_string()
}

/// Check which providers have expired credentials and need re-authentication.
///
/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
pub fn check_provider_readiness(creds: &HashMap<String, Credential>) -> Vec<String> {
    creds
        .values()
        .filter(|c| c.is_expired())
        .map(|c| c.provider.clone())
        .collect()
}

/// Register the `coding_job` tool using real workspace-backed adapters.
pub fn register_coding_job_tool(registry: &mut ToolRegistryImpl, workspace: &Path) {
    let repo_validator = WorkspaceRepoValidator::new(workspace.to_path_buf());
    let skill_resolver = WorkspaceSkillResolver::new(workspace.to_path_buf());
    let coordinator = CodingCoordinator::new(
        repo_validator,
        skill_resolver,
        CoordinatorPolicy {
            skill_denylist: Vec::new(),
            skill_allowlist: Vec::new(),
            // Bound in-memory job retention per coordinator instance.
            max_retained_jobs: Some(512),
        },
    );
    let service: Arc<Mutex<dyn CodingJobService>> = Arc::new(Mutex::new(coordinator));
    registry.register(Arc::new(CodingJobTool::new(service)));
}

/// Coordinator scope policy used by current runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingCoordinatorScopePolicy {
    PerSession,
    Shared,
}

pub fn cli_coding_coordinator_scope() -> CodingCoordinatorScopePolicy {
    CodingCoordinatorScopePolicy::PerSession
}

pub fn gateway_inbound_coding_coordinator_scope() -> CodingCoordinatorScopePolicy {
    CodingCoordinatorScopePolicy::PerSession
}

pub fn gateway_background_coding_coordinator_scope() -> CodingCoordinatorScopePolicy {
    CodingCoordinatorScopePolicy::Shared
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::security::sandbox::Sandbox;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;

    fn frontmatter(name: &str, desc: &str, body: &str) -> String {
        format!("---\nname: {}\ndescription: {}\n---\n{}", name, desc, body)
    }

    #[test]
    fn test_load_skill_prompt_with_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            frontmatter("weather", "Weather forecasts", "Fetch weather data"),
        )
        .unwrap();
        let prompt = load_skill_prompt(tmp.path());
        assert_eq!(prompt, "Fetch weather data");
    }

    #[test]
    fn test_load_skill_prompt_empty_when_no_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prompt = load_skill_prompt(tmp.path());
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_load_skill_prompt_skips_invalid_frontmatter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("bad");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "No frontmatter").unwrap();
        let prompt = load_skill_prompt(tmp.path());
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_merge_prompts_skill_only() {
        let result = merge_prompts("Skill content", &None);
        assert_eq!(result, "Skill content");
    }

    #[test]
    fn test_merge_prompts_skill_and_user() {
        let result = merge_prompts("Skill content", &Some("User prompt".to_string()));
        assert_eq!(result, "Skill content\n\nUser prompt");
    }

    #[test]
    fn test_merge_prompts_skill_with_empty_user() {
        let result = merge_prompts("Skill content", &Some(String::new()));
        assert_eq!(result, "Skill content");
    }

    #[test]
    fn test_register_coding_job_tool_adds_definition() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();
        std::fs::create_dir_all(workspace.join("skills")).unwrap();

        let sandbox = Sandbox::new(Some(workspace.clone()), true);
        let mut registry = ToolRegistryImpl::with_core_tools(workspace.clone(), sandbox);
        register_coding_job_tool(&mut registry, &workspace);

        assert!(
            registry
                .definitions()
                .iter()
                .any(|d| d.name == "coding_job")
        );
    }

    #[test]
    fn test_coding_scope_policy_values() {
        assert_eq!(
            cli_coding_coordinator_scope(),
            CodingCoordinatorScopePolicy::PerSession
        );
        assert_eq!(
            gateway_inbound_coding_coordinator_scope(),
            CodingCoordinatorScopePolicy::PerSession
        );
        assert_eq!(
            gateway_background_coding_coordinator_scope(),
            CodingCoordinatorScopePolicy::Shared
        );
    }
}
