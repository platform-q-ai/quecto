//! S4d: a session's keeper looks until its run is done or the session
//! ends, and records its renewals (successes folded, each refusal its
//! own) and its fold-back, only while the session records.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardClockError, BoardWrite, ProjectBoardStore, ReferenceStatus,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    PlanState, RunOutcome, TaskKind, TaskStatus,
};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::interface::tools::project_board::recording::tests::{Records, recording};

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

/// A clock standing at 11:30, its monotonic time never moving.
struct HalfPast;

impl BoardClock for HalfPast {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        Ok(at("11:30:00"))
    }
    fn reference(&self) -> Option<Timestamp> {
        None
    }
    fn reference_status(&self) -> ReferenceStatus {
        ReferenceStatus::NoReference("a test".into())
    }
    fn pause(&self, _: usize) {}
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
}

/// A swarm board read as scripted, the last answer standing.
struct Script(Mutex<VecDeque<Value>>);

impl StartedSwarm for Script {
    fn board(&self, _: &str) -> Result<Value, String> {
        let mut answers = self.0.lock().unwrap();
        match answers.len() {
            0 | 1 => answers.front().cloned().ok_or_else(|| "none".into()),
            _ => Ok(answers.pop_front().expect("an answer")),
        }
    }
}

fn board(status: &str) -> Value {
    json!({"run": {"status": status, "budget": {"observed_tokens": 9}},
           "items": [], "members": [{"role": "coordinator", "status": "live"}]})
}

/// Task `t`, started by Ada at 10:00 (held until 12:00), run 1 going.
fn started() -> MemoryBoardStore {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let plan = Plan::new(PlanFields {
        task: slug("t"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Build it.".into(),
        items: vec![PlanItem {
            id: slug("api"),
            title: "Build the api".into(),
            criteria: vec!["built".into()],
            depends_on: vec![],
        }],
        team: None,
        proposed_by: ada(),
        edited_by: None,
        accepted: Some((ada(), at("09:00:00"))),
        created: at("09:00:00"),
        updated: at("09:00:00"),
    })
    .unwrap();
    let task = Task::new(TaskFields {
        id: slug("t"),
        title: "T".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at("09:00:00"),
        updated: at("09:00:00"),
    })
    .unwrap();
    let (task, run) = start(&task, &[], Some(&plan), &ada(), &at("10:00:00"))
        .unwrap()
        .into_writes();
    let head = store.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Task(task),
        BoardArtifact::Plan(plan),
        BoardArtifact::Run(run),
    ];
    let written = store.write(&head, &artifacts, "board: t started");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store
}

fn keeper(store: &MemoryBoardStore, answers: Vec<Value>) -> KeepStartedRun {
    KeepStartedRun::new(
        Arc::new(store.client()),
        Arc::new(HalfPast),
        Arc::new(MarkdownTextCells),
        Arc::new(Script(Mutex::new(answers.into()))),
    )
    .unwrap()
}

fn kept() -> KeptRun {
    KeptRun {
        task: slug("t"),
        run: 1,
        environment_ref: "C1".into(),
    }
}

/// Looks until the swarm ends: the claim, half an hour left, is renewed
/// once and recorded once its keeping ended; the fold-back is recorded.
#[test]
fn a_kept_run_records_its_renewals_and_its_fold_back() {
    let store = started();
    let records = Arc::new(Records::default());
    let answers = vec![board("running"), board("running"), board("succeeded")];
    let end = kept_until_done(
        &keeper(&store, answers),
        &kept(),
        &SessionEnd::default(),
        Some(recording(&records)),
        Duration::ZERO,
    );
    assert_eq!(end, KeeperEnd::Folded);
    let run = store.read().unwrap().runs.into_iter().next().unwrap();
    assert_eq!(run.fields().outcome, Some(RunOutcome::Succeeded));
    let renewals = records.renewals();
    assert_eq!(renewals.len(), 1, "{renewals:?}");
    let line = serde_json::to_value(&renewals[0]).unwrap();
    assert_eq!(
        line,
        json!({"task": "t", "run": 1, "outcome": "ok", "renewals": 1, "min_left_s": 1800})
    );
    assert_eq!(records.fold_backs().len(), 1);
}

/// With the session's records off, the keeper keeps the run all the same
/// and records nothing.
#[test]
fn a_keeper_unrecorded_keeps_the_run_and_records_nothing() {
    let store = started();
    let end = kept_until_done(
        &keeper(&store, vec![board("cancelled")]),
        &kept(),
        &SessionEnd::default(),
        None,
        Duration::ZERO,
    );
    assert_eq!(end, KeeperEnd::Folded);
}

/// A session that ended stops its keeper before its next look: nothing is
/// read or written.
#[test]
fn a_session_end_stops_the_keeper_before_its_next_look() {
    let store = started();
    let session = SessionEnd::default();
    session.end();
    let writes = store.writes();
    let end = kept_until_done(
        &keeper(&store, vec![]),
        &kept(),
        &session,
        None,
        Duration::from_secs(3600),
    );
    assert_eq!(end, KeeperEnd::SessionEnded);
    assert_eq!(store.writes(), writes);
    assert!(!session.wait(Duration::from_secs(3600)), "it stays ended");
}

/// A session ending while its keeper waits wakes the keeper at once.
#[test]
fn a_session_ending_wakes_a_waiting_keeper() {
    let session = Arc::new(SessionEnd::default());
    let waiting = Arc::clone(&session);
    let waiter = std::thread::spawn(move || waiting.wait(Duration::from_secs(3600)));
    std::thread::sleep(Duration::from_millis(20));
    session.end();
    assert!(!waiter.join().unwrap());
    assert!(
        SessionEnd::default().wait(Duration::ZERO),
        "a live session goes on"
    );
}

/// Consecutive renewals fold into one record with the least time left;
/// each refusal ends them and is a record of its own with its codes; a
/// swarm given up is `swarm_lost`.
#[test]
fn renewals_fold_and_each_refusal_is_its_own_record() {
    let records = Arc::new(Records::default());
    let mut renewals = Renewals::new(recording(&records), &kept());
    renewals.note(&Renewal::Renewed { left_s: 2_000 });
    renewals.note(&Renewal::Renewed { left_s: 1_500 });
    renewals.note(&Renewal::Renewed { left_s: 1_800 });
    renewals.note(&Renewal::Refused {
        left_s: 900,
        error: ProjectBoardError::Rule(BoardRuleError::NotHolder),
    });
    renewals.note(&Renewal::Renewed { left_s: 700 });
    renewals.lost();
    renewals.flush();
    let lines: Vec<Value> = records
        .renewals()
        .iter()
        .map(|record| serde_json::to_value(record).unwrap())
        .collect();
    assert_eq!(
        lines,
        vec![
            json!({"task": "t", "run": 1, "outcome": "ok", "renewals": 3, "min_left_s": 1500}),
            json!({"task": "t", "run": 1, "outcome": "refused", "code": "rule",
                   "rule": "not_holder", "renewals": 0, "min_left_s": 900}),
            json!({"task": "t", "run": 1, "outcome": "ok", "renewals": 1, "min_left_s": 700}),
            json!({"task": "t", "run": 1, "outcome": "refused", "code": "swarm_lost",
                   "renewals": 0}),
        ]
    );
}
