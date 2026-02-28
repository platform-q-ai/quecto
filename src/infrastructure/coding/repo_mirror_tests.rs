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
    assert!(git(path, &["init", "--quiet", "."]));
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

#[test]
fn test_safe_git_url_accepts_valid() {
    assert!(is_safe_git_url("https://github.com/org/repo.git"));
    assert!(is_safe_git_url("ssh://git@github.com/org/repo.git"));
    assert!(is_safe_git_url("git@github.com:org/repo.git"));
    assert!(is_safe_git_url("git://host/repo"));
    assert!(is_safe_git_url("file:///tmp/repo"));
    assert!(is_safe_git_url("/tmp/repo"));
}

#[test]
fn test_safe_git_url_rejects_ext() {
    assert!(!is_safe_git_url("ext::sh -c evil"));
    assert!(!is_safe_git_url("EXT::cmd"));
}

#[test]
fn test_safe_job_id_valid() {
    assert!(is_safe_job_id("job_000001"));
    assert!(is_safe_job_id("my-job-123"));
}

#[test]
fn test_safe_job_id_rejects_traversal() {
    assert!(!is_safe_job_id("../../etc"));
    assert!(!is_safe_job_id("foo/bar"));
    assert!(!is_safe_job_id(""));
    assert!(!is_safe_job_id("foo\\bar"));
}

#[test]
fn test_job_repo_path() {
    let (td, store) = setup();
    let path = store.job_repo_path("job_001");
    let expected = td.path().join("jobs/job_001/repo");
    assert_eq!(path, expected.to_string_lossy().to_string());
}

// ── Issue 3: workspace collision — pre-existing dest dir ─────────────────

#[test]
fn test_clone_for_job_succeeds_when_dest_already_exists() {
    // Simulates a retry after a previous aborted run left a stale repo dir.
    // clone_for_job must remove the stale dir and succeed.
    let (td, store, _origin) = setup_with_mirror();

    // First clone — succeeds and leaves the dir in place.
    let r1 = store.clone_for_job(&cj("org/my-app", "job_001", "main", "quecto/job/job_001"));
    assert!(r1.ok, "first clone failed: {:?}", r1.error);
    let repo_dir = td.path().join("jobs/job_001/repo");
    assert!(repo_dir.exists(), "repo dir should exist after first clone");

    // Simulate a retry with the same job_id — dest dir is non-empty.
    let r2 = store.clone_for_job(&cj("org/my-app", "job_001", "main", "quecto/job/job_001"));
    assert!(
        r2.ok,
        "second clone (retry) must succeed even though dest already exists: {:?}",
        r2.error
    );
    // The repo is still valid.
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&head.stdout).trim(),
        "quecto/job/job_001"
    );
}
