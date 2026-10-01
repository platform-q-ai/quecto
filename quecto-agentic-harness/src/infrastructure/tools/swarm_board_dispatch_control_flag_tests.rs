//! #2390 review M2: the dispatcher says, typed and next to each decision,
//! whether an op changed the run's control state (its status, control
//! generation or deadline), so the run watch is nudged exactly then. Every
//! control-changing decision is driven here on a real board file: a
//! renamed or new decision cannot fall back to the 600 s refresh silently.
use serde_json::{Value, json};

use super::tests::{board, running};
use super::{CallOrigin, SwarmBoardHandles, call, call_deciding, may_control_run};

/// Whether `member`'s call of `op` changed the run's control state; the
/// call must be answered (or refused after its writes committed).
fn controls(handles: &SwarmBoardHandles, member: &str, op: &str, args: Value) -> bool {
    let (answer, controls_run) = call_deciding(handles, member, op, args, CallOrigin::Member);
    if let Err(refusal) = &answer {
        assert!(!controls_run, "{op} refused: {}", refusal.message());
    }
    if controls_run {
        assert!(
            may_control_run(op),
            "{op} changes control but is not listed"
        );
    }
    controls_run
}

fn criteria() -> Value {
    json!([{"id": "t", "kind": "command", "description": "test"}])
}

fn created(handles: &SwarmBoardHandles) {
    running(handles);
}

fn with_worker(handles: &SwarmBoardHandles) {
    created(handles);
    call(handles, "parent", "_admit", json!(["worker", "r1"])).unwrap();
    call(
        handles,
        "parent",
        "_activate",
        json!(["worker", "r1", 7, "s", "/w.sock"]),
    )
    .unwrap();
}

#[test]
fn creating_a_run_changes_its_control_state() {
    let (_dir, handles) = board(1_000.0);
    assert!(controls(
        &handles,
        "parent",
        "create",
        json!(["ship", [], criteria(), 3, 4_600.0])
    ));
    let (_dir, handles) = board(1_000.0);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    assert!(controls(
        &handles,
        "parent",
        "create",
        json!(["ship", [], criteria(), 3, 4_600.0])
    ));
}

#[test]
fn supervisor_and_coordinator_control_ops_change_it_only_when_applied() {
    let (_dir, handles) = board(1_000.0);
    created(&handles);
    assert!(!controls(&handles, "parent", "summary", json!([])));
    assert!(!controls(&handles, "parent", "_watch", json!([null])));
    assert!(controls(&handles, "parent", "pause", json!(["why"])));
    assert!(
        !controls(&handles, "parent", "pause", json!(["why"])),
        "unchanged"
    );
    assert!(controls(
        &handles,
        "parent",
        "_extend_deadline",
        json!([60])
    ));
    assert!(controls(&handles, "parent", "_resume_external", json!([])));
    assert!(!controls(&handles, "parent", "_resume_external", json!([])));
    assert!(controls(
        &handles,
        "parent",
        "stop",
        json!(["blocked", "why"])
    ));
    assert!(controls(&handles, "parent", "_close", json!([])));
    assert!(
        !controls(&handles, "parent", "_close", json!([])),
        "unchanged"
    );
}

#[test]
fn a_budget_pause_changes_it_and_a_budget_that_holds_does_not() {
    let (_dir, handles) = board(1_000.0);
    created(&handles);
    let failed = json!({"request_id": "r1", "instrumented_attempts": 1, "outcome": "failed"});
    assert!(!controls(
        &handles,
        "parent",
        "_record_request",
        json!([failed])
    ));
    assert!(!controls(
        &handles,
        "parent",
        "_request_admission",
        json!([])
    ));
    assert!(
        controls(&handles, "parent", "usage_budget", json!([100])),
        "paused"
    );
    let (_dir, handles) = board(1_000.0);
    created(&handles);
    assert!(!controls(&handles, "parent", "usage_budget", json!([100])));
    let failed = json!({"request_id": "r2", "instrumented_attempts": 1, "outcome": "failed"});
    assert!(
        controls(&handles, "parent", "_record_request", json!([failed])),
        "paused"
    );
}

#[test]
fn completing_a_run_changes_it() {
    let (_dir, handles) = board(1_000.0);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    with_worker(&handles);
    call(
        &handles,
        "parent",
        "task_create",
        json!(["r1", "t", ["tests pass"]]),
    )
    .unwrap();
    let token = call(&handles, "worker", "claim", json!([1])).unwrap()["token"].clone();
    let evidence = json!([{"artifact": "report", "revision": "R1"}]);
    call(&handles, "worker", "submit", json!([1, token, evidence])).unwrap();
    call(&handles, "parent", "verify_task", json!([1, token, "R1"])).unwrap();
    call(
        &handles,
        "parent",
        "evidence",
        json!(["t", "report", "R1", "command", true]),
    )
    .unwrap();
    assert!(controls(&handles, "parent", "complete", json!(["R1"])));
}

/// A recorded loss changes it exactly when it ends the run by loss: a
/// worker's past its grace, or the coordinator's; not one inside its grace,
/// nor a later one on a run the first loss already ended.
#[test]
fn a_loss_changes_it_exactly_when_it_ends_the_run() {
    let (dir, handles) = board(1_000.0);
    with_worker(&handles);
    assert!(
        !controls(&handles, "parent", "_quarantine", json!(["worker"])),
        "grace"
    );
    backdate_observations(&dir.path().join("swarm.sqlite"));
    assert!(controls(
        &handles,
        "parent",
        "_quarantine",
        json!(["worker"])
    ));
    assert!(
        !controls(&handles, "parent", "_quarantine", json!(["worker"])),
        "already lost"
    );
    assert!(
        !controls(&handles, "parent", "_confirmed_dead", json!(["worker"])),
        "the run already ended"
    );
    let (_dir, handles) = board(1_000.0);
    with_worker(&handles);
    assert!(controls(
        &handles,
        "worker",
        "_quarantine",
        json!(["parent"])
    ));
    let (_dir, handles) = board(1_000.0);
    with_worker(&handles);
    assert!(controls(
        &handles,
        "worker",
        "_confirmed_dead",
        json!(["parent"])
    ));
    assert!(
        !controls(&handles, "worker", "_lose_coordinator", json!([])),
        "not lost"
    );
    let (_dir, handles) = board(1_000.0);
    created(&handles);
    assert!(controls(&handles, "parent", "_lose_coordinator", json!([])));
}

/// Every loss observation on the board file moved a minute into the past,
/// as the clock would after the grace.
fn backdate_observations(database: &std::path::Path) {
    rusqlite::Connection::open(database)
        .unwrap()
        .execute(
            "UPDATE events SET time = time - 60 WHERE action = 'scope_observed'",
            [],
        )
        .unwrap();
}
