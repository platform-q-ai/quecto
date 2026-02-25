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

        // Check for stale lock
        if let Ok(contents) = fs::read_to_string(&lock_path) {
            let pid_str = contents.trim();
            if let Ok(pid) = pid_str.parse::<u32>() {
                let proc_path = format!("/proc/{}", pid);
                if Path::new(&proc_path).exists() {
                    return Err("mirror fetch lock is held by another process".to_string());
                }
                // Stale lock — remove it
                let _ = fs::remove_file(&lock_path);
            }
        }

        // Write our PID
        let pid = std::process::id();
        fs::write(&lock_path, pid.to_string()).map_err(|e| format!("lock write: {e}"))?;
        Ok(LockGuard { path: lock_path })
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
        let job_dir = self.job_dir(job_id);
        if job_dir.exists() {
            fs::remove_dir_all(&job_dir).is_ok()
        } else {
            true
        }
    }

    fn remove_job_repo_keep_artifacts(&self, job_id: &str) -> bool {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, FileRepoMirrorStore) {
        let td = tempfile::TempDir::new().unwrap();
        let store = FileRepoMirrorStore::new(td.path().to_path_buf());
        (td, store)
    }

    fn cj<'a>(repo: &'a str, job: &'a str, base: &'a str, branch: &'a str) -> CloneJobParams<'a> {
        CloneJobParams {
            repo,
            job_id: job,
            base_ref: base,
            job_branch: branch,
        }
    }

    fn git(path: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .unwrap()
            .success()
    }

    fn init_origin(path: &Path) {
        fs::create_dir_all(path).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("--quiet")
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
        fs::write(path.join("README.md"), "hello\n").unwrap();
        assert!(git(path, &["add", "."]));
        assert!(git(
            path,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
                "--quiet"
            ]
        ));
        assert!(git(path, &["branch", "-M", "main"]));
    }

    /// Setup store with an origin repo already mirrored.
    fn setup_with_mirror() -> (tempfile::TempDir, FileRepoMirrorStore, tempfile::TempDir) {
        let (td, mut store) = setup();
        let origin = tempfile::TempDir::new().unwrap();
        init_origin(origin.path());
        store.create_mirror("org/my-app", origin.path().to_str().unwrap());
        (td, store, origin)
    }

    #[test]
    fn test_mirror_path_variants() {
        let (_td, store) = setup();
        assert_eq!(
            store.mirror_path_for_repo("org/my-app"),
            Some("org__my-app.git".into())
        );
        assert_eq!(
            store.mirror_path_for_repo("org/my-app.git"),
            Some("org__my-app.git".into())
        );
        assert_eq!(store.mirror_path_for_repo("../escape/attempt"), None);
        assert_eq!(store.mirror_path_for_repo("/etc/passwd"), None);
        assert_eq!(store.mirror_path_for_repo(""), None);
    }

    #[test]
    fn test_mirror_exists_lifecycle() {
        let (td, store) = setup();
        assert!(!store.mirror_exists("org/my-app"));
        let dir = td.path().join("mirrors").join("org__my-app.git");
        fs::create_dir_all(&dir).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("--bare")
                .arg("--quiet")
                .arg(&dir)
                .status()
                .unwrap()
                .success()
        );
        assert!(store.mirror_exists("org/my-app"));
    }

    #[test]
    fn test_create_mirror_and_idempotent() {
        let origin = tempfile::TempDir::new().unwrap();
        init_origin(origin.path());
        let (_td, mut store) = setup();
        let r1 = store.create_mirror("org/my-app", origin.path().to_str().unwrap());
        assert!(r1.ok, "create failed: {:?}", r1.error);
        assert!(store.mirror_exists("org/my-app"));
        let r2 = store.create_mirror("org/my-app", origin.path().to_str().unwrap());
        assert!(r2.ok, "idempotent create failed");
    }

    #[test]
    fn test_create_mirror_rejects_invalid() {
        let (_td, mut store) = setup();
        let r = store.create_mirror("../escape", "file:///tmp/fake");
        assert!(!r.ok);
        assert_eq!(r.error_code.as_deref(), Some("invalid_repo"));
    }

    #[test]
    fn test_clone_for_job_success() {
        let (td, store, _origin) = setup_with_mirror();
        let r = store.clone_for_job(&cj("org/my-app", "job_001", "main", "quecto/job/job_001"));
        assert!(r.ok, "clone failed: {:?}", r.error);
        let repo = td.path().join("jobs/job_001/repo");
        assert!(repo.join(".git").exists());
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "quecto/job/job_001"
        );
    }

    #[test]
    fn test_clone_invalid_ref_and_no_mirror() {
        let (_td, store, _origin) = setup_with_mirror();
        let r = store.clone_for_job(&cj("org/my-app", "j1", "nonexistent", "quecto/job/j1"));
        assert!(!r.ok);
        assert_eq!(r.error_code.as_deref(), Some("invalid_base_ref"));

        let (_, store2) = setup();
        let r2 = store2.clone_for_job(&cj("org/my-app", "j1", "main", "quecto/job/j1"));
        assert!(!r2.ok);
        assert_eq!(r2.error_code.as_deref(), Some("no_mirror"));
    }

    #[test]
    fn test_remove_job_repo_and_keep_artifacts() {
        let (td, store, _origin) = setup_with_mirror();
        store.clone_for_job(&cj("org/my-app", "job_001", "main", "quecto/job/job_001"));
        assert!(td.path().join("jobs/job_001/repo").exists());
        assert!(store.remove_job_repo("job_001"));
        assert!(!td.path().join("jobs/job_001").exists());
        assert!(store.mirror_exists("org/my-app"));

        // Test keep_artifacts variant
        store.clone_for_job(&cj("org/my-app", "job_002", "main", "quecto/job/job_002"));
        let arts = td.path().join("jobs/job_002/artifacts");
        fs::create_dir_all(&arts).unwrap();
        fs::write(arts.join("s.json"), "{}").unwrap();
        assert!(store.remove_job_repo_keep_artifacts("job_002"));
        assert!(!td.path().join("jobs/job_002/repo").exists());
        assert!(td.path().join("jobs/job_002/artifacts").exists());
    }

    #[test]
    fn test_two_jobs_isolated() {
        let (td, store, _origin) = setup_with_mirror();
        assert!(
            store
                .clone_for_job(&cj("org/my-app", "j1", "main", "quecto/job/j1"))
                .ok
        );
        assert!(
            store
                .clone_for_job(&cj("org/my-app", "j2", "main", "quecto/job/j2"))
                .ok
        );
        let g1 = td.path().join("jobs/j1/repo/.git").canonicalize().unwrap();
        let g2 = td.path().join("jobs/j2/repo/.git").canonicalize().unwrap();
        assert_ne!(g1, g2);
    }

    #[test]
    fn test_fetch_mirror() {
        let (_td, store, _origin) = setup_with_mirror();
        assert!(store.fetch_mirror("org/my-app").ok);
        let (_, store2) = setup();
        assert!(!store2.fetch_mirror("org/my-app").ok);
    }

    #[test]
    fn test_workspace_and_path_safety() {
        let ws = tempfile::TempDir::new().unwrap();
        let store = FileRepoMirrorStore::with_workspace(ws.path().join("c"), ws.path().into());
        assert!(store.job_dir("j1").starts_with(ws.path()));
        let (_td, store2) = setup();
        assert!(!store2.job_dir("j1").to_string_lossy().contains(".."));
    }
}
