// Skill loader: loads skills from workspace, global, and builtin sources.

use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;
use crate::domain::skill::{Skill, SkillLoader, SkillSource};

/// File-based skill loader that resolves skills from multiple directories.
#[derive(Debug)]
pub struct FileSkillLoader {
    workspace_dir: PathBuf,
    global_dir: PathBuf,
    builtin_dir: PathBuf,
}

impl FileSkillLoader {
    pub fn new(
        workspace: impl AsRef<Path>,
        global: impl AsRef<Path>,
        builtin: impl AsRef<Path>,
    ) -> Self {
        Self {
            workspace_dir: workspace.as_ref().join("skills"),
            global_dir: global.as_ref().join("skills"),
            builtin_dir: builtin.as_ref().to_path_buf(),
        }
    }

    /// Load skills from a single directory with a given source label.
    fn load_from_dir(dir: &Path, source: SkillSource) -> Vec<Skill> {
        let mut skills = Vec::new();
        if !dir.is_dir() {
            return skills;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let skill_md = entry.path().join("SKILL.md");
                    let content = if skill_md.exists() {
                        std::fs::read_to_string(&skill_md).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    skills.push(Skill {
                        name,
                        content,
                        source: source.clone(),
                    });
                }
            }
        }
        skills
    }
}

impl SkillLoader for FileSkillLoader {
    fn list(&self) -> Result<Vec<Skill>, DomainError> {
        let mut all = Vec::new();
        all.extend(Self::load_from_dir(
            &self.workspace_dir,
            SkillSource::Workspace,
        ));
        all.extend(Self::load_from_dir(&self.global_dir, SkillSource::Global));
        all.extend(Self::load_from_dir(&self.builtin_dir, SkillSource::Builtin));
        Ok(all)
    }

    fn load(&self, name: &str) -> Result<Option<Skill>, DomainError> {
        // Search workspace first, then global, then builtin
        for (dir, source) in [
            (&self.workspace_dir, SkillSource::Workspace),
            (&self.global_dir, SkillSource::Global),
            (&self.builtin_dir, SkillSource::Builtin),
        ] {
            let skill_dir = dir.join(name);
            if skill_dir.is_dir() {
                let skill_md = skill_dir.join("SKILL.md");
                let content = if skill_md.exists() {
                    std::fs::read_to_string(&skill_md).unwrap_or_default()
                } else {
                    String::new()
                };
                return Ok(Some(Skill {
                    name: name.to_string(),
                    content,
                    source,
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_dirs() -> (TempDir, TempDir, TempDir) {
        (
            TempDir::new().unwrap(),
            TempDir::new().unwrap(),
            TempDir::new().unwrap(),
        )
    }

    fn create_skill(base: &Path, name: &str, content: &str) {
        let skill_dir = base.join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    fn create_builtin_skill(base: &Path, name: &str, content: &str) {
        let skill_dir = base.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_list_workspace_skills() {
        let (ws, global, builtin) = setup_dirs();
        create_skill(ws.path(), "weather", "Weather skill content");

        let loader = FileSkillLoader::new(ws.path(), global.path(), builtin.path());
        let skills = loader.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "weather");
        assert_eq!(skills[0].source, SkillSource::Workspace);
    }

    #[test]
    fn test_list_from_multiple_sources() {
        let (ws, global, builtin) = setup_dirs();
        create_skill(ws.path(), "weather", "ws weather");
        create_skill(global.path(), "calculator", "global calc");
        create_builtin_skill(builtin.path(), "news", "builtin news");

        let loader = FileSkillLoader::new(ws.path(), global.path(), builtin.path());
        let skills = loader.list().unwrap();
        assert_eq!(skills.len(), 3);

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"weather"));
        assert!(names.contains(&"calculator"));
        assert!(names.contains(&"news"));
    }

    #[test]
    fn test_load_specific_skill() {
        let (ws, global, builtin) = setup_dirs();
        create_skill(ws.path(), "weather", "Weather content");

        let loader = FileSkillLoader::new(ws.path(), global.path(), builtin.path());
        let skill = loader.load("weather").unwrap();
        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert_eq!(skill.content, "Weather content");
        assert_eq!(skill.source, SkillSource::Workspace);
    }

    #[test]
    fn test_load_nonexistent_skill() {
        let (ws, global, builtin) = setup_dirs();
        let loader = FileSkillLoader::new(ws.path(), global.path(), builtin.path());
        let skill = loader.load("nonexistent").unwrap();
        assert!(skill.is_none());
    }

    #[test]
    fn test_workspace_priority_over_global() {
        let (ws, global, builtin) = setup_dirs();
        create_skill(ws.path(), "weather", "workspace version");
        create_skill(global.path(), "weather", "global version");

        let loader = FileSkillLoader::new(ws.path(), global.path(), builtin.path());
        let skill = loader.load("weather").unwrap().unwrap();
        assert_eq!(skill.source, SkillSource::Workspace);
        assert_eq!(skill.content, "workspace version");
    }

    #[test]
    fn test_empty_dirs() {
        let (ws, global, builtin) = setup_dirs();
        let loader = FileSkillLoader::new(ws.path(), global.path(), builtin.path());
        let skills = loader.list().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_skill_without_skill_md() {
        let (ws, global, builtin) = setup_dirs();
        // Create skill dir without SKILL.md
        std::fs::create_dir_all(ws.path().join("skills").join("empty_skill")).unwrap();

        let loader = FileSkillLoader::new(ws.path(), global.path(), builtin.path());
        let skills = loader.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert!(skills[0].content.is_empty());
    }
}
