use super::*;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use std::path::Path;
use std::process::Command;

/// git, isolated from the user's configuration and from any repository the
/// test itself runs inside (a hook's GIT_DIR and friends).
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A repository shaped like one with a committed standard container: its
/// `.quecto/` is tracked.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(dir.path().join(".quecto/containers/standard")).unwrap();
    std::fs::write(
        dir.path().join(".quecto/containers/standard/Containerfile"),
        "FROM x\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("app.py"), "code\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    dir
}

fn context(checkout: &Path) -> SwarmContext {
    SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    }
}

/// Create a run as the container's creator does: make the store's directory,
/// then create the run there.
fn created_run(checkout: &Path) -> (SwarmContext, String) {
    let context = context(checkout);
    std::fs::create_dir_all(context.database().parent().unwrap()).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let pid = std::process::id();
    context
        .create_run(
            &serde_json::json!({"goal":"g", "constraints":[], "criteria":[{"id":"t","kind":"command","description":"pass"}], "member_limit":2, "deadline":deadline}),
            &crate::domain::swarm::ProcessIdentity {
                pid,
                started: super::super::swarm_bridge::process_start(pid).unwrap(),
            },
            None,
        )
        .unwrap();
    let id = run_id(&context);
    assert_ne!(id, "null", "the run has an id");
    (context, id)
}

/// The run's id, as the store reports it.
fn run_id(context: &SwarmContext) -> String {
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("_status", serde_json::json!([]))
    })
    .unwrap()["id"]
        .to_string()
}

/// The board belongs to the checkout's own git directory whenever it has
/// one (a linked worktree's too); only a checkout with none keeps it in the
/// work tree.
#[test]
fn the_board_lives_in_the_checkouts_own_git_directory() {
    let repo = repository();
    assert_eq!(
        located(repo.path()),
        repo.path().join(".git/quecto/swarm.sqlite")
    );
    let worktrees = tempfile::tempdir().unwrap();
    let linked = worktrees.path().join("wt");
    git(
        repo.path(),
        &["worktree", "add", "-q", linked.to_str().unwrap()],
    );
    let own = repo.path().join(".git/worktrees/wt");
    assert_eq!(
        std::fs::canonicalize(located(&linked).parent().unwrap().parent().unwrap()).unwrap(),
        std::fs::canonicalize(own).unwrap()
    );
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        located(plain.path()),
        plain.path().join(".quecto/swarm.sqlite")
    );
    std::fs::write(plain.path().join(".git"), "gitdir: /nowhere\n").unwrap();
    assert_eq!(
        located(plain.path()),
        plain.path().join(".quecto/swarm.sqlite")
    );
}

/// #2145: the git commands agents run on a checkout neither delete nor
/// replace the live board, even where a branch tracks a stale
/// `.quecto/swarm.sqlite`.
#[test]
fn the_board_survives_every_git_command_agents_run() {
    let repo = repository();
    git(repo.path(), &["checkout", "-q", "-b", "stale"]);
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "stale board").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "committed a board"]);
    git(repo.path(), &["checkout", "-q", "main"]);
    let (context, id) = created_run(repo.path());
    std::fs::write(repo.path().join("app.py"), "changed\n").unwrap();
    for command in [
        &["stash", "-u"][..],
        &["clean", "-fdx"],
        &["checkout", "-q", "stale"],
        &["checkout", "-q", "main"],
        &["merge", "-q", "--no-edit", "stale"],
        &["reset", "-q", "--hard", "HEAD~1"],
        &["clean", "-fdX"],
    ] {
        git(repo.path(), command);
        let status = context.control_status();
        assert!(status.is_ok(), "after git {command:?}: {status:?}");
        assert_eq!(run_id(&context), id, "after git {command:?}");
    }
}

/// PR #2148 swarm review: in a linked worktree the board lives in that
/// worktree's git directory, so `git clean -fdx` there leaves it.
#[test]
fn a_linked_worktrees_board_survives_git_clean() {
    let repo = repository();
    let worktrees = tempfile::tempdir().unwrap();
    let linked = worktrees.path().join("wt");
    git(
        repo.path(),
        &["worktree", "add", "-q", linked.to_str().unwrap()],
    );
    let (context, id) = created_run(&linked);
    git(&linked, &["clean", "-fdx"]);
    assert_eq!(run_id(&context), id);
}

/// A repository that tracks a stale board: the location follows the layout,
/// so the stale file is never taken for this container's board.
#[test]
fn a_stale_board_committed_in_the_repository_is_never_adopted() {
    let repo = repository();
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "stale board").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "committed a board"]);
    assert!(!context(repo.path()).run_created().unwrap());
}

/// PR #2148 swarm review: a layout that changes mid-run never moves a live
/// board. A checkout with no git directory gains one (and a store directory
/// in it): this process keeps its board; a new process finds no store there
/// and its member is refused rather than given another board.
#[test]
fn a_board_found_is_pinned_and_a_later_layout_change_fails_closed() {
    let plain = tempfile::tempdir().unwrap();
    let (context, id) = created_run(plain.path());
    git(plain.path(), &["init", "-q"]);
    std::fs::create_dir_all(plain.path().join(".git/quecto")).unwrap();
    assert_eq!(
        context.database(),
        plain.path().join(".quecto/swarm.sqlite")
    );
    assert_eq!(run_id(&context), id);
    // However the checkout is spelled, the pin holds.
    let links = tempfile::tempdir().unwrap();
    let spelled = links.path().join("checkout");
    std::os::unix::fs::symlink(plain.path(), &spelled).unwrap();
    let respelled = SwarmContext {
        checkout: spelled,
        ..context.clone()
    };
    assert_eq!(
        respelled.database(),
        plain.path().join(".quecto/swarm.sqlite")
    );
    forget_pin(plain.path());
    let newcomer = SwarmContext {
        member: "late".into(),
        ..context.clone()
    };
    assert_eq!(
        newcomer.database(),
        plain.path().join(".git/quecto/swarm.sqlite")
    );
    let refused = super::super::swarm_lifecycle::join_current_process(
        &newcomer,
        None,
        super::super::swarm_bridge::Participation::none(),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("store missing"), "{refused}");
}

/// A board removed from under a run fails as missing where it was: it is
/// never replaced by another file that happens to exist.
#[test]
fn a_removed_board_fails_as_missing_where_it_was() {
    let repo = repository();
    let (context, _) = created_run(repo.path());
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "some other board").unwrap();
    std::fs::remove_dir_all(repo.path().join(".git/quecto")).unwrap();
    let error = context.control_status().unwrap_err().to_string();
    assert!(
        error.contains(&format!(
            "coordination store missing at {}",
            repo.path().join(".git/quecto/swarm.sqlite").display()
        )),
        "{error}"
    );
}

/// In a git checkout the host reads only the board in the git directory: a
/// work-tree `.quecto/swarm.sqlite` (committed, left from before #2145,
/// planted, or pinned by a member before a `git init`) is never taken for a
/// run — but while no board is where the layout places it, it is never
/// taken for "no swarm" either: the read fails, so the environment is kept
/// (#2206). A board removed from the git directory is never replaced by
/// the other file.
#[test]
fn the_host_never_reads_a_work_tree_board_in_a_git_checkout() {
    let repo = repository();
    let elsewhere = tempfile::tempdir().unwrap();
    let (board, _) = created_run(elsewhere.path());
    std::fs::copy(board.database(), repo.path().join(".quecto/swarm.sqlite")).unwrap();
    let hosted = super::super::swarm_bridge::HostedStore::at(
        repo.path().to_path_buf(),
        crate::composition::swarm::swarm_board(),
    );
    let displaced = hosted.hosted_run().unwrap_err().to_string();
    assert!(
        displaced.contains("is not where the checkout's layout now places it"),
        "{displaced}"
    );
    let (_, id) = created_run(repo.path());
    let run = hosted
        .hosted_run()
        .unwrap()
        .expect("the git directory's board");
    assert_eq!(serde_json::Value::from(run.id).to_string(), id);
    std::fs::remove_dir_all(repo.path().join(".git/quecto")).unwrap();
    assert!(hosted.hosted_run().is_err(), "never the other file");
}

/// The host opens a store only where it really is inside the checkout: a
/// `.git/quecto` linked elsewhere is refused.
#[test]
fn the_host_refuses_a_store_linked_outside_its_checkout() {
    let repo = repository();
    let elsewhere = tempfile::tempdir().unwrap();
    let (planted, _) = created_run(elsewhere.path());
    std::os::unix::fs::symlink(
        planted.database().parent().unwrap(),
        repo.path().join(".git/quecto"),
    )
    .unwrap();
    let hosted = super::super::swarm_bridge::HostedStore::at(
        repo.path().to_path_buf(),
        crate::composition::swarm::swarm_board(),
    );
    let error = hosted.hosted_run().unwrap_err().to_string();
    assert!(error.contains("is outside its checkout"), "{error}");
}

/// A store removed from under a run names its path, not "unavailable or
/// contended".
#[test]
fn a_missing_store_names_its_path() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    std::fs::remove_file(context.database()).unwrap();
    let error = context.control_status().unwrap_err().to_string();
    assert!(
        error.contains(&format!(
            "coordination store missing at {}: it was deleted while the run was live",
            context.database().display()
        )),
        "{error}"
    );
    assert!(!error.contains("contended"), "{error}");
}

/// Where no board is, at every place (each answers "no such file"), the
/// host finds no run and does not try to open one. Anything else at a
/// board's place — a directory — is not "no board" (#2206 round 2): the
/// read fails, so the environment is kept, and nothing is opened.
#[test]
fn the_host_finds_no_run_where_there_is_no_store_file() {
    let repo = repository();
    let hosted = super::super::swarm_bridge::HostedStore::at(
        repo.path().to_path_buf(),
        crate::composition::swarm::swarm_board(),
    );
    assert!(hosted.hosted_run().unwrap().is_none());
    std::fs::create_dir_all(repo.path().join(".git/quecto/swarm.sqlite")).unwrap();
    let error = hosted.hosted_run().unwrap_err().to_string();
    assert!(error.contains("is not a regular file"), "{error}");
}

/// The host pins nothing: once a checkout's layout places a new board (here
/// a checkout that gained a git directory and a new run), the host reads
/// that board, not the one it read before.
#[test]
fn the_host_follows_the_layout_on_every_read() {
    let plain = tempfile::tempdir().unwrap();
    let (_, first) = created_run(plain.path());
    let hosted = super::super::swarm_bridge::HostedStore::at(
        plain.path().to_path_buf(),
        crate::composition::swarm::swarm_board(),
    );
    let run = hosted.hosted_run().unwrap().expect("the first board");
    assert_eq!(serde_json::Value::from(run.id).to_string(), first);
    git(plain.path(), &["init", "-q"]);
    // The new board is put in place directly: nothing here touches a pin.
    let elsewhere = tempfile::tempdir().unwrap();
    let (board, second) = created_run(elsewhere.path());
    assert_ne!(first, second);
    std::fs::create_dir_all(plain.path().join(".git/quecto")).unwrap();
    std::fs::copy(
        board.database(),
        plain.path().join(".git/quecto/swarm.sqlite"),
    )
    .unwrap();
    let run = hosted.hosted_run().unwrap().expect("the second board");
    assert_eq!(serde_json::Value::from(run.id).to_string(), second);
}

/// Preparing a checkout for a join makes the store's directory, in the git
/// directory.
#[test]
fn preparing_a_checkout_makes_the_store_directory() {
    let repo = repository();
    super::super::swarm_lifecycle::prepare_checkout(&context(repo.path())).unwrap();
    assert!(repo.path().join(".git/quecto").is_dir());
}

// ─── #2206: a board the layout no longer names is never "no swarm" ─────────

fn sandbox_record(workspace: &Path) -> crate::domain::environment_registry::EnvironmentRecord {
    use crate::domain::environment_registry::{
        EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
    };
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-1".into(),
        environment_uuid: "u1".into(),
        name: None,
        workspace_path: workspace.to_path_buf(),
        repository: String::new(),
        script_name: "standard".into(),
        retained_exec_argv: vec!["exec.sh".into()],
        retained_kill_argv: vec!["kill.sh".into()],
        retained_cleanup_argv: vec!["cleanup.sh".into()],
        retained_inspect_argv: vec![],
        members: vec!["coord".into()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({"checkout": workspace.display().to_string()}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

/// Records the scripts the finalizer runs.
struct Scripts(std::sync::Mutex<Vec<&'static str>>);

impl crate::application::environments::ports::EnvironmentProcessCommands for Scripts {
    fn run_retained_inspect<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> crate::application::environments::ports::PortFuture<'a, Result<serde_json::Value, String>>
    {
        Box::pin(async { Ok(serde_json::json!({})) })
    }

    fn run_retained_kill<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> crate::application::environments::ports::PortFuture<'a, Result<(), String>> {
        self.0.lock().unwrap().push("kill");
        Box::pin(async { Ok(()) })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> crate::application::environments::ports::PortFuture<'a, Result<(), String>> {
        self.0.lock().unwrap().push("cleanup");
        Box::pin(async { Ok(()) })
    }

    fn observe_liveness<'a>(
        &'a self,
        _: &'a crate::domain::environment_registry::EnvironmentRecord,
    ) -> crate::application::environments::ports::PortFuture<
        'a,
        crate::application::environments::dto::EnvironmentLiveness,
    > {
        Box::pin(async { panic!("a final-member teardown never asks liveness") })
    }
}

/// The #2206 round-1 probe: an agent's `git init` in a sandbox checkout (no
/// clone) moves where the layout places the board, while the coordinator
/// keeps running on the one it pinned. The host must read that as an
/// unreadable store — never as a plain container — so neither the owner's
/// end nor a harness shutdown destroys the live swarm's box.
#[test]
fn a_live_swarm_whose_layout_changed_is_never_taken_for_a_plain_container() {
    use crate::application::environments::ports::HostedSwarmRunInspection;
    use crate::application::environments::use_cases::FinalizeEnvironmentMember;
    use crate::domain::environment_registry::{EnvironmentRegistry, EnvironmentStatus};
    use crate::domain::environment_retention::{
        MemberFinalizeMode, SwarmRunObservation, ends_plain_environment,
    };
    use crate::infrastructure::tools::environment_commands::HostedStoreObservation;
    for mode in [
        MemberFinalizeMode::OwnerEnd,
        MemberFinalizeMode::ParentKill,
        MemberFinalizeMode::Exit,
    ] {
        let state = tempfile::tempdir().unwrap();
        let workspace = state.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let (_coordinator, _) = created_run(&workspace);
        let record = sandbox_record(&workspace);
        let before = HostedStoreObservation::new(crate::composition::swarm::swarm_board())
            .inspect_hosted_run(&record);
        assert!(
            matches!(before, SwarmRunObservation::Run(ref run) if run.created()),
            "{before:?}"
        );
        git(&workspace, &["init", "-q"]);
        let after = HostedStoreObservation::new(crate::composition::swarm::swarm_board())
            .inspect_hosted_run(&record);
        assert!(
            matches!(after, SwarmRunObservation::Unreadable(_)),
            "{mode:?}: {after:?}"
        );
        assert!(!ends_plain_environment(mode, &after));

        let registry = EnvironmentRegistry::new();
        registry.commit(record);
        let scripts = std::sync::Arc::new(Scripts(Default::default()));
        let finalize = FinalizeEnvironmentMember::new(
            registry.clone(),
            scripts.clone(),
            std::sync::Arc::new(HostedStoreObservation::new(
                crate::composition::swarm::swarm_board(),
            )),
        );
        futures::executor::block_on(finalize.finalize_member("C1", "coord", None, mode));
        assert!(
            scripts.0.lock().unwrap().is_empty(),
            "{mode:?}: no cleanup, no kill"
        );
        assert_eq!(
            registry.get("C1").map(|record| record.status),
            Some(EnvironmentStatus::Retained),
            "{mode:?}: the box is kept"
        );
    }
}

/// With no board at either location a sandbox checkout is plain, and a
/// board where the layout places it is read as before.
#[test]
fn a_displaced_board_is_found_only_where_the_layout_does_not_place_it() {
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(displaced_board(plain.path()), None);
    let (_context, _) = created_run(plain.path());
    assert_eq!(
        displaced_board(plain.path()),
        None,
        "it is where the layout says"
    );
    git(plain.path(), &["init", "-q"]);
    assert_eq!(
        displaced_board(plain.path()),
        Some(plain.path().join(".quecto/swarm.sqlite"))
    );
    assert!(!matches!(
        board_presence(plain.path()),
        BoardPresence::Absent
    ));
}

/// #2206 round 3: a board where the layout places it now does not hide a
/// board at a place it no longer names. When the current board holds only
/// the bootstrap placeholder, the other may hold the live run a member
/// pinned: the read fails, so the box is kept. A created run at the
/// current place is read as before.
#[test]
fn a_current_placeholder_board_never_hides_a_displaced_one() {
    let plain = tempfile::tempdir().unwrap();
    let (_coordinator, _) = created_run(plain.path());
    git(plain.path(), &["init", "-q"]);
    // A new member's bootstrap writes a placeholder where the layout now
    // places the board.
    let newcomer = context(plain.path());
    forget_pin(plain.path());
    std::fs::create_dir_all(plain.path().join(".git/quecto")).unwrap();
    newcomer
        .call(
            "_bootstrap",
            serde_json::json!([std::process::id(), "1", null]),
        )
        .unwrap();
    assert!(matches!(
        board_presence(plain.path()),
        BoardPresence::Current {
            displaced: Some(_),
            ..
        }
    ));
    let hosted = super::super::swarm_bridge::HostedStore::at(
        plain.path().to_path_buf(),
        crate::composition::swarm::swarm_board(),
    );
    let error = hosted.hosted_run().unwrap_err().to_string();
    assert!(
        error.contains("holds no created run, but another board exists"),
        "{error}"
    );

    // A created run at the current place is read, displaced board or not.
    let repo = repository();
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "stale").unwrap();
    let (_, id) = created_run(repo.path());
    let hosted = super::super::swarm_bridge::HostedStore::at(
        repo.path().to_path_buf(),
        crate::composition::swarm::swarm_board(),
    );
    let run = hosted.hosted_run().unwrap().expect("the created run");
    assert_eq!(serde_json::Value::from(run.id).to_string(), id);
}
