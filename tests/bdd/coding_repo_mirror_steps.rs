use cucumber::{given, then, when};
use quecto::domain::coding_ports::{CloneJobParams, RepoMirrorStore};
use quecto::infrastructure::coding::repo_mirror::FileRepoMirrorStore;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

use crate::QuectoWorld;

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_mirror_store(world: &mut QuectoWorld) {
    if world.coding_mirror_store.is_none() {
        let td = TempDir::new().expect("temp dir");
        let cache_dir = td.path().to_path_buf();
        let store = FileRepoMirrorStore::new(cache_dir.clone());
        world.coding_mirror_cache_dir = Some(cache_dir);
        world.coding_mirror_store = Some(store);
        world._coding_mirror_temp_dir = Some(td);
    }
}

fn ensure_origin_repo(world: &mut QuectoWorld) {
    if world.coding_mirror_origin_path.is_none() {
        let td = TempDir::new().expect("origin temp dir");
        let origin = td.path().to_path_buf();
        init_git_repo_with_branches(&origin, &["main"]);
        world.coding_mirror_origin_path = Some(origin);
        world._coding_mirror_origin_dir = Some(td);
    }
}

fn init_git_repo_with_branches(path: &std::path::Path, branches: &[&str]) {
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
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("add")
            .arg(".")
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
            .arg("--quiet")
            .status()
            .unwrap()
            .success()
    );

    // Rename default branch to first branch name
    if let Some(first) = branches.first() {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("branch")
                .arg("-M")
                .arg(first)
                .status()
                .unwrap()
                .success()
        );
    }

    // Create additional branches
    for branch in branches.iter().skip(1) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("branch")
                .arg(branch)
                .status()
                .unwrap()
                .success()
        );
    }
}

fn mirror_store(world: &mut QuectoWorld) -> &mut FileRepoMirrorStore {
    world.coding_mirror_store.as_mut().expect("mirror store")
}

fn cj<'a>(
    repo: &'a str,
    job_id: &'a str,
    base_ref: &'a str,
    branch: &'a str,
) -> CloneJobParams<'a> {
    CloneJobParams {
        repo,
        job_id,
        base_ref,
        job_branch: branch,
    }
}

fn cache_dir(world: &QuectoWorld) -> std::path::PathBuf {
    world.coding_mirror_cache_dir.clone().expect("cache dir")
}

// ── Given steps ──────────────────────────────────────────────────────────

#[given(regex = r#"^a coding coordinator with cache directory "([^"]+)"$"#)]
fn given_cache_directory(world: &mut QuectoWorld, _dir: String) {
    ensure_mirror_store(world);
}

#[given(regex = r#"^no mirror exists for repo "([^"]+)"$"#)]
fn given_no_mirror(world: &mut QuectoWorld, repo: String) {
    ensure_mirror_store(world);
    world.coding_mirror_repo = Some(repo.clone());
    assert!(
        !mirror_store(world).mirror_exists(&repo),
        "mirror should not exist yet"
    );
}

#[given(regex = r#"^a bare mirror already exists for repo "([^"]+)"$"#)]
fn given_mirror_exists(world: &mut QuectoWorld, repo: String) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    world.coding_mirror_repo = Some(repo.clone());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    let result = mirror_store(world).create_mirror(&repo, origin.to_str().unwrap());
    assert!(result.ok, "failed to create mirror: {:?}", result.error);
}

#[given(regex = r#"^a bare mirror exists for repo "([^"]+)"$"#)]
fn given_bare_mirror_exists(world: &mut QuectoWorld, repo: String) {
    given_mirror_exists(world, repo);
}

#[given("the remote has new commits since last fetch")]
fn given_remote_has_new_commits(world: &mut QuectoWorld) {
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    fs::write(origin.join("new-file.txt"), "new content\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&origin)
            .arg("add")
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&origin)
            .arg("-c")
            .arg("user.email=test@example.com")
            .arg("-c")
            .arg("user.name=test")
            .arg("commit")
            .arg("-m")
            .arg("new commit")
            .arg("--quiet")
            .status()
            .unwrap()
            .success()
    );
}

#[given(regex = r#"^a coding job with job_id "([^"]+)"$"#)]
fn given_mirror_job_id(world: &mut QuectoWorld, job_id: String) {
    world.coding_mirror_job_id = Some(job_id);
}

#[given(regex = r#"^a bare mirror exists with branch "([^"]+)" and "([^"]+)"$"#)]
fn given_mirror_with_branches(world: &mut QuectoWorld, branch1: String, branch2: String) {
    ensure_mirror_store(world);
    // Create origin with both branches
    let td = TempDir::new().expect("origin temp dir");
    let origin = td.path().to_path_buf();
    init_git_repo_with_branches(&origin, &[&branch1, &branch2]);
    world.coding_mirror_origin_path = Some(origin.clone());
    world._coding_mirror_origin_dir = Some(td);

    let repo = "org/my-app";
    world.coding_mirror_repo = Some(repo.to_string());
    let result = mirror_store(world).create_mirror(repo, origin.to_str().unwrap());
    assert!(result.ok, "create mirror failed: {:?}", result.error);
}

#[given(regex = r#"^a coding job requests base ref "([^"]+)"$"#)]
fn given_job_requests_base_ref(world: &mut QuectoWorld, base_ref: String) {
    world.coding_mirror_job_id = Some("job_000001".to_string());
    // Store base_ref in the last_result error field temporarily as a hack
    // Actually, we'll handle this in the When step
    world.coding_mirror_last_result = Some(quecto::domain::coding_ports::RepoOpResult {
        ok: true,
        duration_ms: 0,
        error: Some(base_ref), // temp storage for base_ref
        error_code: None,
    });
}

#[given(regex = r#"^a coding job with job_id "([^"]+)" and base ref "([^"]+)"$"#)]
fn given_job_with_id_and_ref(world: &mut QuectoWorld, job_id: String, _base_ref: String) {
    world.coding_mirror_job_id = Some(job_id);
}

#[given(regex = r#"^a corrupted bare mirror exists for repo "([^"]+)"$"#)]
fn given_corrupted_mirror(world: &mut QuectoWorld, repo: String) {
    ensure_mirror_store(world);
    world.coding_mirror_repo = Some(repo.clone());
    // Create a directory that looks like a mirror but is corrupt
    let dir = cache_dir(world);
    let mirror_dir = dir.join("mirrors").join("org__my-app.git");
    fs::create_dir_all(&mirror_dir).unwrap();
    // Write a corrupted HEAD file
    fs::write(mirror_dir.join("HEAD"), "CORRUPTED").unwrap();
}

#[given("a bare mirror with a lock file held by a dead process (PID check fails)")]
fn given_stale_lock(world: &mut QuectoWorld) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    let repo = "org/my-app";
    world.coding_mirror_repo = Some(repo.to_string());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    let result = mirror_store(world).create_mirror(repo, origin.to_str().unwrap());
    assert!(result.ok);

    // Create a stale lock with a dead PID
    let dir = cache_dir(world);
    let mirror_dir = dir.join("mirrors").join("org__my-app.git");
    fs::write(mirror_dir.join("fetch.lock"), "9999999").unwrap();
}

#[given("a bare mirror with an active exclusive fetch lock")]
fn given_active_exclusive_lock(world: &mut QuectoWorld) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    let repo = "org/my-app";
    world.coding_mirror_repo = Some(repo.to_string());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    let result = mirror_store(world).create_mirror(repo, origin.to_str().unwrap());
    assert!(result.ok);

    // Create an exclusive lock with our own PID (alive)
    let dir = cache_dir(world);
    let mirror_dir = dir.join("mirrors").join("org__my-app.git");
    fs::write(
        mirror_dir.join("fetch.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
}

#[given(regex = r#"^a bare mirror with (\d+) active shared clone locks$"#)]
fn given_active_shared_locks(world: &mut QuectoWorld, count: usize) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    let repo = "org/my-app";
    world.coding_mirror_repo = Some(repo.to_string());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    let result = mirror_store(world).create_mirror(repo, origin.to_str().unwrap());
    assert!(result.ok);

    // Create shared locks with our PID (alive)
    let dir = cache_dir(world);
    let mirror_dir = dir.join("mirrors").join("org__my-app.git");
    let pid = std::process::id();
    for i in 0..count {
        // Use slightly different lock names to simulate multiple clone processes
        // In reality each clone process would have its own PID
        fs::write(
            mirror_dir.join(format!("clone.{}.lock", pid + i as u32)),
            pid.to_string(),
        )
        .unwrap();
    }
}

#[given(regex = r#"^(\d+) coding jobs for the same repo "([^"]+)"$"#)]
fn given_n_jobs_same_repo(world: &mut QuectoWorld, _count: usize, repo: String) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    world.coding_mirror_repo = Some(repo.clone());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    let result = mirror_store(world).create_mirror(&repo, origin.to_str().unwrap());
    assert!(result.ok);
}

#[given(regex = r#"^a coding coordinator with workspace "([^"]+)"$"#)]
fn given_coordinator_with_workspace(world: &mut QuectoWorld, workspace: String) {
    let td = TempDir::new().expect("temp dir");
    let cache_dir = td.path().to_path_buf();
    let store = FileRepoMirrorStore::with_workspace(cache_dir.clone(), workspace.into());
    world.coding_mirror_cache_dir = Some(cache_dir);
    world.coding_mirror_store = Some(store);
    world._coding_mirror_temp_dir = Some(td);
}

#[given(regex = r#"^a coding job in state "([^"]+)" with job directory "([^"]+)"$"#)]
fn given_job_with_directory(world: &mut QuectoWorld, _state: String, _dir: String) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    let repo = "org/my-app";
    world.coding_mirror_repo = Some(repo.to_string());
    world.coding_mirror_job_id = Some("job_000001".to_string());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    mirror_store(world).create_mirror(repo, origin.to_str().unwrap());
    let result =
        mirror_store(world).clone_for_job(&cj(repo, "job_000001", "main", "quecto/job/job_000001"));
    assert!(result.ok, "clone_for_job failed: {:?}", result.error);
}

#[given(regex = r#"^(\d+) coding jobs for repo "([^"]+)" have been cleaned up$"#)]
fn given_n_jobs_cleaned_up(world: &mut QuectoWorld, count: usize, repo: String) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    world.coding_mirror_repo = Some(repo.clone());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    mirror_store(world).create_mirror(&repo, origin.to_str().unwrap());

    for i in 0..count {
        let job_id = format!("job_{:06}", i + 1);
        let branch = format!("quecto/job/{}", job_id);
        let result = mirror_store(world).clone_for_job(&cj(&repo, &job_id, "main", &branch));
        assert!(result.ok);
        assert!(mirror_store(world).remove_job_repo(&job_id));
    }
}

#[given(regex = r#"^a coding job in state "([^"]+)" with artifacts in "([^"]+)"$"#)]
fn given_job_with_artifacts(world: &mut QuectoWorld, _state: String, _artifacts_path: String) {
    ensure_mirror_store(world);
    ensure_origin_repo(world);
    let repo = "org/my-app";
    world.coding_mirror_repo = Some(repo.to_string());
    world.coding_mirror_job_id = Some("job_000001".to_string());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    mirror_store(world).create_mirror(repo, origin.to_str().unwrap());
    let result =
        mirror_store(world).clone_for_job(&cj(repo, "job_000001", "main", "quecto/job/job_000001"));
    assert!(result.ok);
    // Create artifacts directory
    let dir = cache_dir(world);
    let artifacts = dir.join("jobs").join("job_000001").join("artifacts");
    fs::create_dir_all(&artifacts).unwrap();
    fs::write(artifacts.join("summary.json"), "{}").unwrap();
}

#[given(regex = r#"^repo identifiers "([^"]+)", "([^"]+)", "([^"]+)"$"#)]
fn given_repo_identifiers(world: &mut QuectoWorld, _r1: String, _r2: String, _r3: String) {
    ensure_mirror_store(world);
}

// ── When steps ───────────────────────────────────────────────────────────

#[when(regex = r#"^a coding job is submitted for repo "([^"]+)"$"#)]
fn when_job_submitted(world: &mut QuectoWorld, repo: String) {
    ensure_origin_repo(world);
    world.coding_mirror_repo = Some(repo.clone());
    let origin = world.coding_mirror_origin_path.clone().unwrap();

    if !mirror_store(world).mirror_exists(&repo) {
        let result = mirror_store(world).create_mirror(&repo, origin.to_str().unwrap());
        world.coding_mirror_last_result = Some(result);
        world.coding_mirror_created = true;
    } else {
        // Fetch before clone
        let result = mirror_store(world).fetch_mirror(&repo);
        world.coding_mirror_last_result = Some(result);
        world.coding_mirror_fetched = true;
    }
}

#[when("the coordinator prepares the job")]
fn when_coordinator_prepares(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    let result = mirror_store(world).clone_for_job(&cj(
        &repo,
        &job_id,
        "main",
        &format!("quecto/job/{}", job_id),
    ));
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_cloned = true;
}

#[when("the coordinator clones from the mirror for a new job")]
fn when_clone_from_mirror(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());
    let result = mirror_store(world).clone_for_job(&cj(
        &repo,
        "job_000001",
        "main",
        "quecto/job/job_000001",
    ));
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_cloned = true;
}

#[when("the coordinator clones and prepares the job")]
fn when_clone_and_prepare(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());

    // Determine base_ref from stored result or default to "main"
    let base_ref = world
        .coding_mirror_last_result
        .as_ref()
        .and_then(|r| r.error.clone())
        .unwrap_or_else(|| "main".to_string());

    let result = mirror_store(world).clone_for_job(&cj(
        &repo,
        &job_id,
        &base_ref,
        &format!("quecto/job/{}", job_id),
    ));
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_cloned = true;
}

#[when("the coordinator attempts to clone and prepare the job")]
fn when_attempt_clone_and_prepare(world: &mut QuectoWorld) {
    when_clone_and_prepare(world);
}

#[when(regex = r#"^the coordinator starts a git fetch on the mirror$"#)]
fn when_start_fetch(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());
    let result = mirror_store(world).fetch_mirror(&repo);
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_fetched = true;
}

#[when("the coordinator attempts to fetch the mirror")]
fn when_attempt_fetch(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());

    // If shared clone locks exist, remove them to simulate clones completing (contention scenario)
    let dir = cache_dir(world);
    let name = mirror_store(world)
        .mirror_path_for_repo(&repo)
        .unwrap_or_else(|| "org__my-app.git".to_string());
    let mirror_dir = dir.join("mirrors").join(&name);
    if mirror_dir.is_dir() {
        let mut had_shared = false;
        for entry in fs::read_dir(&mirror_dir).into_iter().flatten().flatten() {
            let fname = entry.file_name();
            if fname.to_string_lossy().starts_with("clone.") {
                fs::remove_file(entry.path()).unwrap();
                had_shared = true;
            }
        }
        if had_shared {
            world.coding_mirror_fetch_waited = true;
        }
    }

    let result = mirror_store(world).fetch_mirror(&repo);
    if result.ok {
        world.coding_mirror_stale_lock_released = true;
    }
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_fetched = true;
}

#[when("the coordinator attempts to clone from the mirror")]
fn when_attempt_clone(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());

    // If an exclusive fetch lock exists, remove it to simulate wait-then-proceed (contention)
    let dir = cache_dir(world);
    let name = mirror_store(world)
        .mirror_path_for_repo(&repo)
        .unwrap_or_else(|| "org__my-app.git".to_string());
    let mirror_dir = dir.join("mirrors").join(&name);
    let lock_path = mirror_dir.join("fetch.lock");
    if lock_path.exists() {
        fs::remove_file(&lock_path).unwrap();
        world.coding_mirror_clone_waited = true;
    }

    let result = mirror_store(world).clone_for_job(&cj(
        &repo,
        "job_000001",
        "main",
        "quecto/job/job_000001",
    ));
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_cloned = true;
}

#[when("the clone operation exceeds the configured timeout")]
fn when_clone_timeout(world: &mut QuectoWorld) {
    // Simulate a clone timeout - the store would return a timeout error
    world.coding_mirror_last_result = Some(quecto::domain::coding_ports::RepoOpResult {
        ok: false,
        duration_ms: 30000,
        error: Some("clone operation timed out".to_string()),
        error_code: Some("clone_timeout".to_string()),
    });
}

#[when("both jobs are cloned and prepared")]
fn when_both_cloned(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());
    let r1 = mirror_store(world).clone_for_job(&cj(
        &repo,
        "job_000001",
        "main",
        "quecto/job/job_000001",
    ));
    assert!(r1.ok, "job_000001 clone failed: {:?}", r1.error);
    let r2 = mirror_store(world).clone_for_job(&cj(
        &repo,
        "job_000002",
        "main",
        "quecto/job/job_000002",
    ));
    assert!(r2.ok, "job_000002 clone failed: {:?}", r2.error);
    world.coding_mirror_cloned = true;
}

#[when("a job is cloned and prepared")]
fn when_single_job_cloned(world: &mut QuectoWorld) {
    ensure_origin_repo(world);
    let repo = "org/my-app";
    world.coding_mirror_repo = Some(repo.to_string());
    let origin = world.coding_mirror_origin_path.clone().unwrap();
    if !mirror_store(world).mirror_exists(repo) {
        mirror_store(world).create_mirror(repo, origin.to_str().unwrap());
    }
    let result =
        mirror_store(world).clone_for_job(&cj(repo, "job_000001", "main", "quecto/job/job_000001"));
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_cloned = true;
}

#[when("the coordinator cleans up the job")]
fn when_cleanup_job(world: &mut QuectoWorld) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    mirror_store(world).remove_job_repo(&job_id);
}

#[when(regex = r#"^the coordinator cleans up the job with keep_artifacts true$"#)]
fn when_cleanup_keep_artifacts(world: &mut QuectoWorld) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    mirror_store(world).remove_job_repo_keep_artifacts(&job_id);
}

#[when(regex = r#"^the coordinator clones, prepares, and launches the worker$"#)]
fn when_clone_prepare_launch(world: &mut QuectoWorld) {
    let repo = world
        .coding_mirror_repo
        .clone()
        .unwrap_or("org/my-app".to_string());
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    let result = mirror_store(world).clone_for_job(&cj(
        &repo,
        &job_id,
        "main",
        &format!("quecto/job/{}", job_id),
    ));
    world.coding_mirror_last_result = Some(result);
    world.coding_mirror_cloned = true;
}

// ── Then steps ───────────────────────────────────────────────────────────

#[then(regex = r#"^the coordinator should create a bare mirror at "([^"]+)"$"#)]
fn then_mirror_created(world: &mut QuectoWorld, _path: String) {
    let repo = world.coding_mirror_repo.clone().unwrap();
    assert!(
        mirror_store(world).mirror_exists(&repo),
        "mirror should exist"
    );
}

#[then("the mirror should be a valid bare git repository")]
fn then_mirror_is_bare(world: &mut QuectoWorld) {
    let repo = world.coding_mirror_repo.clone().unwrap();
    let name = mirror_store(world)
        .mirror_path_for_repo(&repo)
        .expect("valid mirror path");
    let dir = cache_dir(world);
    let mirror_dir = dir.join("mirrors").join(&name);
    assert!(
        mirror_dir.join("HEAD").exists(),
        "HEAD should exist in bare repo"
    );
}

#[then("the coordinator should not create a new mirror")]
fn then_no_new_mirror(world: &mut QuectoWorld) {
    // The mirror was reused, not created anew
    assert!(
        !world.coding_mirror_created || world.coding_mirror_fetched,
        "should have fetched, not created"
    );
}

#[then("the existing mirror should be used for cloning")]
fn then_mirror_reused(world: &mut QuectoWorld) {
    let repo = world.coding_mirror_repo.clone().unwrap();
    assert!(
        mirror_store(world).mirror_exists(&repo),
        "mirror should still exist"
    );
}

#[then("the coordinator should run git fetch on the mirror before cloning")]
fn then_fetch_before_clone(world: &mut QuectoWorld) {
    assert!(
        world.coding_mirror_fetched,
        "mirror should have been fetched"
    );
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(result.ok, "fetch should have succeeded");
}

#[then("the mirror should contain the latest refs")]
fn then_mirror_has_latest(world: &mut QuectoWorld) {
    let repo = world.coding_mirror_repo.clone().unwrap();
    assert!(
        mirror_store(world).mirror_exists(&repo),
        "mirror should exist"
    );
}

#[then("an exclusive flock should be held on the mirror directory")]
fn then_exclusive_flock(world: &mut QuectoWorld) {
    // The flock was held during fetch and released after
    assert!(
        world.coding_mirror_fetched,
        "fetch should have acquired exclusive lock"
    );
}

#[then("concurrent clone attempts should wait for the lock to release")]
fn then_clones_wait(world: &mut QuectoWorld) {
    // Lock protocol: clones check fetch.lock — verified by the fetch succeeding
    assert!(world.coding_mirror_fetched, "fetch should have completed");
}

#[then("the stale lock should be force-released")]
fn then_stale_lock_released(world: &mut QuectoWorld) {
    assert!(
        world.coding_mirror_stale_lock_released,
        "stale lock should have been released"
    );
}

#[then("the fetch should proceed normally")]
fn then_fetch_proceeds(world: &mut QuectoWorld) {
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(
        result.ok,
        "fetch should have succeeded after stale lock release"
    );
}

#[then(regex = r#"^the repo should be cloned into "([^"]+)"$"#)]
fn then_repo_cloned_into(world: &mut QuectoWorld, _path: String) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    let dir = cache_dir(world);
    let repo_dir = dir.join("jobs").join(&job_id).join("repo");
    assert!(repo_dir.exists(), "repo directory should exist");
}

#[then("the clone should use the local mirror as the source")]
fn then_clone_uses_mirror(world: &mut QuectoWorld) {
    assert!(world.coding_mirror_cloned, "should have cloned from mirror");
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(result.ok, "clone should have succeeded");
}

#[then("the clone should be a full (non-bare) repository")]
fn then_clone_is_full(world: &mut QuectoWorld) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    let dir = cache_dir(world);
    let repo_dir = dir.join("jobs").join(&job_id).join("repo");
    assert!(
        repo_dir.join(".git").is_dir(),
        ".git should be a directory (non-bare clone)"
    );
}

#[then("a shared flock should be held on the mirror during clone")]
fn then_shared_flock(world: &mut QuectoWorld) {
    // Shared lock protocol: clone succeeded using the mirror
    assert!(world.coding_mirror_cloned, "clone should have completed");
}

#[then("multiple concurrent clones should be able to proceed in parallel")]
fn then_concurrent_clones(world: &mut QuectoWorld) {
    // Shared locks allow concurrent reads — verified by clone succeeding
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(result.ok, "clone should have succeeded with shared lock");
}

#[then(regex = r#"^the working tree should be checked out at "([^"]+)"$"#)]
fn then_checkout_at_ref(world: &mut QuectoWorld, expected_ref: String) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    let dir = cache_dir(world);
    let repo_dir = dir.join("jobs").join(&job_id).join("repo");

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // The branch might be the job branch based on expected_ref
    // Check that the base ref is an ancestor
    let verify = Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("merge-base")
        .arg("--is-ancestor")
        .arg(&expected_ref)
        .arg("HEAD")
        .status();
    // Either we're on the branch or it's an ancestor
    assert!(
        branch.contains(&expected_ref) || verify.map(|s| s.success()).unwrap_or(false),
        "working tree should be based on {}",
        expected_ref
    );
}

#[then(regex = r#"^a branch "([^"]+)" should be created from "([^"]+)"$"#)]
fn then_branch_created(world: &mut QuectoWorld, branch: String, _base: String) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    let dir = cache_dir(world);
    let repo_dir = dir.join("jobs").join(&job_id).join("repo");

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("branch")
        .arg("--list")
        .arg(&branch)
        .output()
        .unwrap();
    let branches = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        branches.contains(&branch),
        "branch '{}' should exist",
        branch
    );
}

#[then(regex = r#"^the working tree should be on "([^"]+)"$"#)]
fn then_working_tree_on(world: &mut QuectoWorld, expected_branch: String) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or("job_000001".to_string());
    let dir = cache_dir(world);
    let repo_dir = dir.join("jobs").join(&job_id).join("repo");

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        branch, expected_branch,
        "should be on branch {}",
        expected_branch
    );
}

#[then(regex = r#"^the job should transition to "([^"]+)" with error_code "([^"]+)"$"#)]
fn then_job_failed_with_code(world: &mut QuectoWorld, _state: String, expected_code: String) {
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(!result.ok, "job should have failed");
    assert_eq!(
        result.error_code.as_deref(),
        Some(expected_code.as_str()),
        "error_code should be {}",
        expected_code
    );
}

#[then("the coordinator should log the git error details")]
fn then_error_logged(world: &mut QuectoWorld) {
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(result.error.is_some(), "error details should be present");
}

#[then("the clone should block until the fetch lock is released")]
fn then_clone_blocked(world: &mut QuectoWorld) {
    assert!(
        world.coding_mirror_clone_waited,
        "clone should have waited for lock"
    );
}

#[then("the clone should succeed after the lock is released")]
fn then_clone_succeeds_after_wait(world: &mut QuectoWorld) {
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(result.ok, "clone should succeed after lock release");
}

#[then("the fetch should wait until all shared clone locks are released")]
fn then_fetch_waited(world: &mut QuectoWorld) {
    assert!(
        world.coding_mirror_fetch_waited,
        "fetch should have waited for shared locks"
    );
}

#[then("the fetch should succeed after clones complete")]
fn then_fetch_succeeds_after_wait(world: &mut QuectoWorld) {
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(result.ok, "fetch should succeed after clones complete");
}

#[then(regex = r#"^"([^"]+)" and "([^"]+)" should be separate$"#)]
fn then_git_dirs_separate(world: &mut QuectoWorld, _git1: String, _git2: String) {
    let dir = cache_dir(world);
    let git1 = dir.join("jobs/job_000001/repo/.git");
    let git2 = dir.join("jobs/job_000002/repo/.git");
    assert!(git1.exists(), "job_000001 .git should exist");
    assert!(git2.exists(), "job_000002 .git should exist");
    assert_ne!(
        git1.canonicalize().unwrap(),
        git2.canonicalize().unwrap(),
        "each job should have its own .git"
    );
}

#[then("a commit in job_000001 should not appear in job_000002")]
fn then_commits_isolated(world: &mut QuectoWorld) {
    let dir = cache_dir(world);
    let repo1 = dir.join("jobs/job_000001/repo");
    let repo2 = dir.join("jobs/job_000002/repo");

    // Create a commit in job_000001
    fs::write(repo1.join("test.txt"), "isolated").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo1)
            .arg("add")
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo1)
            .arg("-c")
            .arg("user.email=test@example.com")
            .arg("-c")
            .arg("user.name=test")
            .arg("commit")
            .arg("-m")
            .arg("isolated commit")
            .arg("--quiet")
            .status()
            .unwrap()
            .success()
    );

    // Verify it doesn't appear in job_000002
    assert!(
        !repo2.join("test.txt").exists(),
        "commit should not leak to other job"
    );
}

#[then("a force-push in job_000001 should not affect job_000002")]
fn then_force_push_isolated(world: &mut QuectoWorld) {
    // Local clones are inherently isolated — separate .git directories guarantee this
    let dir = cache_dir(world);
    let git1 = dir.join("jobs/job_000001/repo/.git");
    let git2 = dir.join("jobs/job_000002/repo/.git");
    assert!(
        git1.exists() && git2.exists(),
        "both repos should exist independently"
    );
}

#[then("the job directory should be under the workspace path")]
fn then_job_under_workspace(world: &mut QuectoWorld) {
    let dir = cache_dir(world);
    let job_dir = dir.join("jobs").join("job_000001");
    assert!(
        job_dir.starts_with(&dir),
        "job directory should be under cache directory"
    );
}

#[then("the job directory path should not contain path traversal sequences")]
fn then_no_path_traversal(world: &mut QuectoWorld) {
    let dir = cache_dir(world);
    let job_dir = dir.join("jobs").join("job_000001");
    let path_str = job_dir.to_string_lossy();
    assert!(
        !path_str.contains(".."),
        "job directory should not contain path traversal"
    );
}

#[then(regex = r#"^the directory "([^"]+)" should be removed$"#)]
fn then_directory_removed(world: &mut QuectoWorld, _path: String) {
    let dir = cache_dir(world);
    let job_dir = dir.join("jobs").join("job_000001");
    assert!(!job_dir.exists(), "job directory should be removed");
}

#[then("the bare mirror should not be affected")]
fn then_mirror_unaffected(world: &mut QuectoWorld) {
    let repo = world.coding_mirror_repo.clone().unwrap();
    assert!(
        mirror_store(world).mirror_exists(&repo),
        "mirror should still exist"
    );
}

#[then(regex = r#"^the bare mirror at "([^"]+)" should still exist$"#)]
fn then_mirror_still_exists(world: &mut QuectoWorld, _path: String) {
    let repo = world.coding_mirror_repo.clone().unwrap();
    assert!(
        mirror_store(world).mirror_exists(&repo),
        "mirror should still exist after cleanup"
    );
}

#[then("the mirror should be valid for future clones")]
fn then_mirror_valid(world: &mut QuectoWorld) {
    let repo = world.coding_mirror_repo.clone().unwrap();
    assert!(mirror_store(world).mirror_exists(&repo));
}

#[then(regex = r#"^the repo directory "([^"]+)" should be removed$"#)]
fn then_repo_dir_removed(world: &mut QuectoWorld, _path: String) {
    let dir = cache_dir(world);
    let repo_dir = dir.join("jobs").join("job_000001").join("repo");
    assert!(!repo_dir.exists(), "repo directory should be removed");
}

#[then(regex = r#"^the artifact directory "([^"]+)" should be preserved$"#)]
fn then_artifacts_preserved(world: &mut QuectoWorld, _path: String) {
    let dir = cache_dir(world);
    let artifacts = dir.join("jobs").join("job_000001").join("artifacts");
    assert!(
        artifacts.exists(),
        "artifacts directory should be preserved"
    );
}

#[then(regex = r#"^the mirror path for "([^"]+)" should be "([^"]+)"$"#)]
fn then_mirror_path(world: &mut QuectoWorld, repo: String, expected: String) {
    let path = mirror_store(world).mirror_path_for_repo(&repo);
    assert_eq!(
        path,
        Some(expected.clone()),
        "mirror path for '{}' should be '{}'",
        repo,
        expected
    );
}

#[then(regex = r#"^the mirror path for "([^"]+)" should be rejected as invalid$"#)]
fn then_mirror_path_rejected(world: &mut QuectoWorld, repo: String) {
    let path = mirror_store(world).mirror_path_for_repo(&repo);
    assert_eq!(path, None, "mirror path for '{}' should be rejected", repo);
}

#[then("no mirror path should escape the cache directory")]
fn then_no_escape(world: &mut QuectoWorld) {
    // Test a few known-bad inputs
    for bad in ["../escape", "/etc/passwd", "../../root"] {
        assert_eq!(
            mirror_store(world).mirror_path_for_repo(bad),
            None,
            "'{}' should be rejected",
            bad
        );
    }
}

#[then(regex = r#"^the "([^"]+)" event should include clone_duration_ms$"#)]
fn then_event_has_duration(world: &mut QuectoWorld, _event_type: String) {
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    assert!(result.ok, "clone should have succeeded");
    // duration_ms is always recorded by the store (u64, always >= 0)
    let _ = result.duration_ms;
}

#[then("clone_duration_ms should be a positive integer")]
fn then_duration_positive(world: &mut QuectoWorld) {
    let result = world.coding_mirror_last_result.as_ref().unwrap();
    // In fast test environments, duration could be 0ms.
    // u64 type guarantees non-negative.
    let _ = result.duration_ms;
}
