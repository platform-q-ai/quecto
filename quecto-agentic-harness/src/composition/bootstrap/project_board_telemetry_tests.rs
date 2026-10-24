//! #2663: over the real git store, a write's `board_op` names its board
//! by an opaque id and the commit it made.
use std::sync::Arc;

use serde_json::{Value, json};

use super::tests::{environment, repository};
use super::*;
use crate::application::project_board::operations::ListSummaries;
use crate::domain::audit::AuditEvent;
use crate::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
use crate::interface::tools::project_board::recording::BoardCall;
use crate::interface::tools::project_board::recording::tests::{Records, recording};
use crate::interface::tools::project_board::run_recorded_with;

/// `request` on `handles`, recorded as a front end records it: its line.
fn recorded(handles: &ProjectBoardHandles, request: Value) -> Value {
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Cli, Client::Cli);
    call.on_board(handles.board.as_ref());
    let answer = run_recorded_with(
        &*handles.operations,
        &request.to_string(),
        ListSummaries::Omitted,
        &mut call,
    );
    call.finish(&answer, false);
    let mut taken = records.taken();
    assert_eq!(taken.len(), 1, "{answer}");
    serde_json::to_value(AuditEvent::BoardOp(Box::new(taken.remove(0)))).unwrap()
}

#[test]
fn a_write_names_its_board_and_the_commit_it_made() {
    let (dir, work) = repository();
    let handles = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    recorded(&handles, json!({"op": "bootstrap"}));
    let created = recorded(
        &handles,
        json!({"op": "create", "task": "a", "title": "A", "status": "ready"}),
    );
    let remote = dir.path().join("remote.git");
    let head = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/quecto/board"])
        .current_dir(&remote)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();
    assert_eq!(created["head"], json!(head), "{created}");
    let board = created["board"].as_str().unwrap_or_default().to_string();
    assert!(
        board.len() == 16 && board.bytes().all(|b| b.is_ascii_hexdigit()),
        "{created}"
    );
    assert!(!board.contains("remote"), "the id names no remote");
    // One remote, one id: a read names the same board.
    let listed = recorded(&handles, json!({"op": "list"}));
    assert_eq!(listed["board"], json!(board), "{listed}");
}

/// #2663 verify round: a relative remote is the clone's top level's, as
/// git resolves it, so the CLI run from a subdirectory names the same
/// board as from the clone itself.
#[test]
fn a_relative_remote_names_one_board_from_any_directory_of_the_clone() {
    let (dir, work) = repository();
    let git = |args: &[&str]| {
        let done = std::process::Command::new("git")
            .args(args)
            .current_dir(&work)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(done.success(), "{args:?}");
    };
    git(&["remote", "set-url", "origin", "../remote.git"]);
    let sub = work.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let board_of = |repo: &std::path::Path| {
        build_project_board_handles(&environment(repo, Access::Read))
            .unwrap()
            .board
            .unwrap()
    };
    let relative = board_of(&work);
    assert_eq!(relative, board_of(&sub));
    // From the work tree, not its git directory: `../remote.git` is the
    // remote the absolute URL names.
    let absolute = dir.path().join("remote.git");
    git(&["remote", "set-url", "origin", absolute.to_str().unwrap()]);
    assert_eq!(relative, board_of(&work), "the work tree's ../remote.git");
}
