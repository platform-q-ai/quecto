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

    fn default_branch(&self, repo: &str) -> Option<String> {
        let repo_path = self.resolve_repo_path(repo);
        if !self.repo_exists(repo) {
            return None;
        }
        // Try `git symbolic-ref --short HEAD` first (works on non-empty repos)
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .arg("symbolic-ref")
            .arg("--short")
            .arg("HEAD")
            .output()
            .ok()?;
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }

    fn list_branches(&self, repo: &str, limit: usize) -> Vec<String> {
        if limit == 0 {
            return vec![];
        }
        let repo_path = self.resolve_repo_path(repo);
        if !self.repo_exists(repo) {
            return vec![];
        }
        let output = match Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .arg("branch")
            .arg("--format=%(refname:short)")
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return vec![],
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(limit)
            .map(str::to_string)
            .collect()
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

/// Validate that a git URL is safe for import.
///
/// Only HTTPS and SSH are allowed. Rejected transports:
/// - `ext::` — arbitrary command execution
/// - `git://` — unauthenticated, unencrypted TCP (SSRF risk on port 9418)
/// - `http://` — unencrypted (consistent with provider URL validation)
/// - `file://` and local paths — import is for remote repos only
fn is_safe_import_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    if lower.starts_with("ext::") {
        return false;
    }
    lower.starts_with("https://") || lower.starts_with("ssh://") || lower.starts_with("git@")
}

/// Git subprocess timeout in seconds (covers init, add, commit, branch, clone).
const GIT_TIMEOUT_SECS: u64 = 120;

/// Sanitize git stderr for LLM consumption: strip network topology details.
fn sanitize_git_error(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return "git operation failed".to_string();
    }
    // Cap length to avoid leaking verbose error output
    let capped = if trimmed.len() > 256 {
        &trimmed[..256]
    } else {
        trimmed
    };
    capped.to_string()
}

/// Run a git command with env_clear, stdin null, and timeout.
///
/// All git subprocesses get a clean environment (no API keys leaked),
/// stdin closed (no credential prompts), and a wall-clock timeout.
fn run_git(args: &[&str], cwd: &Path) -> Result<(), CommandError> {
    use std::process::Stdio;
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CommandError::GitFailed(format!("spawn: {e}")))?;

    let timeout = std::time::Duration::from_secs(GIT_TIMEOUT_SECS);
    match child_wait_timeout(&mut child, timeout) {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => {
            let stderr = read_child_stderr(&mut child);
            Err(CommandError::GitFailed(sanitize_git_error(&stderr)))
        }
        Err(msg) => Err(CommandError::GitFailed(msg)),
    }
}

/// Wait for a child process with a timeout. Kills on timeout.
fn child_wait_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Result<std::process::ExitStatus, String> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("git timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait: {e}")),
        }
    }
}

/// Read stderr from a child process (best effort).
fn read_child_stderr(child: &mut std::process::Child) -> String {
    child
        .stderr
        .take()
        .map(|mut s| {
            let mut buf = String::new();
            use std::io::Read;
            let _ = s.read_to_string(&mut buf);
            buf
        })
        .unwrap_or_default()
}

impl RepoCreator for WorkspaceRepoCreator {
    fn create(&self, name: &str, description: Option<&str>) -> Result<String, CommandError> {
        let repo_path = self.workspace.join(name);
        // Use create_dir (not create_dir_all) so concurrent creation fails
        // atomically rather than silently succeeding on an existing directory.
        std::fs::create_dir(&repo_path)
            .map_err(|e| CommandError::GitFailed(format!("mkdir: {e}")))?;

        // Cleanup guard: remove the directory on any failure so a retry
        // doesn't permanently hit AlreadyExists.
        let result = self.create_inner(&repo_path, name, description);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&repo_path);
        }
        result
    }

    fn import(&self, url: &str, name: &str) -> Result<String, CommandError> {
        if !is_safe_import_url(url) {
            return Err(CommandError::InvalidUrl);
        }
        let repo_path = self.workspace.join(name);
        let dest = repo_path.to_string_lossy().to_string();
        // Use run_git for consistent env_clear, stdin null, and timeout.
        // Clone runs in the workspace dir since the repo doesn't exist yet.
        run_git(
            &["clone", "--quiet", "--depth", "1", url, &dest],
            &self.workspace,
        )?;
        Ok(dest)
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

impl WorkspaceRepoCreator {
    /// Inner create logic, called after directory creation. On error the
    /// caller removes the directory to avoid orphaned state.
    fn create_inner(
        &self,
        repo_path: &Path,
        name: &str,
        description: Option<&str>,
    ) -> Result<String, CommandError> {
        run_git(&["init"], repo_path)?;

        let readme = repo_path.join("README.md");
        let content = match description {
            Some(d) => format!("# {name}\n\n{d}\n"),
            None => format!("# {name}\n"),
        };
        std::fs::write(&readme, content)
            .map_err(|e| CommandError::GitFailed(format!("write: {e}")))?;

        run_git(&["add", "README.md"], repo_path)?;
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
            repo_path,
        )?;
        run_git(&["branch", "-M", "main"], repo_path)?;

        Ok(repo_path.to_string_lossy().to_string())
    }
}

#[cfg(test)]
#[path = "runtime_adapters_tests.rs"]
mod tests;
