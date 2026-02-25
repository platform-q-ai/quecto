use std::path::{Path, PathBuf};
use std::process::Command;

use crate::domain::coding_ports::{RepoValidator, SkillResolver};
use crate::domain::skill::is_valid_skill_name;

#[derive(Debug, Clone)]
pub struct WorkspaceRepoValidator {
    workspace: PathBuf,
}

impl WorkspaceRepoValidator {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }

    fn resolve_repo_path(&self, repo: &str) -> PathBuf {
        let input = PathBuf::from(repo);
        if input.is_absolute() {
            input
        } else {
            self.workspace.join(input)
        }
    }

    fn is_within_workspace(&self, path: &Path) -> bool {
        let ws = match self.workspace.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };
        let candidate = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };
        candidate.starts_with(ws)
    }
}

impl RepoValidator for WorkspaceRepoValidator {
    fn repo_exists(&self, repo: &str) -> bool {
        let repo_path = self.resolve_repo_path(repo);
        if !self.is_within_workspace(&repo_path) {
            return false;
        }
        repo_path.is_dir() && repo_path.join(".git").exists()
    }

    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool {
        let repo_path = self.resolve_repo_path(repo);
        if !self.repo_exists(repo) {
            return false;
        }

        let qualified = format!("{base_ref}^{{commit}}");
        Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("rev-parse")
            .arg("--verify")
            .arg("--quiet")
            .arg(qualified)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceSkillResolver {
    workspace: PathBuf,
}

impl WorkspaceSkillResolver {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

impl SkillResolver for WorkspaceSkillResolver {
    fn skill_exists(&self, name: &str) -> bool {
        if !is_valid_skill_name(name) {
            return false;
        }
        let p = self.workspace.join("skills").join(name).join("SKILL.md");
        p.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_git_repo(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        let st = Command::new("git").arg("init").arg(path).status().unwrap();
        assert!(st.success());

        let readme = path.join("README.md");
        std::fs::write(&readme, "hello\n").unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("add")
                .arg("README.md")
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("-c")
                .arg("user.email=test@example.com")
                .arg("-c")
                .arg("user.name=test")
                .arg("commit")
                .arg("-m")
                .arg("init")
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("branch")
                .arg("-M")
                .arg("main")
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn test_repo_exists_and_ref_exists_inside_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("repo-a");
        init_git_repo(&repo);

        let v = WorkspaceRepoValidator::new(tmp.path().to_path_buf());
        assert!(v.repo_exists("repo-a"));
        assert!(v.ref_exists("repo-a", "main"));
    }

    #[test]
    fn test_repo_outside_workspace_rejected() {
        let ws = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let repo = outside.path().join("repo-b");
        init_git_repo(&repo);

        let v = WorkspaceRepoValidator::new(ws.path().to_path_buf());
        assert!(!v.repo_exists(repo.to_str().unwrap()));
    }

    #[test]
    fn test_skill_exists_checks_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("skills").join("default-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: default-skill\ndescription: d\n---\nX",
        )
        .unwrap();

        let r = WorkspaceSkillResolver::new(tmp.path().to_path_buf());
        assert!(r.skill_exists("default-skill"));
        assert!(!r.skill_exists("missing"));
    }

    #[test]
    fn test_skill_invalid_name_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let r = WorkspaceSkillResolver::new(tmp.path().to_path_buf());
        assert!(!r.skill_exists("../escape"));
    }
}
