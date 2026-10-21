//! #2687: a worker may not commit, so it submits its evidence at the commit
//! its uncommitted change sits on; the coordinator commits that change and
//! verifies at its own commit. The board carries the submission's evidence
//! forward to that commit when git confirms the submitted commit is in its
//! history, so the worker is not sent through release and reclaim to
//! resubmit the same evidence (the extra round every live run paid). Run on
//! the composed board over a real git checkout.
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use crate::swarm_board_diff_membership::HOUR;
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;

/// git in `dir` with no user or system configuration, no inherited git
/// environment and a fixed committer: its trimmed standard output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        // Only PATH passes: a git hook's GIT_DIR or GIT_INDEX_FILE (the
        // pre-push gate runs these tests) must never reach the real
        // repository.
        .env_clear()
        .envs(std::env::var_os("PATH").map(|path| ("PATH", path)))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git prints text")
        .trim_end()
        .to_owned()
}

/// A checkout with a base commit, the coordinator's commit of the worker's
/// change on top of it, and a commit outside that history (no parent).
struct Checkout {
    dir: tempfile::TempDir,
    base: String,
    committed: String,
    unrelated: String,
}

fn checkout() -> Checkout {
    let dir = tempfile::tempdir().expect("a checkout directory");
    let root = dir.path();
    git(root, &["init", "-q"]);
    std::fs::write(
        root.join("lib.py"),
        "def count_words(text):\n    return 0\n",
    )
    .unwrap();
    git(root, &["add", "lib.py"]);
    git(root, &["commit", "-q", "-m", "base"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    // The worker's change, uncommitted when it submits; the coordinator
    // commits it.
    std::fs::write(root.join("lib.py"), "def char_count(text):\n    return 0\n").unwrap();
    git(root, &["commit", "-q", "-am", "Add char_count"]);
    let committed = git(root, &["rev-parse", "HEAD"]);
    let tree = git(root, &["rev-parse", "HEAD^{tree}"]);
    let unrelated = git(root, &["commit-tree", &tree, "-m", "unrelated"]);
    Checkout {
        dir,
        base,
        committed,
        unrelated,
    }
}

fn ok(outcome: Outcome) -> Value {
    match outcome {
        Outcome::Ok(value) => value,
        other => panic!("refused: {other:?}"),
    }
}

/// `parent` coordinates a run with a command and a review criterion;
/// `worker` claims task 1 and submits its evidence at `revision`. Answers
/// the board and the claim token.
fn submitted(database: &Path, checkout: &Path, revision: &str) -> (RustBoard, Value) {
    let board = RustBoard::open(database, checkout);
    ok(board.call(
        "parent",
        "create_run",
        &json!([
            "ship char_count",
            [],
            [
                {"id": "tests", "kind": "command", "description": "tests pass"},
                {"id": "review", "kind": "review", "description": "reviewed"}
            ],
            3,
            NOW + HOUR
        ]),
        NOW,
    ));
    ok(board.call(
        "parent",
        "_admit",
        &json!(["worker", "reservation-w"]),
        NOW + 1.0,
    ));
    ok(board.call(
        "parent",
        "_activate",
        &json!(["worker", "reservation-w", 12_345, "start-w", "/tmp/w.sock"]),
        NOW + 2.0,
    ));
    ok(board.call(
        "worker",
        "task_create",
        &json!([
            "implement",
            "Implement char_count",
            ["char_count works"],
            []
        ]),
        NOW + 3.0,
    ));
    let claimed = ok(board.call("worker", "claim", &json!([1]), NOW + 4.0));
    let token = claimed["token"].clone();
    assert!(token.is_string(), "{claimed}");
    ok(board.call(
        "worker",
        "submit",
        &json!([1, token, [{"artifact": "item-1-evidence.txt", "revision": revision}]]),
        NOW + 5.0,
    ));
    (board, token)
}

/// The live runs' flow, now one round: the worker submits at the base, the
/// coordinator commits the change and verifies at its own commit. The task
/// completes with its evidence carried to that commit, and the run
/// completes there, with no resubmission.
#[test]
fn the_coordinator_verifies_at_its_own_commit_of_the_submitted_change() {
    let checkout = checkout();
    let store = tempfile::tempdir().expect("a board directory");
    let database = store.path().join("swarm.sqlite");
    let (board, token) = submitted(&database, checkout.dir.path(), &checkout.base);
    let verified = ok(board.call(
        "parent",
        "verify_task",
        &json!([1, token, checkout.committed]),
        NOW + 6.0,
    ));
    assert_eq!(verified["status"], json!("completed"), "{verified}");
    assert_eq!(
        verified["evidence"],
        json!([{"artifact": "item-1-evidence.txt", "revision": checkout.committed}]),
        "{verified}"
    );
    for (criterion, kind) in [("tests", "command"), ("review", "review")] {
        ok(board.call(
            "parent",
            "evidence",
            &json!([criterion, "checks.txt", checkout.committed, kind, true]),
            NOW + 7.0,
        ));
    }
    let completed = ok(board.call(
        "parent",
        "complete",
        &json!([checkout.committed]),
        NOW + 8.0,
    ));
    assert_eq!(completed["outcome"], json!("succeeded"), "{completed}");
}

/// Evidence is carried only along the commit's own history: a commit that
/// does not hold the submitted one is refused as before, and the
/// submission stays as the worker made it.
#[test]
fn evidence_is_not_carried_to_a_commit_outside_its_history() {
    let checkout = checkout();
    let store = tempfile::tempdir().expect("a board directory");
    let database = store.path().join("swarm.sqlite");
    let (board, token) = submitted(&database, checkout.dir.path(), &checkout.committed);
    for revision in [&checkout.unrelated, &checkout.base] {
        assert_eq!(
            board.call(
                "parent",
                "verify_task",
                &json!([1, token, revision]),
                NOW + 6.0
            ),
            Outcome::Refused("stale evidence revision".to_owned()),
            "{revision}"
        );
    }
    let task = ok(board.call("parent", "task", &json!([1]), NOW + 7.0));
    assert_eq!(task["status"], json!("submitted"), "{task}");
    assert_eq!(
        task["evidence"],
        json!([{"artifact": "item-1-evidence.txt", "revision": checkout.committed}]),
        "{task}"
    );
}
