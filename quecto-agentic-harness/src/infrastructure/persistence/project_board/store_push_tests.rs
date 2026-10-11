//! How a push is judged: by its own ref status, or, when it reports none,
//! by whether the remote board holds our commit.
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
        matches!(write, Err(BoardStoreError::Unavailable(_))),
        "{write:?}"
    );
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
}

#[test]
fn reading_the_board_fetches_no_tags_and_prunes_nothing() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    sh(&repos.bob, &["tag", "remote-only"]);
    sh(&repos.bob, &["push", "-q", "origin", "remote-only"]);
    sh(&repos.alice, &["tag", "local-only"]);
    sh(
        &repos.alice,
        &["update-ref", "refs/remotes/origin/gone", "HEAD"],
    );
    for (key, value) in [
        ("fetch.prune", "true"),
        ("fetch.pruneTags", "true"),
        ("remote.origin.prune", "true"),
        ("remote.origin.tagOpt", "--tags"),
    ] {
        sh(&repos.alice, &["config", key, value]);
    }
    let refs = || {
        sh(
            &repos.alice,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/tags",
                "refs/remotes/origin/gone",
            ],
        )
    };
    let before = refs();
    alice.read().unwrap();
    assert_eq!(refs(), before);
    assert_eq!(before, "refs/remotes/origin/gone\nrefs/tags/local-only");
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
}
