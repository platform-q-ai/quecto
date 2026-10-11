//! How a push is judged: by its own ref status when it reports our ref
//! updated, and otherwise by whether the remote board holds our commit.
use super::*;

/// A `post-receive` hook on the remote that stacks another writer's commit
/// on the board once, right after our push lands, then runs `then`.
fn stack_after_next_push(repos: &Repos, then: &str) {
    let hook = repos.remote().join("hooks/post-receive");
    let script = format!(
        "#!/bin/sh\nrm -- \"$0\"\n\
         c=$(git -c user.name=Other -c user.email=o@x.io commit-tree \"{BOARD_REF}^{{tree}}\" -p {BOARD_REF} -m stacked)\n\
         git update-ref {BOARD_REF} \"$c\"\n{then}\n"
    );
    executable(&hook, &script);
}

fn stacked_on(repos: &Repos, ours: &BoardHead) {
    let remote = sh(&repos.remote(), &["log", "-2", "--format=%s %H", BOARD_REF]);
    let lines: Vec<&str> = remote.lines().collect();
    assert!(lines[0].starts_with("stacked "), "{remote}");
    assert_eq!(lines[1], format!("board: add a {}", ours.0));
}

#[test]
fn a_push_that_landed_is_written_though_another_writer_stacked_on_it_at_once() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    stack_after_next_push(&repos, "");
    let ours = written(alice.write(&head, &[task("a")], "board: add a"));
    stacked_on(&repos, &ours);
}

#[test]
fn a_push_that_landed_but_timed_out_is_written() {
    let repos = Repos::new();
    let mut git = Git::new(
        &repos.alice,
        GitEnvironment::allowlisted(vars(&repos.alice, &[])),
        person("Ada"),
    )
    .unwrap();
    git.network_timeout = std::time::Duration::from_secs(3);
    let alice = GitBoardStore::new(git, "origin").unwrap();
    let head = created(&alice);
    stack_after_next_push(&repos, "sleep 30");
    let started = std::time::Instant::now();
    let ours = written(alice.write(&head, &[task("a")], "board: add a"));
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    stacked_on(&repos, &ours);
}

#[test]
fn a_push_the_remote_declines_is_refused_and_the_board_stays() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    executable(
        &repos.remote().join("hooks/pre-receive"),
        "#!/bin/sh\nrm -- \"$0\"\necho no >&2\nexit 1\n",
    );
    let refused = alice.write(&head, &[task("a")], "board: add a");
    assert!(
        matches!(&refused, Ok(BoardWrite::Refused(why)) if why.contains("declined")),
        "{refused:?}"
    );
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
}

#[test]
fn a_write_from_a_stale_head_is_lost_and_refetches() {
    let repos = Repos::new();
    let (alice, bob) = (store(&repos.alice), store(&repos.bob));
    let stale = created(&alice);
    bob.read().unwrap();
    let won = written(alice.write(&stale, &[task("a")], "board: add a"));
    assert_eq!(
        bob.write(&stale, &[task("b")], "board: add b"),
        Ok(BoardWrite::Lost)
    );
    assert_eq!(
        sh(
            &repos.bob,
            &["rev-parse", "refs/remotes/origin/quecto/board"]
        ),
        won.0,
        "the loser refetched"
    );
}

#[test]
fn a_push_that_reports_nothing_and_did_not_land_is_unavailable() {
    let repos = Repos::new();
    let mut git = Git::new(
        &repos.alice,
        GitEnvironment::allowlisted(vars(&repos.alice, &[])),
        person("Ada"),
    )
    .unwrap();
    git.network_timeout = std::time::Duration::from_secs(3);
    let alice = GitBoardStore::new(git, "origin").unwrap();
    let head = created(&alice);
    executable(
        &repos.remote().join("hooks/pre-receive"),
        "#!/bin/sh\nrm -- \"$0\"\nsleep 30\n",
    );
    let write = alice.write(&head, &[task("a")], "board: add a");
    assert!(
        matches!(&write, Err(BoardStoreError::Unavailable(why)) if why.contains("may have moved")),
        "{write:?}"
    );
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
}

#[test]
fn reading_the_board_fetches_no_tags() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    sh(&repos.bob, &["tag", "remote-only"]);
    sh(&repos.bob, &["push", "-q", "origin", "remote-only"]);
    sh(&repos.alice, &["tag", "local-only"]);
    sh(&repos.alice, &["config", "remote.origin.tagOpt", "--tags"]);
    alice.read().unwrap();
    let tags = sh(
        &repos.alice,
        &["for-each-ref", "--format=%(refname)", "refs/tags"],
    );
    assert_eq!(tags, "refs/tags/local-only");
}

/// A commit on top of `base` that was never pushed.
fn unpushed(repos: &Repos, base: &BoardHead) -> String {
    sh(
        &repos.alice,
        &[
            "commit-tree",
            &format!("{}^{{tree}}", base.0),
            "-p",
            &base.0,
            "-m",
            "unpushed",
        ],
    )
}

fn rejected(summary: &str) -> PushReport {
    PushReport {
        rejected: Some(summary.into()),
        said: String::new(),
    }
}

fn silent(said: &str) -> PushReport {
    PushReport {
        rejected: None,
        said: said.into(),
    }
}

#[test]
fn any_push_outcome_but_updated_is_judged_by_the_board() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let base = created(&alice);
    let landed = written(alice.write(&base, &[task("a")], "board: add a"));
    // Our commit is on the board, whatever the push reported.
    assert_eq!(
        alice.judge(Some(&base.0), &landed.0, &rejected("[remote failure]")),
        Ok(Pushed::Landed)
    );
    assert_eq!(
        alice.judge(Some(&base.0), &landed.0, &silent("no report")),
        Ok(Pushed::Landed)
    );
    // The board is where we left it: only the remote's own rejection
    // refused us.
    let ours = unpushed(&repos, &landed);
    assert_eq!(
        alice.judge(
            Some(&landed.0),
            &ours,
            &rejected("[remote rejected] (pre-receive hook declined)")
        ),
        Ok(Pushed::Refused(
            "[remote rejected] (pre-receive hook declined)".into()
        ))
    );
    // A stale lease is git's own rejection: another writer moved the board
    // (and it may have moved back since), so the write is lost.
    assert_eq!(
        alice.judge(Some(&landed.0), &ours, &rejected("[rejected] (stale info)")),
        Ok(Pushed::Lost)
    );
    // It moved past where we started: lost.
    let stale = unpushed(&repos, &base);
    assert_eq!(
        alice.judge(Some(&base.0), &stale, &rejected("[rejected] (stale info)")),
        Ok(Pushed::Lost)
    );
    // The remote rejected it for a reason it gives when it declines a
    // push: refused. For a race on its ref: lost. For any other reason:
    // unknown.
    for (reason, judged) in [
        (
            "(protected branch hook declined)",
            Some(Pushed::Refused(String::new())),
        ),
        ("(hook declined)", Some(Pushed::Refused(String::new()))),
        (
            "(push declined due to repository rule violations)",
            Some(Pushed::Refused(String::new())),
        ),
        (
            "(cannot lock ref 'refs/heads/quecto/board': is at x but expected y)",
            Some(Pushed::Lost),
        ),
        ("(incorrect old value provided)", Some(Pushed::Lost)),
        (
            "(cannot lock ref 'refs/heads/quecto/board': Unable to create 'x.lock': File exists.)",
            None,
        ),
        ("(failed to update ref)", None),
        ("(shallow update not allowed)", None),
    ] {
        let summary = format!("[remote rejected] {reason}");
        let outcome = alice.judge(Some(&landed.0), &ours, &rejected(&summary));
        match (judged, &outcome) {
            (Some(Pushed::Refused(_)), Ok(Pushed::Refused(why))) => {
                assert!(why.contains(reason), "{why}")
            }
            (Some(Pushed::Lost), Ok(Pushed::Lost)) => {}
            (None, Err(BoardStoreError::Unavailable(why))) => {
                assert!(
                    why.contains("may have moved") && why.contains(reason),
                    "{why}"
                )
            }
            (judged, outcome) => panic!("{reason}: {outcome:?}, not {judged:?}"),
        }
    }
    // Nothing reported, or a failure the remote did not explain, and the
    // board did not move: it may still.
    for report in [
        silent("the push timed out"),
        rejected("[remote failure] (remote failed to report status)"),
    ] {
        let unknown = alice.judge(Some(&landed.0), &ours, &report);
        assert!(
            matches!(&unknown, Err(BoardStoreError::Unavailable(why)) if why.contains("may have moved")),
            "{report:?}: {unknown:?}"
        );
    }
}

/// A bootstrap whose push fails outright (no push rights, say) says the
/// push's own error, not what reading the absent board said.
#[test]
fn a_bootstrap_push_that_fails_says_why() {
    let repos = Repos::new();
    let nowhere = repos.dir.path().join("nowhere.git");
    sh(
        &repos.alice,
        &["config", "remote.origin.pushurl", nowhere.to_str().unwrap()],
    );
    let bootstrap = store(&repos.alice).bootstrap();
    assert!(
        matches!(&bootstrap, Err(BoardStoreError::Unavailable(why)) if why.contains("does not appear to be a git repository")),
        "{bootstrap:?}"
    );
}

#[test]
fn a_push_that_fails_outright_says_why() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let nowhere = repos.dir.path().join("nowhere.git");
    sh(
        &repos.alice,
        &["config", "remote.origin.pushurl", nowhere.to_str().unwrap()],
    );
    let write = alice.write(&head, &[task("a")], "board: add a");
    assert!(
        matches!(&write, Err(BoardStoreError::Unavailable(why)) if why.contains("does not appear to be a git repository") && !why.contains("Failed {") && !why.contains("stderr:")),
        "{write:?}"
    );
}

#[test]
fn ref_status_reads_only_our_ref_line() {
    let line =
        |flag: &str, summary: &str| format!("To r\n{flag}\tabc:{BOARD_REF}\t{summary}\nDone\n");
    assert_eq!(
        ref_status(line(" ", "a..b").as_bytes()),
        Some(RefStatus::Updated)
    );
    assert_eq!(
        ref_status(line("*", "[new branch]").as_bytes()),
        Some(RefStatus::Updated)
    );
    assert_eq!(
        ref_status(line("!", "[rejected] (stale info)").as_bytes()),
        Some(RefStatus::Rejected("[rejected] (stale info)".into()))
    );
    assert_eq!(
        ref_status(format!("!\tabc:{BOARD_REF}x\t[rejected]\n").as_bytes()),
        None
    );
    assert_eq!(ref_status(b"error: failed\n"), None);
    assert_eq!(ref_status(line("?", "odd").as_bytes()), None);
    // The remote's reason is quoted, never passed through raw.
    assert_eq!(
        ref_status(line("!", "[remote rejected] (\u{1b}[31mno\u{202e}pe\u{200b})").as_bytes()),
        Some(RefStatus::Rejected("[remote rejected] (no pe )".into()))
    );
}
