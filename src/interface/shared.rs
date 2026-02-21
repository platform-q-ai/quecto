//! Shared utility functions used by both CLI and REPL modules.

use std::path::Path;

use crate::domain::skill::SkillLoader;
use crate::infrastructure::persistence::skill_loader::FileSkillLoader;

/// Load all workspace skills and concatenate their non-empty content.
pub fn load_skill_prompt(base_dir: &Path) -> String {
    let workspace = base_dir.join("workspace");
    let global = base_dir.join("global");
    let builtin = base_dir.join("builtin");
    let loader = FileSkillLoader::new(&workspace, &global, &builtin);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_skill_prompt_with_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Fetch weather data").unwrap();
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
    fn test_load_skill_prompt_skips_empty_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("empty");
        std::fs::create_dir_all(&skill_dir).unwrap();
        // No SKILL.md — content will be empty
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
}
