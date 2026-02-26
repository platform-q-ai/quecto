use std::path::{Path, PathBuf};
use std::process::Command;

use crate::domain::coding_command::CommandError;
use crate::domain::coding_ports::{RepoCreator, RepoValidator, SkillResolver};
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

    fn git_path_within_workspace(&self, repo_path: &Path, arg: &str) -> bool {
        let output = match Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("rev-parse")
            .arg(arg)
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return false,
        };

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            return false;
        }

        let p = PathBuf::from(&raw);
        let resolved = if p.is_absolute() {
            p
        } else {
            repo_path.join(p)
        };
        self.is_within_workspace(&resolved)
    }
}

impl RepoValidator for WorkspaceRepoValidator {
    fn repo_exists(&self, repo: &str) -> bool {
        let repo_path = self.resolve_repo_path(repo);
        if !self.is_within_workspace(&repo_path) {
            return false;
        }
        if !(repo_path.is_dir() && repo_path.join(".git").exists()) {
            return false;
        }

        self.git_path_within_workspace(&repo_path, "--git-dir")
            && self.git_path_within_workspace(&repo_path, "--show-toplevel")
    }

    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool {
        let repo_path = self.resolve_repo_path(repo);
        if !self.repo_exists(repo) {
            return false;
        }
        if base_ref.starts_with('-') {
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
        if !p.is_file() {
            return false;
        }

        let ws = match self.workspace.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };
        let skill = match p.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };
        skill.starts_with(ws)
    }
}

// ============================================================================
// WorkspaceRepoCreator — git init + clone into workspace
// ============================================================================

/// Creates and imports repositories inside the workspace directory.
#[derive(Debug, Clone)]
pub struct WorkspaceRepoCreator {
    workspace: PathBuf,
}

impl WorkspaceRepoCreator {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

/// Validate that a repo name is safe for filesystem use.
fn is_safe_repo_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    if name.starts_with('.') || name.starts_with('-') {
        return false;
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return false;
    }
    name.chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Derive a repository name from a remote URL.
fn derive_name_from_url(url: &str) -> Option<String> {
    // Handle git@host:org/repo.git and https://host/org/repo.git
    let path_part = if let Some(rest) = url.strip_prefix("git@") {
        rest.split(':').nth(1)?
    } else {
        url.split("//").nth(1).and_then(|s| {
            let parts: Vec<&str> = s.splitn(2, '/').collect();
            if parts.len() == 2 {
                Some(parts[1])
            } else {
                None
            }
        })?
    };
    let name = path_part.rsplit('/').next()?.trim_end_matches(".git");
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Validate that a git URL is safe (no ext:: or other dangerous transports).
fn is_safe_import_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    if lower.starts_with("ext::") {
        return false;
    }
    lower.starts_with("https://")
        || lower.starts_with("ssh://")
        || lower.starts_with("git@")
        || lower.starts_with("git://")
}

fn run_git(args: &[&str], cwd: &Path) -> Result<(), CommandError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| CommandError::GitFailed(format!("spawn: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CommandError::GitFailed(stderr.trim().to_string()));
    }
    Ok(())
}

impl RepoCreator for WorkspaceRepoCreator {
    fn create(&self, name: &str, description: Option<&str>) -> Result<String, CommandError> {
        let repo_path = self.workspace.join(name);
        std::fs::create_dir_all(&repo_path)
            .map_err(|e| CommandError::GitFailed(format!("mkdir: {e}")))?;

        run_git(&["init"], &repo_path)?;

        let readme = repo_path.join("README.md");
        let content = match description {
            Some(d) => format!("# {name}\n\n{d}\n"),
            None => format!("# {name}\n"),
        };
        std::fs::write(&readme, content)
            .map_err(|e| CommandError::GitFailed(format!("write: {e}")))?;

        run_git(&["add", "README.md"], &repo_path)?;
        run_git(
            &[
                "-c",
                "user.email=quecto@localhost",
                "-c",
                "user.name=quecto",
                "commit",
                "-m",
                "Initial commit",
            ],
            &repo_path,
        )?;
        run_git(&["branch", "-M", "main"], &repo_path)?;

        Ok(repo_path.to_string_lossy().to_string())
    }

    fn import(&self, url: &str, name: &str) -> Result<String, CommandError> {
        if !is_safe_import_url(url) {
            return Err(CommandError::InvalidUrl);
        }
        let repo_path = self.workspace.join(name);
        let output = Command::new("git")
            .args(["clone", "--quiet", url, &repo_path.to_string_lossy()])
            .output()
            .map_err(|e| CommandError::GitFailed(format!("spawn: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CommandError::GitFailed(stderr.trim().to_string()));
        }
        Ok(repo_path.to_string_lossy().to_string())
    }

    fn exists(&self, name: &str) -> bool {
        self.workspace.join(name).exists()
    }

    fn validate_name(&self, name: &str) -> Result<(), CommandError> {
        if !is_safe_repo_name(name) {
            return Err(CommandError::InvalidName);
        }
        Ok(())
    }

    fn name_from_url(&self, url: &str) -> Result<String, CommandError> {
        derive_name_from_url(url).ok_or(CommandError::InvalidUrl)
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
    fn test_repo_with_gitdir_outside_workspace_rejected() {
        let ws = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let outside_repo = outside.path().join("outside-repo");
        init_git_repo(&outside_repo);

        let fake_repo = ws.path().join("fake-repo");
        std::fs::create_dir_all(&fake_repo).unwrap();
        std::fs::write(
            fake_repo.join(".git"),
            format!("gitdir: {}\n", outside_repo.join(".git").display()),
        )
        .unwrap();

        let v = WorkspaceRepoValidator::new(ws.path().to_path_buf());
        assert!(!v.repo_exists("fake-repo"));
    }

    #[test]
    fn test_ref_with_option_like_prefix_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("repo-a");
        init_git_repo(&repo);

        let v = WorkspaceRepoValidator::new(tmp.path().to_path_buf());
        assert!(!v.ref_exists("repo-a", "--help"));
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

    #[cfg(unix)]
    #[test]
    fn test_skill_symlink_outside_workspace_rejected() {
        use std::os::unix::fs::symlink;

        let ws = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();

        let outside_skill = outside.path().join("outside-skill");
        std::fs::create_dir_all(&outside_skill).unwrap();
        std::fs::write(outside_skill.join("SKILL.md"), "x").unwrap();

        let skills_dir = ws.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        symlink(&outside_skill, skills_dir.join("default-skill")).unwrap();

        let r = WorkspaceSkillResolver::new(ws.path().to_path_buf());
        assert!(!r.skill_exists("default-skill"));
    }

    // ========================================================================
    // is_safe_repo_name tests
    // ========================================================================

    #[test]
    fn test_safe_repo_name_valid() {
        assert!(is_safe_repo_name("my-project"));
        assert!(is_safe_repo_name("project_123"));
        assert!(is_safe_repo_name("a"));
        assert!(is_safe_repo_name("repo.v2"));
    }

    #[test]
    fn test_safe_repo_name_rejects_empty() {
        assert!(!is_safe_repo_name(""));
    }

    #[test]
    fn test_safe_repo_name_rejects_traversal() {
        assert!(!is_safe_repo_name(".."));
        assert!(!is_safe_repo_name("a/b"));
        assert!(!is_safe_repo_name("a\\b"));
    }

    #[test]
    fn test_safe_repo_name_rejects_leading_dot_or_dash() {
        assert!(!is_safe_repo_name(".hidden"));
        assert!(!is_safe_repo_name("-flag"));
    }

    #[test]
    fn test_safe_repo_name_rejects_too_long() {
        let long = "a".repeat(129);
        assert!(!is_safe_repo_name(&long));
        let exactly = "a".repeat(128);
        assert!(is_safe_repo_name(&exactly));
    }

    #[test]
    fn test_safe_repo_name_rejects_special_chars() {
        assert!(!is_safe_repo_name("a b"));
        assert!(!is_safe_repo_name("a@b"));
        assert!(!is_safe_repo_name("a:b"));
    }

    // ========================================================================
    // derive_name_from_url tests
    // ========================================================================

    #[test]
    fn test_derive_name_https() {
        assert_eq!(
            derive_name_from_url("https://github.com/org/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_derive_name_https_no_git_suffix() {
        assert_eq!(
            derive_name_from_url("https://github.com/org/repo"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_derive_name_ssh() {
        assert_eq!(
            derive_name_from_url("git@github.com:org/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_derive_name_ssh_no_git_suffix() {
        assert_eq!(
            derive_name_from_url("git@github.com:org/my-project"),
            Some("my-project".to_string())
        );
    }

    #[test]
    fn test_derive_name_invalid() {
        assert_eq!(derive_name_from_url("not-a-url"), None);
    }

    // ========================================================================
    // is_safe_import_url tests
    // ========================================================================

    #[test]
    fn test_safe_import_url_accepts_https() {
        assert!(is_safe_import_url("https://github.com/org/repo.git"));
    }

    #[test]
    fn test_safe_import_url_accepts_ssh() {
        assert!(is_safe_import_url("ssh://git@github.com/org/repo"));
        assert!(is_safe_import_url("git@github.com:org/repo.git"));
    }

    #[test]
    fn test_safe_import_url_rejects_ext() {
        assert!(!is_safe_import_url("ext::sh -c evil"));
    }

    #[test]
    fn test_safe_import_url_rejects_file() {
        // Import should only allow remote URLs, not local file://
        assert!(!is_safe_import_url("file:///tmp/repo"));
    }

    #[test]
    fn test_safe_import_url_rejects_http() {
        // Plain http is not accepted for import
        assert!(!is_safe_import_url("http://example.com/repo"));
    }

    #[test]
    fn test_safe_import_url_rejects_local_path() {
        assert!(!is_safe_import_url("/tmp/repo"));
    }

    // ========================================================================
    // WorkspaceRepoCreator tests
    // ========================================================================

    #[test]
    fn test_creator_validate_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
        assert!(c.validate_name("good-name").is_ok());
        assert!(c.validate_name("../escape").is_err());
        assert!(c.validate_name("").is_err());
    }

    #[test]
    fn test_creator_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
        assert!(!c.exists("nonexistent"));
        std::fs::create_dir(tmp.path().join("exists")).unwrap();
        assert!(c.exists("exists"));
    }

    #[test]
    fn test_creator_name_from_url() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
        assert_eq!(
            c.name_from_url("https://github.com/org/repo.git").unwrap(),
            "repo"
        );
        assert!(c.name_from_url("bad").is_err());
    }

    #[test]
    fn test_creator_create_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
        let path = c.create("test-proj", Some("A test project")).unwrap();
        assert!(PathBuf::from(&path).join(".git").exists());
        assert!(PathBuf::from(&path).join("README.md").exists());
        let readme = std::fs::read_to_string(PathBuf::from(&path).join("README.md")).unwrap();
        assert!(readme.contains("test-proj"));
        assert!(readme.contains("A test project"));

        // Verify the repo has a main branch with at least one commit
        let v = WorkspaceRepoValidator::new(tmp.path().to_path_buf());
        assert!(v.repo_exists("test-proj"));
        assert!(v.ref_exists("test-proj", "main"));
    }

    #[test]
    fn test_creator_create_repo_no_description() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
        let path = c.create("minimal", None).unwrap();
        let readme = std::fs::read_to_string(PathBuf::from(&path).join("README.md")).unwrap();
        assert_eq!(readme, "# minimal\n");
    }
}
