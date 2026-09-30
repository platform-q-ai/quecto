use std::cell::RefCell;

use serde_json::{Value, json};

use super::{LOSS_GRACE, end_by_loss, grace_elapsed, lost};
use crate::application::swarm::board_test_support::{
    MemoryBoard, RecordedEvent, SteppingClock, member_row, paused, recorded, running_board,
};
use crate::application::swarm::dto::{DictRow, LatestActivity, ScopeObservation};
use crate::application::swarm::ports::{BoardEvents, BoardRepository};
use crate::domain::swarm::{BoardError, RefusalKind, RunRecord, RunState};

/// Loss observations as scripted, and the events written.
#[derive(Default)]
struct Observations {
    stored: Vec<ScopeObservation>,
    written: RefCell<Vec<(String, f64, String, Value)>>,
}

impl BoardEvents for Observations {
    fn event(
        &self,
        actor: &str,
        time: f64,
        action: &str,
        detail: &Value,
    ) -> Result<(), BoardError> {
        self.written
            .borrow_mut()
            .push((actor.to_owned(), time, action.to_owned(), detail.clone()));
        Ok(())
    }

    fn control_generation(&self) -> Result<i64, BoardError> {
        unreachable!("the grace reads no control generation")
    }

    fn created_at(&self) -> Result<Option<f64>, BoardError> {
        unreachable!("the grace reads no creation time")
    }

    fn scope_observations(&self) -> Result<Vec<ScopeObservation>, BoardError> {
        Ok(self.stored.clone())
    }

    fn latest_activity(&self, _actors: &[&str]) -> Result<Vec<LatestActivity>, BoardError> {
        unreachable!("the grace reads no activity")
    }

    fn event_time(&self, _id: i64) -> Result<Option<Value>, BoardError> {
        unreachable!("the grace reads no event time")
    }

    fn event_page(&self, _after: u64, _limit: i64) -> Result<Vec<DictRow>, BoardError> {
        unreachable!("the grace reads no event page")
    }
}

fn observed(actor: &str, time: Value, member: Value) -> ScopeObservation {
    ScopeObservation {
        actor: Some(actor.to_owned()),
        time,
        member,
    }
}

fn elapsed(stored: Vec<ScopeObservation>, now: f64) -> (Result<bool, BoardError>, usize) {
    let events = Observations {
        stored,
        ..Observations::default()
    };
    let answer = grace_elapsed(&events, &*SteppingClock::fixed(now), "parent", &json!("w"));
    let written = events.written.borrow().len();
    (answer, written)
}

/// The `for … else`'s `else` branch: no observation of the actor's own,
/// so one is recorded now, and the grace runs from the earliest (its own
/// when there is none).
#[test]
fn without_its_own_observation_the_actor_records_one() {
    let events = Observations::default();
    let clock = SteppingClock::fixed(100.0);
    assert!(!grace_elapsed(&events, &*clock, "parent", &json!("w")).unwrap());
    assert_eq!(
        *events.written.borrow(),
        [(
            "parent".to_owned(),
            100.0,
            "scope_observed".to_owned(),
            json!({"member": "w"})
        )]
    );
    // Another observer's earlier one is where the grace runs from.
    let other = vec![observed("other", json!(90.0), json!("w"))];
    assert_eq!(elapsed(other, 100.0).0, Ok(true));
    let other = vec![observed("other", json!(90.5), json!("w"))];
    assert_eq!(elapsed(other, 100.0), (Ok(false), 1));
}

/// The loop's `break`: the actor's own observation ends the scan, and
/// nothing is recorded; the grace runs from the earliest observation of
/// the member, not the actor's own, and never from another member's.
#[test]
fn the_actors_own_observation_ends_the_scan_and_records_nothing() {
    let stored = vec![
        observed("other", json!(80.0), json!("x")),
        observed("other", json!(85.0), json!("w")),
        observed("parent", json!(99.0), json!("w")),
    ];
    assert_eq!(elapsed(stored.clone(), 95.0), (Ok(true), 0));
    assert_eq!(elapsed(stored, 94.9), (Ok(false), 0));
    let own = vec![observed("parent", json!(90.0), json!("w"))];
    assert_eq!(elapsed(own.clone(), 100.0), (Ok(true), 0));
    assert_eq!(elapsed(own, 100.0 - LOSS_GRACE / 2.0), (Ok(false), 0));
}

/// Python's `if first is None`: a NULL time leaves the earliest unset for
/// the next observation of the member; one that is still unset after the
/// actor's own is not a time to measure from, and neither is text.
#[test]
fn an_observation_time_is_read_as_python_reads_it() {
    let null_then_timed = vec![
        observed("other", Value::Null, json!("w")),
        observed("parent", json!(90.0), json!("w")),
    ];
    assert_eq!(elapsed(null_then_timed, 100.0), (Ok(true), 0));
    let null_unowned = vec![observed("other", Value::Null, json!("w"))];
    assert_eq!(elapsed(null_unowned, 100.0), (Ok(false), 1));
    let edited = BoardError::new(
        RefusalKind::Store,
        "the board's loss observation is not as the board writes it",
    );
    for stored in [
        vec![observed("parent", Value::Null, json!("w"))],
        vec![observed("parent", json!("soon"), json!("w"))],
        vec![
            observed("other", json!("soon"), json!("w")),
            observed("parent", json!(1.0), json!("w")),
        ],
    ] {
        assert_eq!(elapsed(stored, 100.0).0, Err(edited.clone()));
    }
}

fn ended(
    state: crate::application::swarm::board_test_support::BoardState,
) -> (bool, RunRecord, Vec<RecordedEvent>) {
    let board = MemoryBoard::with(state);
    let run = board.snapshot().run.unwrap().record;
    let mut changed = false;
    board
        .atomic(false, &mut |transaction| {
            changed = end_by_loss(
                transaction,
                &*SteppingClock::fixed(7.0),
                "parent",
                &run,
                "gone",
            )?;
            Ok(())
        })
        .unwrap();
    let after = board.snapshot();
    (changed, after.run.unwrap().record, after.events)
}

/// `_end_by_loss` in each run state.
#[test]
fn a_loss_ends_each_run_state_as_python_does() {
    let stop = recorded(
        "parent",
        "stop",
        json!({"status": "failed", "reason": "gone"}),
    );
    let at = |mut event: RecordedEvent| {
        event.time = 7.0;
        event
    };
    let mut setup = running_board(100.0);
    setup.run.as_mut().unwrap().record.status = Some(RunState::SETUP);
    let (changed, run, events) = ended(setup);
    assert!(changed);
    assert_eq!((run.status, run.outcome), (Some(RunState::FAILED), None));
    assert_eq!(events, [at(stop.clone())]);

    let (changed, run, events) = ended(running_board(100.0));
    assert!(changed);
    assert_eq!(
        (
            run.status,
            run.outcome.as_deref(),
            run.outcome_reason.as_deref()
        ),
        (Some(RunState::PAUSED), Some("failed"), Some("gone"))
    );
    let pause = recorded(
        "parent",
        "paused",
        json!({"reason": "gone", "started": 7.0, "outcome": "failed"}),
    );
    assert_eq!(events, [at(stop.clone()), at(pause)]);

    let (changed, run, events) = ended(paused(running_board(100.0), 3.0, None));
    assert!(changed);
    assert_eq!(
        (run.status, run.outcome.as_deref()),
        (Some(RunState::PAUSED), Some("failed"))
    );
    assert_eq!(events.len(), 2, "the pause record, then the stop");
    assert_eq!(events[1], at(stop));

    for held in [
        paused(running_board(100.0), 3.0, Some(("succeeded", "done"))),
        {
            let mut ended = running_board(100.0);
            ended.run.as_mut().unwrap().record.status = Some(RunState::CANCELLED);
            ended
        },
    ] {
        let before = held.run.as_ref().unwrap().record.clone();
        let events_before = held.events.len();
        let (changed, run, events) = ended(held);
        assert!(!changed);
        assert_eq!((run, events.len()), (before, events_before));
    }
    // An empty outcome is no outcome, as Python's truth of it.
    let (changed, run, _) = ended(paused(running_board(100.0), 3.0, Some(("", "r"))));
    assert!(changed);
    assert_eq!(run.outcome.as_deref(), Some("failed"));
}

/// `_lost`: no row, a status that is not alive (dead, or one only an edit
/// writes), or a loss recorded after the latest activation.
#[test]
fn a_member_is_lost_unless_alive_and_not_recorded_lost() {
    let mut state = running_board(100.0);
    for (id, status) in [
        ("live", "live"),
        ("reserved", "reserved"),
        ("dead", "dead"),
        ("zombie", "zombie"),
        ("quarantined", "live"),
        ("back", "live"),
    ] {
        state.members.push(member_row(id, status));
    }
    for (action, member) in [
        ("activated", "quarantined"),
        ("scope_unknown", "quarantined"),
        ("scope_unknown", "back"),
        ("activated", "back"),
    ] {
        state
            .events
            .push(recorded("parent", action, json!({"member": member})));
    }
    let board = MemoryBoard::with(state);
    board
        .atomic(false, &mut |transaction| {
            for (member, expected) in [
                (json!("live"), false),
                (json!("reserved"), false),
                (json!("back"), false),
                (json!("dead"), true),
                (json!("zombie"), true),
                (json!("quarantined"), true),
                (json!("stranger"), true),
                (Value::Null, true),
            ] {
                assert_eq!(lost(transaction, &member)?, expected, "{member}");
            }
            Ok(())
        })
        .unwrap();
}
