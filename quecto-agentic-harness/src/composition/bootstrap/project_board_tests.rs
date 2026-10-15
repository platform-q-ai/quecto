use super::*;
use crate::interface::tools::project_board::run;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

fn sh(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", "/nonexistent"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .envs([("GIT_AUTHOR_NAME", "T"), ("GIT_AUTHOR_EMAIL", "t@x.io")])
        .envs([
            ("GIT_COMMITTER_NAME", "T"),
            ("GIT_COMMITTER_EMAIL", "t@x.io"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A clone of a bare remote whose committer is configured in the clone.
fn repository() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    sh(
        dir.path(),
        &["init", "-q", "--bare", "-b", "main", "remote.git"],
    );
    sh(dir.path(), &["clone", "-q", "remote.git", "work"]);
    let work = dir.path().join("work");
    sh(&work, &["config", "user.name", "Ada"]);
    sh(&work, &["config", "user.email", "ada@example.com"]);
    (dir, work)
}

fn environment(repo: &Path, access: Access) -> ProjectBoardEnvironment {
    let vars: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), repo.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    ProjectBoardEnvironment {
        repo: repo.to_path_buf(),
        remote: "origin".into(),
        access,
        vars,
        cancelled: None,
        launcher: None,
    }
}

/// The real graph over a real remote: bootstrap, create, list, show, from
/// the index and the git store, as the committer the clone configures.
#[test]
fn the_board_runs_end_to_end_over_git_and_the_index() {
    let (_dir, work) = repository();
    let handles = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    let ops = |request: Value| run(&handles.operations, &request.to_string());
    let bootstrap = ops(json!({"op": "bootstrap"}));
    assert_eq!(
        (&bootstrap["ok"], &bootstrap["created"]),
        (&json!(true), &json!(true)),
        "{bootstrap}"
    );
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    assert_eq!(
        created["clock"]["referenced"],
        json!(false),
        "a local remote has no outside time"
    );
    let claimed = ops(json!({"op": "move", "task": "a", "to": "claimed"}));
    assert_eq!(
        claimed["task"]["claim"]["holder"],
        json!({"name": "Ada", "email": "ada@example.com"})
    );
    let listed = ops(json!({"op": "list"}));
    assert_eq!(listed["tasks"][0]["status"], json!("claimed"), "{listed}");
    let complete = ops(json!({"op": "complete", "task": "a"}));
    assert_eq!(
        complete["error"]["code"],
        json!("no_pull_request_host"),
        "{complete}"
    );
}

#[test]
fn a_read_needs_no_committer_and_a_write_says_none_is_configured() {
    let (_dir, work) = repository();
    let writer = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    run(&writer.operations, r#"{"op": "bootstrap"}"#);
    run(
        &writer.operations,
        r#"{"op": "create", "task": "a", "title": "A"}"#,
    );
    sh(&work, &["config", "--unset", "user.name"]);
    let reader = build_project_board_handles(&environment(&work, Access::Read)).unwrap();
    let listed = run(&reader.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["tasks"][0]["id"], json!("a"), "{listed}");
    let refused = build_project_board_handles(&environment(&work, Access::Write));
    assert!(
        matches!(&refused, Err(BuildError::NoCommitter(why)) if why.contains("user.name")),
        "{:?}",
        refused.as_ref().err()
    );
    let mut nowhere = environment(&work, Access::Read);
    nowhere.remote = "nowhere".into();
    assert!(matches!(
        build_project_board_handles(&nowhere),
        Err(BuildError::Unavailable(_))
    ));
}

/// A directory git cannot use as a repository fails with that cause, not
/// as one with no committer configured.
#[test]
fn a_directory_that_is_no_repository_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = environment(dir.path(), Access::Write);
    // Discovery stops at the temporary directory: a repository above it
    // (a stray `/tmp/.git`) is never found.
    let parent = dir.path().parent().unwrap().as_os_str().to_owned();
    env.vars.push(("GIT_CEILING_DIRECTORIES".into(), parent));
    let refused = build_project_board_handles(&env);
    assert!(
        matches!(&refused, Err(BuildError::Unavailable(why)) if why.contains("rev-parse")),
        "{:?}",
        refused.as_ref().err()
    );
}

/// Only "not set" is a missing committer: git failing to read its
/// configuration, or a repository git cannot run in, is the store's
/// failure, with its cause.
#[test]
fn a_committer_git_cannot_read_is_not_a_missing_one() {
    let (_dir, work) = repository();
    let config = work.join(".git/config");
    let kept = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{kept}\n[broken\n")).unwrap();
    let broken = build_project_board_handles(&environment(&work, Access::Write));
    assert!(
        matches!(&broken, Err(BuildError::Unavailable(why)) if why.contains("config")),
        "{:?}",
        broken.as_ref().err()
    );
    std::fs::write(&config, kept).unwrap();
    let mut elsewhere = environment(&work, Access::Write);
    elsewhere.repo = work.join("no-such-directory");
    assert!(
        matches!(
            build_project_board_handles(&elsewhere),
            Err(BuildError::Unavailable(_))
        ),
        "a repository git cannot run in"
    );
    sh(&work, &["config", "user.email", "not an email"]);
    let refused = build_project_board_handles(&environment(&work, Access::Write));
    assert!(
        matches!(&refused, Err(BuildError::NoCommitter(_))),
        "{:?}",
        refused.as_ref().err()
    );
}

/// A remote on GitHub with no `gh` to ask reads no pull requests, and
/// says why: there is no gh on the PATH. One on no such host says that.
#[test]
fn a_github_remote_with_no_gh_says_so() {
    let dir = tempfile::tempdir().unwrap();
    // git's own directory, every program but gh, as a PATH might be.
    let no_gh = dir.path().join("bin");
    std::fs::create_dir(&no_gh).unwrap();
    let git_dir = which("git").parent().unwrap().to_path_buf();
    for entry in std::fs::read_dir(&git_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "gh" {
            std::os::unix::fs::symlink(entry.path(), no_gh.join(entry.file_name())).unwrap();
        }
    }
    let vars = vec![(OsString::from("PATH"), no_gh.into_os_string())];
    let missing = pull_request_states("https://github.com/o/r.git", &vars).err();
    assert!(
        matches!(&missing, Some(NoPullRequestHost::Unreachable(why)) if why.contains("no gh on the PATH")),
        "{missing:?}"
    );
    let elsewhere = pull_request_states("/srv/r.git", &vars).err();
    assert_eq!(elsewhere, Some(NoPullRequestHost::NotAPullRequestHost));
}

/// Where `program` is on this process's PATH.
fn which(program: &str) -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("no {program} on the PATH"))
}

/// The call's cancel flag reaches the composed git store (#2513): a write
/// of a cancelled call pushes nothing.
#[test]
fn a_cancelled_calls_write_lands_nothing() {
    let (_dir, work) = repository();
    let writer = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    run(&writer.operations, r#"{"op": "bootstrap"}"#);
    let mut cancelled = environment(&work, Access::Write);
    cancelled.cancelled = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        true,
    )));
    let handles = build_project_board_handles(&cancelled).unwrap();
    let refused = run(
        &handles.operations,
        r#"{"op": "create", "task": "a", "title": "A"}"#,
    );
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert!(refused.to_string().contains("cancelled"), "{refused}");
    let listed = run(&writer.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["tasks"], json!([]), "{listed}");
}

/// `environment` with git's command-scope configuration `settings`
/// (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`).
fn with_command_config(
    mut environment: ProjectBoardEnvironment,
    settings: &[(&str, &str)],
) -> ProjectBoardEnvironment {
    environment
        .vars
        .push(("GIT_CONFIG_COUNT".into(), settings.len().to_string().into()));
    for (n, (key, value)) in settings.iter().enumerate() {
        environment
            .vars
            .push((format!("GIT_CONFIG_KEY_{n}").into(), (*key).into()));
        environment
            .vars
            .push((format!("GIT_CONFIG_VALUE_{n}").into(), (*value).into()));
    }
    environment
}

/// The author and committer of the board branch's last commit on the
/// remote, `name <email>` each.
fn board_commit_identities(work: &Path) -> String {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>|%cn <%ce>", "quecto/board"])
        .current_dir(work.parent().unwrap().join("remote.git"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

/// A caller (the pocket relay) names the board's committer in git's
/// command scope, which outranks the clone's own `user.name` and
/// `user.email`: the board commits and claims as that identity.
#[test]
fn a_command_scope_identity_outranks_the_clones_own() {
    let (_dir, work) = repository();
    let pocket = with_command_config(
        environment(&work, Access::Write),
        &[
            ("user.name", "quecto pocket"),
            ("user.email", "pocket@quecto.invalid"),
        ],
    );
    let handles = build_project_board_handles(&pocket).unwrap();
    let ops = |request: Value| run(&handles.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    assert_eq!(
        board_commit_identities(&work),
        "quecto pocket <pocket@quecto.invalid>|quecto pocket <pocket@quecto.invalid>"
    );
    let claimed = ops(json!({"op": "move", "task": "a", "to": "claimed"}));
    assert_eq!(
        claimed["task"]["claim"]["holder"],
        json!({"name": "quecto pocket", "email": "pocket@quecto.invalid"}),
        "{claimed}"
    );
}

/// quecto's own container scripts set the identity beside a credential
/// helper (`create.sh`, `exec.sh`): the identity writes the board, the
/// helper is skipped, and nothing is warned of.
#[test]
fn the_container_scripts_identity_writes_the_board() {
    let (_dir, work) = repository();
    let container = with_command_config(
        environment(&work, Access::Write),
        &[
            ("user.name", "quecto pocket"),
            ("user.email", "pocket@quecto.invalid"),
            (
                "credential.https://github.com.helper",
                "!gh auth git-credential",
            ),
        ],
    );
    let mut warnings: Vec<String> = Vec::new();
    let handles = build_with_warnings(&container, &mut |line| warnings.push(line.into())).unwrap();
    assert_eq!(warnings, Vec::<String>::new());
    let ops = |request: Value| run(&handles.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    assert_eq!(
        board_commit_identities(&work),
        "quecto pocket <pocket@quecto.invalid>|quecto pocket <pocket@quecto.invalid>"
    );
}

/// A malformed command-scope set (here a count of 2 with one setting) is
/// dropped whole and the board carries on as before: a write, as the
/// clone's own committer, warns of it once in words that name the
/// variable; a read, which names no committer, warns of nothing.
#[test]
fn a_malformed_set_is_dropped_and_only_a_write_warns_of_it() {
    let (_dir, work) = repository();
    let plain = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    assert_eq!(
        run(&plain.operations, r#"{"op": "bootstrap"}"#)["ok"],
        json!(true)
    );
    let malformed = |access| {
        let mut environment = with_command_config(
            environment(&work, access),
            &[
                ("user.name", "quecto pocket"),
                ("user.email", "p@q.invalid"),
            ],
        );
        environment
            .vars
            .retain(|(name, _)| name != "GIT_CONFIG_KEY_1");
        environment
    };
    let mut warnings: Vec<String> = Vec::new();
    let writer = build_with_warnings(&malformed(Access::Write), &mut |line| {
        warnings.push(line.into())
    })
    .unwrap();
    assert_eq!(
        warnings,
        [
            "warning: project board: ignoring git's command-scope configuration \
          (GIT_CONFIG_COUNT): GIT_CONFIG_KEY_1 is not set"
        ]
    );
    let created = run(
        &writer.operations,
        r#"{"op": "create", "task": "a", "title": "A"}"#,
    );
    assert_eq!(created["ok"], json!(true), "{created}");
    assert_eq!(
        board_commit_identities(&work),
        "Ada <ada@example.com>|Ada <ada@example.com>"
    );
    let mut warnings: Vec<String> = Vec::new();
    let reader = build_with_warnings(&malformed(Access::Read), &mut |line| {
        warnings.push(line.into())
    })
    .unwrap();
    assert_eq!(warnings, Vec::<String>::new());
    let listed = run(&reader.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["tasks"][0]["id"], json!("a"), "{listed}");
}

/// git takes each command-scope key on its own: a write whose set names
/// `user.name` alone commits as that name with the clone's own email,
/// and warns that the set names half an identity; a read warns of
/// nothing.
#[test]
fn a_set_naming_half_an_identity_writes_with_the_clones_other_half() {
    let (_dir, work) = repository();
    let half = |access| {
        with_command_config(
            environment(&work, access),
            &[("user.name", "quecto pocket")],
        )
    };
    let mut warnings: Vec<String> = Vec::new();
    let writer =
        build_with_warnings(&half(Access::Write), &mut |line| warnings.push(line.into())).unwrap();
    assert_eq!(
        warnings,
        [
            "warning: project board: git's command-scope configuration (GIT_CONFIG_COUNT) \
          sets user.name but not user.email, which comes from the repository's own"
        ]
    );
    assert_eq!(
        run(&writer.operations, r#"{"op": "bootstrap"}"#)["ok"],
        json!(true)
    );
    assert_eq!(
        board_commit_identities(&work),
        "quecto pocket <ada@example.com>|quecto pocket <ada@example.com>"
    );
    let mut warnings: Vec<String> = Vec::new();
    let reader =
        build_with_warnings(&half(Access::Read), &mut |line| warnings.push(line.into())).unwrap();
    assert_eq!(warnings, Vec::<String>::new());
    let listed = run(&reader.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["ok"], json!(true), "{listed}");
}

/// Epic #2481 T2: the CLI's daily log is named by the UTC date.
#[test]
fn the_cli_log_key_is_the_utc_date() {
    let at = |secs| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    assert_eq!(cli_log_key(at(0)).as_deref(), Some("board-cli:1970-01-01"));
    // 2026-10-13T23:59:59Z, and one second later.
    assert_eq!(
        cli_log_key(at(1_791_935_999)).as_deref(),
        Some("board-cli:2026-10-13")
    );
    assert_eq!(
        cli_log_key(at(1_791_936_000)).as_deref(),
        Some("board-cli:2026-10-14")
    );
}

/// #2536 review: a time with no `YYYY-MM-DD` date (before the epoch, or
/// in year 10000 or later) has no key, and never panics.
#[test]
fn a_time_with_no_four_digit_utc_date_has_no_cli_log_key() {
    let epoch = std::time::UNIX_EPOCH;
    let before = epoch - std::time::Duration::from_secs(1);
    assert_eq!(cli_log_key(before), None);
    let last = epoch + std::time::Duration::from_secs(253_402_300_799);
    assert_eq!(cli_log_key(last).as_deref(), Some("board-cli:9999-12-31"));
    let year_10000 = epoch + std::time::Duration::from_secs(253_402_300_800);
    assert_eq!(cli_log_key(year_10000), None);
}

/// Epic #2481 T3 review round 1: the agent tool's recording takes the
/// switch its session decided, whatever the owner's global file says now.
#[test]
fn a_sessions_decided_switch_is_kept() {
    let base = tempfile::tempdir().unwrap();
    let config = json!({"telemetry": {"event_log": {"enabled": true}}});
    std::fs::write(base.path().join("config.json"), config.to_string()).unwrap();
    let log = AuditLog::open_sync(base.path(), "decided").unwrap();
    assert!(session_board_recording_decided(false, &log).is_none());
    std::fs::remove_file(base.path().join("config.json")).unwrap();
    assert!(session_board_recording_decided(true, &log).is_some());
}

/// Git records a committer's name byte for byte, so a name spaced with a
/// chat space (an ideographic space between family and given name) is no
/// identity the board can record: every write is refused as having no
/// committer, failing fast before any change; reads still work.
#[test]
fn a_committer_spaced_with_a_chat_space_fails_fast_for_every_write() {
    let (_dir, work) = repository();
    let writer = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    run(&writer.operations, r#"{"op": "bootstrap"}"#);
    sh(
        &work,
        &[
            "config",
            "user.name",
            "\u{5c71}\u{7530}\u{3000}\u{592a}\u{90ce}",
        ],
    );
    let refused = build_project_board_handles(&environment(&work, Access::Write));
    assert!(
        matches!(&refused, Err(BuildError::NoCommitter(why)) if why.contains("plain spaces")),
        "{:?}",
        refused.as_ref().err()
    );
    let reader = build_project_board_handles(&environment(&work, Access::Read)).unwrap();
    let listed = run(&reader.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["ok"], json!(true), "{listed}");
}

/// Q10: `QUECTO_BOARD_COMMITTER` changes who the board's commits and
/// claims are by, and nothing else git does: the clone keeps its own
/// identity for every other commit. A malformed value is refused, never
/// ignored for the clone's own.
#[test]
fn the_board_committer_override_changes_only_the_boards_commits() {
    let (_dir, work) = repository();
    let mut pocket = environment(&work, Access::Write);
    pocket.vars.push((
        "QUECTO_BOARD_COMMITTER".into(),
        "quecto pocket <pocket@quecto.invalid>".into(),
    ));
    let handles = build_project_board_handles(&pocket).unwrap();
    let ops = |request: Value| run(&handles.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    assert_eq!(
        board_commit_identities(&work),
        "quecto pocket <pocket@quecto.invalid>|quecto pocket <pocket@quecto.invalid>"
    );
    let claimed = ops(json!({"op": "move", "task": "a", "to": "claimed"}));
    assert_eq!(
        claimed["task"]["claim"]["holder"],
        json!({"name": "quecto pocket", "email": "pocket@quecto.invalid"})
    );
    // The clone's own configuration still names its own committer.
    let output = Command::new("git")
        .args(["config", "user.name"])
        .current_dir(&work)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env(
            "QUECTO_BOARD_COMMITTER",
            "quecto pocket <pocket@quecto.invalid>",
        )
        .envs([
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Ada");
    for malformed in [
        "quecto pocket",
        "<pocket@quecto.invalid>",
        "x <not an email>",
    ] {
        let mut wrong = environment(&work, Access::Write);
        wrong
            .vars
            .push(("QUECTO_BOARD_COMMITTER".into(), malformed.into()));
        match build_project_board_handles(&wrong) {
            Err(BuildError::NoCommitter(why)) => assert!(
                why.starts_with("QUECTO_BOARD_COMMITTER must be \"name <email>\""),
                "{why}"
            ),
            other => panic!("{malformed:?}: {:?}", other.err()),
        }
    }
}

/// Q10: the committer's email is the last `<...>` of the value, so a name
/// that holds ` <` is judged as the name (refused: a name holds no `<`),
/// never read as the start of the email; unset, it names none.
#[test]
fn the_board_committer_is_read_from_its_last_angle_brackets() {
    let named = |value: &str| {
        board_committer(&[(
            OsString::from("QUECTO_BOARD_COMMITTER"),
            OsString::from(value),
        )])
    };
    assert_eq!(
        named("quecto pocket <pocket@quecto.invalid>"),
        Some(Ok(Identity {
            name: "quecto pocket".into(),
            email: "pocket@quecto.invalid".into(),
        }))
    );
    let refused = named("a <b <c@d.example>").unwrap().unwrap_err();
    assert!(refused.contains("committer/name"), "{refused}");
    let bare = named("pocket@quecto.invalid").unwrap().unwrap_err();
    assert!(
        bare.ends_with("it names no email in angle brackets"),
        "{bare}"
    );
    assert_eq!(board_committer(&[]), None);
}

/// A session's swarms whose every launch fails while run 1's coordinator
/// is still seen running.
struct FailedButRunning;

impl crate::application::project_board::ports::RunLiveness for FailedButRunning {
    fn running_coordinator(
        &self,
        _: &crate::domain::project_board::value_objects::slug::Slug,
        run: u32,
    ) -> Option<String> {
        (run == 1).then(|| "7f3a".into())
    }
}

/// It keeps no swarm's board: every end is as a swarm that reported
/// nothing.
impl crate::application::project_board::ports::SwarmRunReports for FailedButRunning {
    fn ended_run(
        &self,
        _: &crate::domain::project_board::value_objects::slug::Slug,
        _: u32,
    ) -> Option<crate::application::project_board::fold_back::SwarmRunView> {
        None
    }
}

impl crate::application::project_board::ports::CoordinatorLaunch for FailedButRunning {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &crate::application::project_board::ports::CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<
        crate::application::project_board::ports::LaunchedCoordinator,
        crate::application::project_board::ports::LaunchRefused,
    > {
        Err(
            crate::application::project_board::ports::LaunchRefused::because(
                "the call was cancelled while the coordinator launched: the launch was dropped",
            ),
        )
    }
}

/// S4c review (L1): composition hands the start the session's liveness, so
/// a failed launch whose coordinator is still seen running undoes nothing
/// and names it, over the real board.
#[test]
fn a_composed_start_asks_whether_its_coordinator_still_runs() {
    let (_dir, work) = repository();
    let mut writing = environment(&work, Access::Write);
    writing.launcher = Some(std::sync::Arc::new(FailedButRunning));
    let handles = build_project_board_handles(&writing).unwrap();
    let ops = |request: Value| run(&handles.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    let proposed = ops(
        json!({"op": "propose_plan", "task": "a", "approach": "Build it.",
                              "items": [{"id": "schema", "title": "Schema"}]}),
    );
    let updated = proposed["plan"]["updated"].clone();
    let accepted = ops(json!({"op": "accept_plan", "task": "a", "version": 1,
                              "expected_updated": updated, "expected_accepted": null}));
    assert_eq!(accepted["ok"], json!(true), "{accepted}");
    let started = ops(json!({"op": "start", "task": "a", "plan_version": 1}));
    assert_eq!(
        (
            &started["error"]["code"],
            &started["error"]["still_running"],
            &started["error"]["run_state"],
            &started["error"]["released"]
        ),
        (
            &json!("launch_failed"),
            &json!("7f3a"),
            &json!("open"),
            &json!(false)
        ),
        "{started}"
    );
}
