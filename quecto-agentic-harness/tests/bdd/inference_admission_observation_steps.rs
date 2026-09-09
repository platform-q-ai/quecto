//! P4 observation scenarios over a real in-process authority; the state and
//! helpers of the authority steps are reused, the observation vocabulary is
//! proven through the public read port only.

use super::inference_admission_authority_steps::{LIMIT, group, new_root, rt};
use super::*;
use quecto::application::ports::AttemptPermit;
use quecto::application::ports::{AdmissionObservation, AttemptAdmission};
use quecto::domain::inference_admission::{Feedback, GroupId};
use quecto::infrastructure::admission::AuthorityConnection;
use quecto::infrastructure::admission::{AdmissionRecorder, ObservedAdmission};
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
pub struct ObservationState {
    recorder: Option<Arc<AdmissionRecorder>>,
    observer: Option<AuthorityConnection>,
    pending: Option<tokio::task::JoinHandle<Result<Box<dyn AttemptPermit>, String>>>,
}
impl std::fmt::Debug for ObservationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<admission observation scenario>")
    }
}

#[when("an observed process attempt queues behind the holder")]
fn when_observed_attempt_queues(world: &mut QuectoWorld) {
    let recorder = Arc::new(AdmissionRecorder::new());
    let s = &mut world.authority;
    let observer = new_root(s);
    let gate = Arc::new(ObservedAdmission::new(
        observer.gate("acct").unwrap(),
        "acct",
        group(),
        recorder.clone(),
    ));
    let pending = rt(s).spawn(async move { gate.acquire().await.map_err(|e| e.to_string()) });
    let start = std::time::Instant::now();
    while recorder.snapshot().waiting == 0 {
        assert!(start.elapsed() < LIMIT, "attempt never queued");
        std::thread::sleep(Duration::from_millis(5));
    }
    let o = &mut world.authority_observation;
    o.recorder = Some(recorder);
    o.observer = Some(observer);
    o.pending = Some(pending);
}

#[then(expr = "the process observes one waiting attempt in group {string} with a growing wait")]
fn then_waiting_observed(world: &mut QuectoWorld, expected_group: String) {
    let o = &mut world.authority_observation;
    let recorder = o.recorder.as_ref().unwrap();
    let first = recorder.snapshot();
    assert_eq!(first.waiting, 1, "{first:?}");
    let attempt = first.attempts.values().next().unwrap();
    assert_eq!(attempt.group, GroupId::new(&expected_group).unwrap());
    assert!(matches!(
        attempt.phase,
        quecto::domain::inference_admission::AdmissionPhase::Waiting { .. }
    ));
    std::thread::sleep(Duration::from_millis(30));
    let second = recorder.snapshot();
    let later = second.attempts.values().next().unwrap();
    assert!(later.elapsed_ms > attempt.elapsed_ms, "wait keeps growing");
    assert!(
        second.observed_at_ms > first.observed_at_ms,
        "freshness advances"
    );
}

#[when("the holding root completes its attempt")]
fn when_holder_completes(world: &mut QuectoWorld) {
    let s = &mut world.authority;
    let permit = s.permits.pop().expect("held permit");
    let _guard = rt(s).enter();
    permit.finish(Feedback::Success);
}

#[then("the process observes the attempt admitted and then completed")]
fn then_admitted_then_completed(world: &mut QuectoWorld) {
    let pending = world.authority_observation.pending.take().unwrap();
    let permit = rt(&world.authority)
        .block_on(async { tokio::time::timeout(LIMIT, pending).await })
        .unwrap()
        .unwrap()
        .expect("granted after the holder released");
    let recorder = world.authority_observation.recorder.clone().unwrap();
    let admitted = recorder.snapshot();
    assert_eq!(
        (admitted.waiting, admitted.admitted),
        (0, 1),
        "{admitted:?}"
    );
    let _guard = rt(&world.authority).enter();
    permit.finish(Feedback::Success);
    let done = recorder.snapshot();
    assert_eq!((done.admitted, done.completed), (0, 1), "{done:?}");
    assert!(
        done.attempts.is_empty(),
        "released attempts leave the live view"
    );
}
