use std::sync::Arc;

use serde_json::{Value, json};

use super::RecordEvidence;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board,
};
use crate::application::swarm::dto::{EvidenceTransition, RecordEvidenceRequest};
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

fn board_state() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.run.as_mut().unwrap().contract.criteria = json!([
        {"id": "tests", "kind": "command", "description": "pass"},
        {"id": "review", "kind": "review", "description": "reviewed"}
    ]);
    state
}

fn evidence(actor: &str, criterion: Value, kind: &str, passed: Value) -> RecordEvidenceRequest {
    RecordEvidenceRequest {
        actor: actor.to_owned(),
        criterion,
        artifact: json!("ci.log"),
        revision: json!("R1"),
        kind: json!(kind),
        passed,
    }
}

fn service(board: &Arc<MemoryBoard>) -> RecordEvidence {
    RecordEvidence::new(board.clone(), SteppingClock::fixed(50.0))
}

/// The artifact (2048 bytes) and the revision (256) are bounded before
/// the store is opened.
#[test]
fn the_artifact_and_revision_are_bounded_before_the_store() {
    let board = MemoryBoard::with(board_state());
    let mut long_artifact = evidence("parent", json!("tests"), "command", json!(true));
    long_artifact.artifact = json!("x".repeat(2_049));
    let mut long_revision = evidence("parent", json!("tests"), "command", json!(true));
    long_revision.revision = json!("x".repeat(257));
    long_revision.artifact = json!(5);
    assert_eq!(
        service(&board).execute(long_revision).unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "artifact reference must be nonempty and at most 2048 bytes"
        )
    );
    long_artifact.artifact = json!("a");
    long_artifact.revision = json!("x".repeat(257));
    assert_eq!(
        service(&board).execute(long_artifact).unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "artifact revision must be nonempty and at most 256 bytes"
        )
    );
    assert!(board.transactions().is_empty(), "refused before the store");
}

/// The criterion must be configured with that kind; a paused run takes no
/// evidence.
#[test]
fn evidence_must_match_a_configured_criterion_and_kind() {
    let board = MemoryBoard::with(board_state());
    for (criterion, kind) in [
        (json!("missing"), "command"),
        (json!("tests"), "review"),
        (json!(["tests"]), "command"),
    ] {
        assert_eq!(
            service(&board)
                .execute(evidence("parent", criterion, kind, json!(true)))
                .unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "evidence must match a configured criterion and kind"
            )
        );
    }
    let mut paused = board_state();
    paused.run.as_mut().unwrap().record.status = Some(RunState::PAUSED);
    let board = MemoryBoard::with(paused);
    assert_eq!(
        service(&board)
            .execute(evidence("parent", json!("tests"), "command", json!(true)))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::NotRunning,
            "run is paused; no new work permitted"
        )
    );
}

/// Only the coordinator's `passed is True` accepts: a worker's record is
/// a proposal, and a truthy `passed` that is not `true` is not a pass.
/// The same record again is a no-op; a changed one replaces it.
#[test]
fn only_the_coordinators_pass_is_accepted_and_a_repeat_is_a_no_op() {
    let board = MemoryBoard::with(board_state());
    let recorded = [
        (
            "worker",
            json!(true),
            EvidenceTransition::Recorded,
            json!(0),
        ),
        (
            "worker",
            json!(true),
            EvidenceTransition::Unchanged,
            json!(0),
        ),
        ("parent", json!(1), EvidenceTransition::Recorded, json!(0)),
        (
            "parent",
            json!(true),
            EvidenceTransition::Recorded,
            json!(1),
        ),
        (
            "parent",
            json!(true),
            EvidenceTransition::Unchanged,
            json!(1),
        ),
    ];
    for (actor, passed, transition, stored) in recorded {
        assert_eq!(
            service(&board)
                .execute(evidence(actor, json!("tests"), "command", passed))
                .unwrap(),
            transition
        );
        let state = board.snapshot();
        let row = state.evidence.iter().find(|(by, _)| by == actor).unwrap();
        assert_eq!(row.1.accepted, stored);
    }
    let state = board.snapshot();
    assert_eq!(state.evidence.len(), 2, "one row per criterion and actor");
    let details: Vec<(String, Value)> = state
        .events
        .into_iter()
        .map(|event| (event.actor, event.detail))
        .collect();
    let detail = |accepted: bool| json!({"criterion": "tests", "artifact": "ci.log", "revision": "R1", "accepted": accepted});
    assert_eq!(
        details,
        [
            ("worker".to_owned(), detail(false)),
            ("parent".to_owned(), detail(false)),
            ("parent".to_owned(), detail(true)),
        ]
    );
}
