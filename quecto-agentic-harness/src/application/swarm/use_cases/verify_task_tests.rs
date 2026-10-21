use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::VerifyTask;
use crate::application::swarm::dto::{TaskTransition, VerifyTaskRequest};
use crate::application::swarm::ports::board_files::{Ancestry, CheckoutHistory};
use crate::application::swarm::test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredFile, member_row, running_board, stored_task,
};
use crate::domain::swarm::{
    events::telemetry::RefusalKind, value_objects::board_error::BoardError,
};

/// What a test does while the history is asked: another member's op.
type During = Box<dyn Fn() + Send + Sync>;

/// A checkout's history: the (ancestor, descendant) pairs it confirms,
/// every question it was asked, and what happens on the board meanwhile.
struct History {
    confirmed: Vec<(&'static str, &'static str)>,
    asked: Mutex<Vec<(Vec<String>, String)>>,
    during: Option<During>,
}

impl History {
    fn of(confirmed: &[(&'static str, &'static str)]) -> Arc<Self> {
        Arc::new(Self {
            confirmed: confirmed.to_vec(),
            asked: Mutex::new(Vec::new()),
            during: None,
        })
    }

    fn none() -> Arc<Self> {
        Self::of(&[])
    }

    fn while_asked(confirmed: &[(&'static str, &'static str)], during: During) -> Arc<Self> {
        Arc::new(Self {
            confirmed: confirmed.to_vec(),
            asked: Mutex::new(Vec::new()),
            during: Some(during),
        })
    }

    fn asked(&self) -> Vec<(Vec<String>, String)> {
        self.asked.lock().unwrap().clone()
    }
}

impl CheckoutHistory for History {
    fn ancestry(&self, ancestors: &[String], descendant: &str) -> Ancestry {
        self.asked
            .lock()
            .unwrap()
            .push((ancestors.to_vec(), descendant.to_owned()));
        if let Some(during) = &self.during {
            during();
        }
        match ancestors
            .iter()
            .all(|ancestor| self.confirmed.contains(&(ancestor.as_str(), descendant)))
        {
            true => Ancestry::Confirmed,
            false => Ancestry::NotAncestor,
        }
    }
}

fn file(path: &str, task: i64, claim: &str) -> StoredFile {
    StoredFile {
        path: path.to_owned(),
        task,
        owner: "worker".to_owned(),
        claim: claim.to_owned(),
        token: "reservation".to_owned(),
    }
}

/// Task 1 submitted by the worker under `stored-token` at revision `R1`,
/// task 2 claimed by the worker; files reserved under task 1's claim, an
/// older claim of task 1, and task 2's claim.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let mut submitted = stored_task(1, "submitted", json!([]), Some("worker"));
    submitted.set(
        "evidence",
        json!([{"artifact": "a", "revision": "R1"}, {"artifact": "b", "revision": "R1"}]),
    );
    state.tasks = vec![
        submitted,
        stored_task(2, "claimed", json!([]), Some("worker")),
    ];
    state.files = vec![
        file("a", 1, "stored-token"),
        file("b", 1, "older"),
        file("c", 2, "stored-token"),
    ];
    state
}

fn verify(actor: &str, task_id: Value, token: &str, revision: Value) -> VerifyTaskRequest {
    VerifyTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token: Value::from(token),
        revision,
    }
}

/// `test_submission_is_not_completion_and_requires_current_token`: only
/// the coordinator verifies; the submission's token and every entry's
/// revision must match. Verification completes the task, deletes the
/// files reserved under that claim only and records
/// `verified{task,revision}`; verifying completed work again is a no-op,
/// but still checks the revision.
#[test]
fn the_coordinator_verifies_the_current_submission_once() {
    let board = MemoryBoard::with(board());
    let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0), History::none());
    for (request, refusal) in [
        (
            verify("worker", json!(1), "stored-token", json!("R1")),
            (
                RefusalKind::NotCoordinator,
                "only the designated coordinator may do this",
            ),
        ),
        (
            verify("parent", json!(1), "stale", json!("R1")),
            (RefusalKind::StaleToken, "stale claim or work not submitted"),
        ),
        (
            verify("parent", json!(2), "stored-token", json!("R1")),
            (RefusalKind::StaleToken, "stale claim or work not submitted"),
        ),
        (
            verify("parent", json!(1), "stored-token", json!("R2")),
            (RefusalKind::StaleRevision, "stale evidence revision"),
        ),
        (
            verify("parent", json!(9), "stored-token", json!("R1")),
            (RefusalKind::NotFound, "unknown task"),
        ),
    ] {
        assert_eq!(
            service.execute(request).unwrap_err(),
            BoardError::new(refusal.0, refusal.1)
        );
    }
    assert_eq!(
        service
            .execute(verify("parent", json!("1"), "stored-token", json!("R1")))
            .unwrap()
            .transition,
        TaskTransition::Applied
    );
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!("R1")))
            .unwrap()
            .transition,
        TaskTransition::Unchanged
    );
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!("R2")))
            .unwrap_err(),
        BoardError::new(RefusalKind::StaleRevision, "stale evidence revision")
    );
    let state = board.snapshot();
    assert_eq!(state.tasks[0].text("status"), Some("completed"));
    assert_eq!(state.tasks[0].text("owner"), Some("worker"));
    let paths: Vec<&str> = state.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["b", "c"]);
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        ("verified", &json!({"task": "1", "revision": "R1"}))
    );
}

/// Revisions compare by Python's `==`: a stored integer revision matches
/// its float, and text never matches a number.
#[test]
fn revisions_compare_as_python_compares_them() {
    let mut state = board();
    state.tasks[0].set("evidence", json!([{"artifact": "a", "revision": 7}]));
    let board = MemoryBoard::with(state);
    let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0), History::none());
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!("7")))
            .unwrap_err(),
        BoardError::new(RefusalKind::StaleRevision, "stale evidence revision")
    );
    service
        .execute(verify("parent", json!(1), "stored-token", json!(7.0)))
        .unwrap();
    assert_eq!(
        board.snapshot().events[0].detail,
        json!({"task": 1, "revision": 7.0})
    );
}

/// `outside_edited_evidence`: stored evidence only a file edited outside
/// the board can hold (not a list, or an entry without a `revision`) is
/// refused by name, where Python raises or iterates the value.
#[test]
fn stored_evidence_without_revisions_is_refused() {
    for evidence in [
        json!({"revision": "R1"}),
        json!("R1"),
        json!(null),
        json!([{"artifact": "a"}]),
        json!(["R1"]),
    ] {
        let mut state = board();
        state.tasks[0].set("evidence", evidence.clone());
        let board = MemoryBoard::with(state);
        let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0), History::none());
        assert_eq!(
            service
                .execute(verify("parent", json!(1), "stored-token", json!("R1")))
                .unwrap_err(),
            BoardError::new(
                RefusalKind::Store,
                "stored evidence is not a list of revisioned entries"
            ),
            "{evidence}"
        );
        assert_eq!(board.snapshot().tasks[0].text("status"), Some("submitted"));
    }
}

/// Entries are read in order until the first stale one, as Python's `any`
/// reads them: a stale revision before an entry without one is refused as
/// stale.
#[test]
fn entries_are_read_until_the_first_stale_revision() {
    for (evidence, refusal) in [
        (
            json!([{"revision": "R2"}, "junk"]),
            (RefusalKind::StaleRevision, "stale evidence revision"),
        ),
        (
            json!([{"revision": "R1"}, "junk"]),
            (
                RefusalKind::Store,
                "stored evidence is not a list of revisioned entries",
            ),
        ),
    ] {
        let mut state = board();
        state.tasks[0].set("evidence", evidence.clone());
        let board = MemoryBoard::with(state);
        let service = VerifyTask::new(board, SteppingClock::fixed(50.0), History::none());
        assert_eq!(
            service
                .execute(verify("parent", json!(1), "stored-token", json!("R1")))
                .unwrap_err(),
            BoardError::new(refusal.0, refusal.1),
            "{evidence}"
        );
    }
}

const BASE: &str = "db03de82dd7a7222b51a301bb9e8f0a01bada0f3";
const COMMIT: &str = "eb4fe3fd96771037e7d9162013d22036fa42d82a";
const LATER: &str = "42b27d46167fc11e281dd005a97859b2ada5f48d";

/// [`board`] with task 1's evidence submitted at `revisions`, one entry
/// each.
fn submitted_at(revisions: &[&str]) -> BoardState {
    let mut state = board();
    let evidence: Vec<Value> = revisions
        .iter()
        .enumerate()
        .map(|(index, revision)| json!({"artifact": format!("e{index}"), "revision": revision}))
        .collect();
    state.tasks[0].set("evidence", Value::from(evidence));
    state
}

/// #2687: the worker submits at the commit its change sits on; the
/// coordinator commits the change and verifies at its own commit. The
/// history confirms the submitted commit is an ancestor, so the evidence
/// is carried to the commit, the task completes, the claim's files are
/// released, and `verified` records where the evidence came from and the
/// submission as made.
#[test]
fn submitted_evidence_is_carried_to_a_descendant_commit() {
    let board = MemoryBoard::with(submitted_at(&[BASE, COMMIT, BASE]));
    let history = History::of(&[(BASE, COMMIT)]);
    let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0), history.clone());
    let change = service
        .execute(verify("parent", json!(1), "stored-token", json!(COMMIT)))
        .unwrap();
    assert_eq!(change.transition, TaskTransition::Applied);
    let carried = json!([
        {"artifact": "e0", "revision": COMMIT},
        {"artifact": "e1", "revision": COMMIT},
        {"artifact": "e2", "revision": COMMIT},
    ]);
    assert_eq!(change.task.get("evidence"), Some(&carried));
    assert_eq!(
        history.asked(),
        [(vec![BASE.to_owned()], COMMIT.to_owned())]
    );
    let state = board.snapshot();
    assert_eq!(state.tasks[0].text("status"), Some("completed"));
    assert_eq!(state.tasks[0].get("evidence"), Some(&carried));
    let paths: Vec<&str> = state.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["b", "c"]);
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        (
            "verified",
            &json!({
                "task": 1,
                "revision": COMMIT,
                "carried_from": [BASE],
                "previous_evidence": [
                    {"artifact": "e0", "revision": BASE},
                    {"artifact": "e1", "revision": COMMIT},
                    {"artifact": "e2", "revision": BASE},
                ],
            })
        )
    );
    // Verifying again finds completed work at the commit: no change.
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!(COMMIT)))
            .unwrap()
            .transition,
        TaskTransition::Unchanged
    );
}

/// Evidence is carried only when every earlier commit is confirmed an
/// ancestor, by the coordinator, of submitted work, with a current claim:
/// anything else is refused as before and leaves the submission as made.
#[test]
fn evidence_is_carried_only_when_the_history_confirms_it() {
    let stale = || BoardError::new(RefusalKind::StaleRevision, "stale evidence revision");
    for (revisions, confirmed, request, refusal) in [
        // The history does not confirm the submitted commit.
        (
            vec![BASE],
            vec![],
            verify("parent", json!(1), "stored-token", json!(COMMIT)),
            stale(),
        ),
        // Only one of two earlier commits is confirmed.
        (
            vec![BASE, LATER],
            vec![(BASE, COMMIT)],
            verify("parent", json!(1), "stored-token", json!(COMMIT)),
            stale(),
        ),
        // Revisions that are not full commit ids are never carried.
        (
            vec!["R1"],
            vec![],
            verify("parent", json!(1), "stored-token", json!(COMMIT)),
            stale(),
        ),
        (
            vec![BASE],
            vec![(BASE, COMMIT)],
            verify("parent", json!(1), "stored-token", json!("HEAD")),
            stale(),
        ),
        // A worker never verifies, so never carries.
        (
            vec![BASE],
            vec![(BASE, COMMIT)],
            verify("worker", json!(1), "stored-token", json!(COMMIT)),
            BoardError::new(
                RefusalKind::NotCoordinator,
                "only the designated coordinator may do this",
            ),
        ),
        // A stale claim is refused before any carry.
        (
            vec![BASE],
            vec![(BASE, COMMIT)],
            verify("parent", json!(1), "older", json!(COMMIT)),
            BoardError::new(RefusalKind::StaleToken, "stale claim or work not submitted"),
        ),
    ] {
        let state = submitted_at(&revisions);
        let before = state.tasks[0].clone();
        let board = MemoryBoard::with(state);
        let service = VerifyTask::new(
            board.clone(),
            SteppingClock::fixed(50.0),
            History::of(&confirmed),
        );
        assert_eq!(
            service.execute(request).unwrap_err(),
            refusal,
            "{revisions:?}"
        );
        let state = board.snapshot();
        assert_eq!(state.tasks[0], before, "{revisions:?}");
        assert!(state.events.is_empty(), "{revisions:?}");
    }
}

/// Completed work is never carried by verification (revalidation is the
/// coordinator's op for it): a stale revision is refused as before.
#[test]
fn completed_work_is_not_carried() {
    let mut state = submitted_at(&[BASE]);
    state.tasks[0].set("status", json!("completed"));
    let board = MemoryBoard::with(state);
    let history = History::of(&[(BASE, COMMIT)]);
    let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0), history.clone());
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!(COMMIT)))
            .unwrap_err(),
        BoardError::new(RefusalKind::StaleRevision, "stale evidence revision")
    );
    assert!(history.asked().is_empty());
    assert_eq!(
        board.snapshot().tasks[0].get("evidence"),
        Some(&json!([{"artifact": "e0", "revision": BASE}]))
    );
}

/// Another member's change to the board.
type Change = fn(&mut BoardState);

/// #2691 review M1: git is asked with no transaction open, and the carry
/// is written only over the submission the history was asked about. A
/// worker that releases, or resubmits, while git runs leaves its change in
/// place (were git asked inside a transaction, its commit would overwrite
/// the change), and the verification is refused as stale.
#[test]
fn a_submission_changed_while_git_runs_is_not_carried() {
    let changes: [(&str, Change); 3] = [
        ("released", |state| {
            state.tasks[0].set("status", json!("ready"));
        }),
        ("reclaimed", |state| {
            state.tasks[0].set("token", json!("newer"));
        }),
        ("resubmitted", |state| {
            state.tasks[0].set("evidence", json!([{"artifact": "other", "revision": BASE}]));
        }),
    ];
    for (name, change) in changes {
        let board = MemoryBoard::with(submitted_at(&[BASE]));
        let during = board.clone();
        let history = History::while_asked(
            &[(BASE, COMMIT)],
            Box::new(move || change(&mut during.state.lock().unwrap())),
        );
        let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0), history.clone());
        let refusal = service
            .execute(verify("parent", json!(1), "stored-token", json!(COMMIT)))
            .unwrap_err();
        assert!(
            matches!(
                refusal.kind(),
                RefusalKind::StaleRevision | RefusalKind::StaleToken
            ),
            "{name}: {refusal:?}"
        );
        assert_eq!(history.asked().len(), 1, "{name}");
        let mut expected = submitted_at(&[BASE]);
        change(&mut expected);
        let state = board.snapshot();
        assert_eq!(state.tasks[0], expected.tasks[0], "{name}");
        assert!(state.events.is_empty(), "{name}");
    }
}

/// A carry runs two operations, each its gate and its work, and asks git
/// between them: four transactions, the history asked after the second.
#[test]
fn git_is_asked_between_the_two_operations() {
    let board = MemoryBoard::with(submitted_at(&[BASE]));
    let during = board.clone();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = opened.clone();
    let history = History::while_asked(
        &[(BASE, COMMIT)],
        Box::new(move || seen.lock().unwrap().push(during.transactions().len())),
    );
    let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0), history);
    service
        .execute(verify("parent", json!(1), "stored-token", json!(COMMIT)))
        .unwrap();
    assert_eq!(*opened.lock().unwrap(), [2]);
    assert_eq!(board.transactions().len(), 4);
}
