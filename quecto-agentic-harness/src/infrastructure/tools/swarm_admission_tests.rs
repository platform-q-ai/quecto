//! A reservation's board calls finish before the call that made them
//! returns (#2329 final review): a dropped, never-launched reservation's
//! release, and a cancelled rollback's confirmation before any release.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use super::LaunchReservation;
use crate::application::swarm::ports::BoardOpLog;
use crate::domain::swarm::{BoardOpObservation, MemberExit};
use crate::infrastructure::tools::call_work::off_the_runtime;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// The event log, in memory, slow to record one op: a call of that op
/// returns only once its record is written, so the record's presence
/// says the call has returned.
struct SlowLog {
    slow_op: &'static str,
    ops: Mutex<Vec<String>>,
}

impl BoardOpLog for SlowLog {
    fn record(&self, observation: BoardOpObservation) {
        if observation.op == self.slow_op {
            std::thread::sleep(Duration::from_millis(300));
        }
        self.ops
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(observation.op);
    }
}

impl SlowLog {
    fn ops(&self) -> Vec<String> {
        self.ops
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// A coordinator's context over a created run of two members, whose board
/// records in a log slow to record `slow_op`.
fn recorded_run(checkout: &std::path::Path, slow_op: &'static str) -> (SwarmContext, Arc<SlowLog>) {
    let log = Arc::new(SlowLog {
        slow_op,
        ops: Mutex::default(),
    });
    let board = crate::composition::swarm::swarm_board();
    assert!(board.record_in(log.clone()));
    let context = SwarmContext {
        board,
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: checkout.to_path_buf(),
        member: "parent".into(),
    };
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    off_the_runtime(|| {
        context.call(
            "create",
            json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], 2, deadline]),
        )
    })
    .unwrap();
    (context, log)
}

fn usage(context: &SwarmContext) -> serde_json::Value {
    off_the_runtime(|| context.summary()).unwrap()["usage"].clone()
}

/// Dropping a never-launched reservation on an async worker: the release
/// has been made, and the slot is free, when the drop returns.
async fn dropped_on_a_worker_releases_before_the_drop_returns() {
    let directory = tempfile::tempdir().unwrap();
    let (context, log) = recorded_run(directory.path(), "_release_unlaunched");
    let reservation = off_the_runtime(|| LaunchReservation::reserve(context.clone())).unwrap();
    assert_eq!(usage(&context), 2);
    drop(reservation);
    assert!(
        log.ops().iter().any(|op| op == "_release_unlaunched"),
        "released before the drop returned: {:?}",
        log.ops()
    );
    assert_eq!(usage(&context), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_reservation_is_released_before_the_drop_returns_on_a_multi_thread_worker() {
    dropped_on_a_worker_releases_before_the_drop_returns().await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_dropped_reservation_is_released_before_the_drop_returns_on_a_current_thread_runtime() {
    dropped_on_a_worker_releases_before_the_drop_returns().await;
}

#[test]
fn a_dropped_reservation_is_released_before_the_drop_returns_outside_any_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let (context, log) = recorded_run(directory.path(), "_release_unlaunched");
    drop(LaunchReservation::reserve(context.clone()).unwrap());
    assert!(log.ops().iter().any(|op| op == "_release_unlaunched"));
    assert_eq!(usage(&context), 1);
}

/// A rollback cancelled after its first poll, then its reservation dropped:
/// the member's death is confirmed before any release of its slot, never
/// beside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_rollback_confirms_the_death_before_any_release() {
    let directory = tempfile::tempdir().unwrap();
    let (context, log) = recorded_run(directory.path(), "_confirmed_dead");
    let mut reservation = off_the_runtime(|| LaunchReservation::reserve(context.clone())).unwrap();
    {
        let rolled_back = std::pin::pin!(reservation.rolled_back(MemberExit::Orderly));
        let _first_poll = futures::poll!(rolled_back);
    }
    drop(reservation);
    let settled = std::time::Instant::now() + Duration::from_secs(10);
    while !log.ops().iter().any(|op| op == "_confirmed_dead") {
        assert!(std::time::Instant::now() < settled, "{:?}", log.ops());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Give a release still running beside it the time to be recorded.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ops = log.ops();
    let confirmed = ops.iter().position(|op| op == "_confirmed_dead").unwrap();
    assert!(
        ops.iter()
            .position(|op| op == "_release_unlaunched")
            .is_none_or(|released| released > confirmed),
        "{ops:?}"
    );
}
