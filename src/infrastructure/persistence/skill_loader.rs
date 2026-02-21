// Skill loader: loads skills from workspace/skills/ with YAML frontmatter.

use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;
use crate::domain::skill::{
    Skill, SkillFrontmatter, SkillLoader, SkillSource, is_valid_skill_name, split_skill_md,
    validate_frontmatter,
};

/// Maximum SKILL.md file size (256 KB). Files larger than this are
/// skipped to prevent OOM from symlinks or oversized files.
const MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;

/// File-based skill loader that reads skills from workspace/skills/.
///
/// Each skill is a directory containing a `SKILL.md` file with YAML
/// frontmatter (name + description required). Skills with missing or
/// invalid frontmatter, or name-directory mismatches, are skipped.
#[derive(Debug)]
pub struct FileSkillLoader {
    skills_dir: PathBuf,
}

impl FileSkillLoader {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            skills_dir: workspace.as_ref().join("skills"),
        }
    }

    /// Parse YAML frontmatter from a raw SKILL.md string.
    /// Returns `None` if the YAML is invalid or missing required fields.
    fn parse_frontmatter(raw: &str) -> Option<(SkillFrontmatter, String)> {
        let (yaml_block, body) = split_skill_md(raw)?;
        let fm: SkillFrontmatter = serde_yaml::from_str(yaml_block).ok()?;
        if !validate_frontmatter(&fm) {
            return None;
        }
        Some((fm, body))
    }

    /// Try to load a single skill from a directory entry.
    /// Returns `None` if the skill is invalid (no SKILL.md,
    /// bad frontmatter, name mismatch, invalid name format,
    /// file too large).
    fn try_load_skill(skill_dir: &Path) -> Option<Skill> {
        let dir_name = skill_dir.file_name()?.to_string_lossy().to_string();
        let skill_md_path = skill_dir.join("SKILL.md");

        // Check file size before reading
        let meta = std::fs::metadata(&skill_md_path).ok()?;
        if meta.len() > MAX_SKILL_FILE_BYTES {
            return None;
        }

        let raw = std::fs::read_to_string(&skill_md_path).ok()?;
        let (fm, body) = Self::parse_frontmatter(&raw)?;

        // Name must be valid format
        if !is_valid_skill_name(&fm.name) {
            return None;
        }

        // Name must match directory name
        if fm.name != dir_name {
            return None;
        }

        Some(Skill {
            name: fm.name,
            description: fm.description,
            content: body,
            source: SkillSource::Workspace,
        })
    }
}

impl SkillLoader for FileSkillLoader {
    fn list(&self) -> Result<Vec<Skill>, DomainError> {
        let mut skills = Vec::new();
        if !self.skills_dir.is_dir() {
            return Ok(skills);
        }
        if let Ok(entries) = std::fs::read_dir(&self.skills_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(skill) = Self::try_load_skill(&entry.path()) {
                        skills.push(skill);
                    }
                }
            }
        }
        Ok(skills)
    }

    fn load(&self, name: &str) -> Result<Option<Skill>, DomainError> {
        // Validate name before constructing path (defense-in-depth)
        if !is_valid_skill_name(name) {
            return Ok(None);
        }
        let skill_dir = self.skills_dir.join(name);
        if skill_dir.is_dir() {
            Ok(Self::try_load_skill(&skill_dir))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_skill(base: &Path, name: &str, content: &str) {
        let skill_dir = base.join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    fn frontmatter(name: &str, desc: &str, body: &str) -> String {
        format!("---\nname: {}\ndescription: {}\n---\n{}", name, desc, body)
    }

    #[test]
    fn test_list_workspace_skills() {
        let ws = TempDir::new().unwrap();
        create_skill(
            ws.path(),
            "weather",
            &frontmatter("weather", "Weather forecasts", "Weather body"),
        );

        let loader = FileSkillLoader::new(ws.path());
        let skills = loader.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "weather");
        assert_eq!(skills[0].description, "Weather forecasts");
        assert_eq!(skills[0].content, "Weather body");
        assert_eq!(skills[0].source, SkillSource::Workspace);
    }

    #[test]
    fn test_load_specific_skill() {
        let ws = TempDir::new().unwrap();
        create_skill(
            ws.path(),
            "weather",
            &frontmatter("weather", "Weather", "Content here"),
        );

        let loader = FileSkillLoader::new(ws.path());
        let skill = loader.load("weather").unwrap().unwrap();
        assert_eq!(skill.name, "weather");
        assert_eq!(skill.content, "Content here");
    }

    #[test]
    fn test_load_nonexistent_skill() {
        let ws = TempDir::new().unwrap();
        let loader = FileSkillLoader::new(ws.path());
        assert!(loader.load("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_load_rejects_invalid_name() {
        let ws = TempDir::new().unwrap();
        let loader = FileSkillLoader::new(ws.path());
        // Path traversal attempt
        assert!(loader.load("../etc").unwrap().is_none());
        assert!(loader.load("My_Skill").unwrap().is_none());
    }

    #[test]
    fn test_empty_dirs() {
        let ws = TempDir::new().unwrap();
        let loader = FileSkillLoader::new(ws.path());
        assert!(loader.list().unwrap().is_empty());
    }

    #[test]
    fn test_skill_without_skill_md_is_skipped() {
        let ws = TempDir::new().unwrap();
        std::fs::create_dir_all(ws.path().join("skills").join("empty")).unwrap();

        let loader = FileSkillLoader::new(ws.path());
        assert!(loader.list().unwrap().is_empty());
    }

    #[test]
    fn test_skill_without_frontmatter_is_skipped() {
        let ws = TempDir::new().unwrap();
        create_skill(ws.path(), "bad-skill", "Just plain text, no frontmatter");

        let loader = FileSkillLoader::new(ws.path());
        assert!(loader.list().unwrap().is_empty());
    }

    #[test]
    fn test_name_directory_mismatch_is_skipped() {
        let ws = TempDir::new().unwrap();
        create_skill(
            ws.path(),
            "weather",
            &frontmatter("forecast", "Forecasts", "Body"),
        );

        let loader = FileSkillLoader::new(ws.path());
        assert!(loader.list().unwrap().is_empty());
    }

    #[test]
    fn test_invalid_name_format_is_skipped() {
        let ws = TempDir::new().unwrap();
        create_skill(
            ws.path(),
            "My_Skill",
            "---\nname: My_Skill\ndescription: Bad\n---\nContent",
        );

        let loader = FileSkillLoader::new(ws.path());
        assert!(loader.list().unwrap().is_empty());
    }

    #[test]
    fn test_multiple_valid_skills() {
        let ws = TempDir::new().unwrap();
        create_skill(
            ws.path(),
            "weather",
            &frontmatter("weather", "Weather", "W body"),
        );
        create_skill(
            ws.path(),
            "code-review",
            &frontmatter("code-review", "Reviews", "CR body"),
        );

        let loader = FileSkillLoader::new(ws.path());
        let skills = loader.list().unwrap();
        assert_eq!(skills.len(), 2);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"weather"));
        assert!(names.contains(&"code-review"));
    }

    #[test]
    fn test_invalid_skills_skipped_alongside_valid() {
        let ws = TempDir::new().unwrap();
        create_skill(
            ws.path(),
            "weather",
            &frontmatter("weather", "Weather", "Valid"),
        );
        create_skill(ws.path(), "bad", "No frontmatter");

        let loader = FileSkillLoader::new(ws.path());
        let skills = loader.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "weather");
    }
}
