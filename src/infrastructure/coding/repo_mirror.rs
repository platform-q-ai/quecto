//! Bare mirror cache and per-job clone implementation.
//!
//! Layout:
//! ```text
//! <cache_dir>/
//!   mirrors/
//!     org__my-app.git/     (bare mirror)
//!   jobs/
//!     <job_id>/
//!       repo/              (per-job clone)
//!       artifacts/         (preserved on keep_artifacts cleanup)
//! ```
//!
//! Flock protocol:
//! - Mirror fetch: exclusive lock on `<mirror_dir>/fetch.lock`
//! - Clone from mirror: shared lock on `<mirror_dir>/fetch.lock`
//!
//! This ensures fetches and clones don't conflict.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::domain::coding_ports::{CloneJobParams, RepoMirrorStore, RepoOpResult};

/// File-backed repo mirror store using git bare clones and local clones.
#[derive(Debug)]
pub struct FileRepoMirrorStore {
    /// Root directory containing `mirrors/` and `jobs/`.
    cache_dir: PathBuf,
    /// Optional workspace boundary for path safety checks.
    workspace: Option<PathBuf>,
}

impl FileRepoMirrorStore {
    /// Create a new store rooted at `cache_dir`.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            workspace: None,
        }
    }

    /// Create a new store with a workspace boundary.
    pub fn with_workspace(cache_dir: PathBuf, workspace: PathBuf) -> Self {
        Self {
            cache_dir,
            workspace: Some(workspace),
        }
    }

    fn mirrors_dir(&self) -> PathBuf {
        self.cache_dir.join("mirrors")
    }

    fn jobs_dir(&self) -> PathBuf {
        self.cache_dir.join("jobs")
    }

    fn mirror_dir_for(&self, repo: &str) -> Option<PathBuf> {
        self.mirror_path_for_repo(repo)
            .map(|name| self.mirrors_dir().join(name))
    }

    fn job_repo_dir(&self, job_id: &str) -> PathBuf {
        self.jobs_dir().join(job_id).join("repo")
    }

    fn job_dir(&self, job_id: &str) -> PathBuf {
        self.jobs_dir().join(job_id)
    }

    fn job_artifacts_dir(&self, job_id: &str) -> PathBuf {
        self.jobs_dir().join(job_id).join("artifacts")
    }

    /// Returns the workspace boundary, if set.
    pub fn workspace(&self) -> Option<&Path> {
        self.workspace.as_deref()
    }

    /// Acquire exclusive flock (for fetch) or shared flock (for clone).
    /// Returns a `FlockGuard` that releases on drop.
    ///
    /// This is a simplified flock implementation using lock files with PIDs.
    /// For a production system, use `flock(2)` via the `fs2` crate.
    fn acquire_exclusive_lock(&self, mirror_dir: &Path) -> Result<LockGuard, String> {
        let lock_path = mirror_dir.join("fetch.lock");
        if let Some(parent) = lock_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        // Remove stale lock from a dead process before attempting atomic create
        if let Ok(contents) = fs::read_to_string(&lock_path) {
            let pid_str = contents.trim();
            if let Ok(pid) = pid_str.parse::<u32>() {
                let proc_path = format!("/proc/{}", pid);
                if Path::new(&proc_path).exists() {
                    return Err("mirror fetch lock is held by another process".to_string());
                }
                let _ = fs::remove_file(&lock_path);
            }
        }

        // Atomic create via O_CREAT|O_EXCL — avoids TOCTOU race
        let pid = std::process::id();
        let result = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path);
        match result {
            Ok(mut f) => {
                use std::io::Write;
                write!(f, "{}", pid).map_err(|e| format!("lock write: {e}"))?;
                Ok(LockGuard { path: lock_path })
            }
            Err(_) => Err("mirror fetch lock race — another process acquired it".to_string()),
        }
    }

    fn acquire_shared_lock(&self, mirror_dir: &Path) -> Result<LockGuard, String> {
        let lock_path = mirror_dir.join("fetch.lock");

        // If exclusive lock is held, wait/fail
        if let Ok(contents) = fs::read_to_string(&lock_path) {
            let pid_str = contents.trim();
            if let Ok(pid) = pid_str.parse::<u32>() {
                let proc_path = format!("/proc/{}", pid);
                if Path::new(&proc_path).exists() {
                    return Err("mirror fetch lock is held — clone must wait".to_string());
                }
                // Stale lock — remove it
                let _ = fs::remove_file(&lock_path);
            }
        }

        // Shared locks use a different file pattern
        let shared_path = mirror_dir.join(format!("clone.{}.lock", std::process::id()));
        fs::write(&shared_path, std::process::id().to_string())
            .map_err(|e| format!("shared lock write: {e}"))?;
        Ok(LockGuard { path: shared_path })
    }

    fn has_active_shared_locks(&self, mirror_dir: &Path) -> bool {
        let entries = match fs::read_dir(mirror_dir) {
            Ok(e) => e,
            Err(_) => return false,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("clone.") && name_str.ends_with(".lock") {
                // Check if the PID is alive
                let pid_str = name_str
                    .strip_prefix("clone.")
                    .and_then(|s| s.strip_suffix(".lock"));
                if let Some(pid_s) = pid_str {
                    if let Ok(pid) = pid_s.parse::<u32>() {
                        let proc_path = format!("/proc/{}", pid);
                        if Path::new(&proc_path).exists() {
                            return true;
                        }
                        // Stale — clean up
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
        false
    }
}

/// RAII lock guard that removes the lock file on drop.
#[derive(Debug)]
struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Validate that a git URL is safe (no ext:: or other dangerous transports).
fn is_safe_git_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    // Block git ext:: transport (arbitrary command execution)
    if lower.starts_with("ext::") {
        return false;
    }
    // Allow https://, ssh://, git://, file://, local paths, and shorthand user@host:path
    lower.starts_with("https://")
        || lower.starts_with("ssh://")
        || lower.starts_with("git://")
        || lower.starts_with("git@")
        || lower.starts_with("http://")
        || lower.starts_with("file://")
        || url.starts_with('/') // absolute local path
}

/// Validate that a job_id contains no path traversal characters.
fn is_safe_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && !job_id.contains('/')
        && !job_id.contains('\\')
        && !job_id.contains("..")
        && job_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

fn repo_err(duration_ms: u64, error: &str, code: &str) -> RepoOpResult {
    RepoOpResult {
        ok: false,
        duration_ms,
        error: Some(error.to_string()),
        error_code: Some(code.to_string()),
    }
}

/// Run a prepared git command and return `Ok(())` on success, `Err(RepoOpResult)` on failure.
fn check_git(cmd: &mut Command, start: &Instant, error_code: &str) -> Result<(), RepoOpResult> {
    match cmd.output() {
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            Err(repo_err(
                start.elapsed().as_millis() as u64,
                &stderr,
                error_code,
            ))
        }
        Err(e) => Err(repo_err(
            start.elapsed().as_millis() as u64,
            &e.to_string(),
            error_code,
        )),
        _ => Ok(()),
    }
}

impl RepoMirrorStore for FileRepoMirrorStore {
    fn mirror_exists(&self, repo: &str) -> bool {
        match self.mirror_dir_for(repo) {
            Some(dir) => dir.is_dir() && dir.join("HEAD").exists(),
            None => false,
        }
    }

    fn create_mirror(&mut self, repo: &str, remote_url: &str) -> RepoOpResult {
        // Validate URL scheme to prevent git ext:: command execution
        if !is_safe_git_url(remote_url) {
            return RepoOpResult {
                ok: false,
                duration_ms: 0,
                error: Some("rejected: unsafe git URL scheme".to_string()),
                error_code: Some("invalid_url".to_string()),
            };
        }

        let mirror_dir = match self.mirror_dir_for(repo) {
            Some(d) => d,
            None => {
                return RepoOpResult {
                    ok: false,
                    duration_ms: 0,
                    error: Some("invalid repo identifier".to_string()),
                    error_code: Some("invalid_repo".to_string()),
                };
            }
        };

        if mirror_dir.exists() {
            return RepoOpResult {
                ok: true,
                duration_ms: 0,
                error: None,
                error_code: None,
            };
        }

        let start = Instant::now();
        if let Some(parent) = mirror_dir.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let output = Command::new("git")
            .arg("clone")
            .arg("--bare")
            .arg("--quiet")
            .arg(remote_url)
            .arg(&mirror_dir)
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match output {
            Ok(o) if o.status.success() => RepoOpResult {
                ok: true,
                duration_ms,
                error: None,
                error_code: None,
            },
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                RepoOpResult {
                    ok: false,
                    duration_ms,
                    error: Some(stderr),
                    error_code: Some("clone_error".to_string()),
                }
            }
            Err(e) => RepoOpResult {
                ok: false,
                duration_ms,
                error: Some(e.to_string()),
                error_code: Some("clone_error".to_string()),
            },
        }
    }

    fn fetch_mirror(&self, repo: &str) -> RepoOpResult {
        let mirror_dir = match self.mirror_dir_for(repo) {
            Some(d) if d.is_dir() => d,
            _ => {
                return RepoOpResult {
                    ok: false,
                    duration_ms: 0,
                    error: Some("mirror does not exist".to_string()),
                    error_code: Some("no_mirror".to_string()),
                };
            }
        };

        // Check for shared clone locks — if clones are active, wait
        if self.has_active_shared_locks(&mirror_dir) {
            return RepoOpResult {
                ok: false,
                duration_ms: 0,
                error: Some("shared clone locks are active — fetch must wait".to_string()),
                error_code: Some("lock_contention".to_string()),
            };
        }

        let _lock = match self.acquire_exclusive_lock(&mirror_dir) {
            Ok(g) => g,
            Err(e) => {
                return RepoOpResult {
                    ok: false,
                    duration_ms: 0,
                    error: Some(e),
                    error_code: Some("lock_error".to_string()),
                };
            }
        };

        let start = Instant::now();
        let output = Command::new("git")
            .arg("-C")
            .arg(&mirror_dir)
            .arg("fetch")
            .arg("--all")
            .arg("--quiet")
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match output {
            Ok(o) if o.status.success() => RepoOpResult {
                ok: true,
                duration_ms,
                error: None,
                error_code: None,
            },
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                RepoOpResult {
                    ok: false,
                    duration_ms,
                    error: Some(stderr),
                    error_code: Some("fetch_error".to_string()),
                }
            }
            Err(e) => RepoOpResult {
                ok: false,
                duration_ms,
                error: Some(e.to_string()),
                error_code: Some("fetch_error".to_string()),
            },
        }
    }

    fn clone_for_job(&self, params: &CloneJobParams<'_>) -> RepoOpResult {
        // Reject refs that start with '-' to prevent git option injection
        if params.base_ref.starts_with('-') || params.job_branch.starts_with('-') {
            return repo_err(0, "ref must not start with '-'", "invalid_ref");
        }
        let mirror_dir = match self.mirror_dir_for(params.repo) {
            Some(d) if d.is_dir() => d,
            _ => return repo_err(0, "mirror does not exist", "no_mirror"),
        };
        let _lock = match self.acquire_shared_lock(&mirror_dir) {
            Ok(g) => g,
            Err(e) => return repo_err(0, &e, "lock_error"),
        };
        let dest = self.job_repo_dir(params.job_id);
        if let Some(p) = dest.parent() {
            let _ = fs::create_dir_all(p);
        }
        let start = Instant::now();
        if let Err(r) = check_git(
            Command::new("git")
                .arg("clone")
                .arg("--quiet")
                .arg(&mirror_dir)
                .arg(&dest),
            &start,
            "clone_error",
        ) {
            return r;
        }
        if let Err(r) = check_git(
            Command::new("git")
                .arg("-C")
                .arg(&dest)
                .arg("checkout")
                .arg(params.base_ref)
                .arg("--quiet"),
            &start,
            "invalid_base_ref",
        ) {
            let _ = fs::remove_dir_all(&dest);
            return r;
        }
        if let Err(r) = check_git(
            Command::new("git")
                .arg("-C")
                .arg(&dest)
                .arg("checkout")
                .arg("-b")
                .arg(params.job_branch)
                .arg("--quiet"),
            &start,
            "branch_error",
        ) {
            return r;
        }
        RepoOpResult {
            ok: true,
            duration_ms: start.elapsed().as_millis() as u64,
            error: None,
            error_code: None,
        }
    }

    fn mirror_path_for_repo(&self, repo: &str) -> Option<String> {
        // Reject path traversal
        if repo.contains("..") || repo.starts_with('/') || repo.starts_with('\\') {
            return None;
        }
        // Reject empty or whitespace-only
        let trimmed = repo.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Strip trailing .git if present
        let clean = trimmed.strip_suffix(".git").unwrap_or(trimmed);
        // Replace / with __ for flat directory structure
        let safe = clean.replace('/', "__");
        // Final validation: must only contain safe chars
        if safe
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            Some(format!("{safe}.git"))
        } else {
            None
        }
    }

    fn remove_job_repo(&self, job_id: &str) -> bool {
        if !is_safe_job_id(job_id) {
            return false;
        }
        let job_dir = self.job_dir(job_id);
        if job_dir.exists() {
            fs::remove_dir_all(&job_dir).is_ok()
        } else {
            true
        }
    }

    fn remove_job_repo_keep_artifacts(&self, job_id: &str) -> bool {
        if !is_safe_job_id(job_id) {
            return false;
        }
        let repo_dir = self.job_repo_dir(job_id);
        let artifacts_dir = self.job_artifacts_dir(job_id);

        // Remove repo directory but keep artifacts
        let repo_removed = if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).is_ok()
        } else {
            true
        };

        // Ensure artifacts directory isn't removed
        if !artifacts_dir.exists() {
            let _ = fs::create_dir_all(&artifacts_dir);
        }

        repo_removed
    }

    fn job_repo_path(&self, job_id: &str) -> String {
        self.job_repo_dir(job_id).to_string_lossy().into_owned()
    }

    fn resolve_local_remote(&self, repo: &str) -> Option<String> {
        // Reject unsafe repo names before joining onto workspace path.
        // Uses the same validation as mirror_path_for_repo() to prevent
        // path traversal (e.g. "../../etc/passwd").
        self.mirror_path_for_repo(repo)?;

        let workspace = self.workspace.as_ref()?;
        let repo_path = workspace.join(repo);

        // Canonicalize and verify the resolved path is inside workspace.
        let canonical = repo_path.canonicalize().ok()?;
        let ws_canonical = workspace.canonicalize().ok()?;
        if !canonical.starts_with(&ws_canonical) {
            return None;
        }

        if canonical.is_dir() && canonical.join(".git").exists() {
            Some(canonical.to_string_lossy().into_owned())
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "repo_mirror_tests.rs"]
mod tests;
