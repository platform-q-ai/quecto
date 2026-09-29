//! #2304 review round 3 (M4): `close` and `finish` never wait forever for
//! an ended member's end to be recorded. The exit's work marks the end
//! recorded however it ends (done, panicked or dropped unrun); closing the
//! input counts against [`EXIT_GRACE`]; and both wait at most
//! [`EXIT_GRACE`] plus [`END_RECORD_MARGIN`] even for work that never runs.
//! Work that panics or is dropped unrun still records the end, as an
//! unknown exit (round 4).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::test_rig::*;
use crate::application::external_agent::dto::{END_RECORD_MARGIN, EXIT_GRACE, SessionRecord};
use crate::application::external_agent::ports::{DetachedWork, ExternalAgentSpawner};

/// What "at once" means here: well under a second.
const PROMPTLY: Duration = Duration::from_millis(500);

/// Drops the work it is handed without running it.
struct DroppingSpawner;

impl ExternalAgentSpawner for DroppingSpawner {
    fn spawn(&self, work: DetachedWork) {
        drop(work);
    }
}

/// Keeps the work it is handed and never runs it.
#[derive(Default)]
struct HoardingSpawner(Mutex<Vec<DetachedWork>>);

impl ExternalAgentSpawner for HoardingSpawner {
    fn spawn(&self, work: DetachedWork) {
        self.0.lock().unwrap().push(work);
    }
}

/// #2304 review round 4: the `ended` records with no exit observed:
/// `clean: false`, no exit code and no signal.
fn unknown_ends(rig: &Rig) -> usize {
    rig.records
        .all()
        .iter()
        .filter(|record| {
            matches!(
                record,
                SessionRecord::Ended {
                    clean: false,
                    exit_code: None,
                    signal: None,
                    ..
                }
            )
        })
        .count()
}

fn ends(rig: &Rig) -> usize {
    rig.records
        .all()
        .iter()
        .filter(|record| matches!(record, SessionRecord::Ended { .. }))
        .count()
}

#[tokio::test(start_paused = true)]
async fn a_panicking_exit_wait_releases_close_and_finish_at_once() {
    let rig = started().await;
    rig.wire.panic_exit.store(true, Ordering::SeqCst);
    let closed = tokio::time::timeout(PROMPTLY, rig.session.close()).await;
    assert!(
        closed.is_ok_and(|closed| closed.is_ok()),
        "close returns once the exit's work has panicked"
    );
    tokio::time::timeout(PROMPTLY, rig.session.finish())
        .await
        .expect("finish returns at once");
    assert_eq!(
        unknown_ends(&rig),
        1,
        "a panicked wait records an unknown end: {:?}",
        rig.records.kinds()
    );
}

#[tokio::test(start_paused = true)]
async fn exit_work_dropped_unrun_releases_close_and_finish_at_once() {
    let rig = rig_spawning(false, Arc::new(DroppingSpawner));
    rig.session.start().await.unwrap();
    tokio::time::timeout(PROMPTLY, rig.session.close())
        .await
        .expect("close returns once the work is dropped")
        .unwrap();
    tokio::time::timeout(PROMPTLY, rig.session.finish())
        .await
        .expect("finish returns at once");
    assert_eq!(
        unknown_ends(&rig),
        1,
        "work dropped unrun records an unknown end: {:?}",
        rig.records.kinds()
    );
}

#[tokio::test(start_paused = true)]
async fn exit_work_never_run_holds_close_and_finish_only_to_the_bound() {
    let spawner = Arc::new(HoardingSpawner::default());
    let rig = rig_spawning(false, spawner.clone());
    rig.session.start().await.unwrap();
    let bound = EXIT_GRACE + END_RECORD_MARGIN;
    let started = tokio::time::Instant::now();
    tokio::time::timeout(bound + PROMPTLY, rig.session.close())
        .await
        .expect("close is bounded")
        .unwrap();
    assert!(started.elapsed() >= bound, "{:?}", started.elapsed());
    tokio::time::timeout(bound + PROMPTLY, rig.session.finish())
        .await
        .expect("finish is bounded");
    assert_eq!(
        spawner.0.lock().unwrap().len(),
        1,
        "the work was handed off"
    );
    assert_eq!(ends(&rig), 0, "work still held records no end yet");
}

#[tokio::test(start_paused = true)]
async fn an_input_that_never_closes_counts_against_the_exit_grace() {
    let rig = started().await;
    rig.wire.hang_close_input.store(true, Ordering::SeqCst);
    let started = tokio::time::Instant::now();
    tokio::time::timeout(EXIT_GRACE + PROMPTLY, rig.session.close())
        .await
        .expect("close returns at the grace")
        .unwrap();
    assert!(started.elapsed() >= EXIT_GRACE, "{:?}", started.elapsed());
    assert_eq!(
        rig.records.all().last(),
        Some(&SessionRecord::Ended {
            clean: false,
            exit_code: None,
            signal: None,
            wall_ms: Some(EXIT_GRACE.as_millis() as u64),
        }),
        "the end is recorded unobserved, at the grace"
    );
}
