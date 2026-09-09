//! Recorder invariants that do not need an authority: bounded live view,
//! saturating counters, cooldown translation and cancellation on drop.
use super::*;
use crate::domain::error::DomainError;
use std::future::Future;
use std::pin::Pin;

#[derive(Debug)]
struct Never;
impl AttemptAdmission for Never {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        Box::pin(std::future::pending())
    }
}

#[derive(Debug)]
struct Refuse;
impl AttemptAdmission for Refuse {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        Box::pin(async { Err(DomainError::Provider("admission: Quarantined".into())) })
    }
}

#[derive(Debug)]
struct Grant;
#[derive(Debug)]
struct Plain;
impl AttemptPermit for Plain {
    fn feedback(&mut self, _: ThrottleFeedback) {}
    fn finish(self: Box<Self>, _: Feedback) {}
}
impl AttemptAdmission for Grant {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        Box::pin(async { Ok(Box::new(Plain) as Box<dyn AttemptPermit>) })
    }
}

fn observed(
    inner: Arc<dyn AttemptAdmission>,
    recorder: &Arc<AdmissionRecorder>,
) -> ObservedAdmission {
    ObservedAdmission::new(inner, "acct", GroupId::new("g").unwrap(), recorder.clone())
}

#[tokio::test]
async fn live_view_is_bounded_and_waits_are_never_leaked() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = observed(Arc::new(Never), &recorder);
    let mut waits: Vec<Pin<Box<dyn Future<Output = _> + Send + '_>>> = Vec::new();
    for _ in 0..(AdmissionActivity::MAX_LIVE_ATTEMPTS + 8) {
        let mut wait = gate.acquire();
        // Register the attempt without completing it.
        let _ = futures::poll!(wait.as_mut());
        waits.push(wait);
    }
    let view = recorder.snapshot();
    assert_eq!(view.attempts.len(), AdmissionActivity::MAX_LIVE_ATTEMPTS);
    assert_eq!(view.waiting, AdmissionActivity::MAX_LIVE_ATTEMPTS);
    drop(waits);
    let after = recorder.snapshot();
    assert_eq!(
        after.waiting, 0,
        "dropped waits are cancellations, not live attempts"
    );
    assert_eq!(after.cancelled, AdmissionActivity::MAX_LIVE_ATTEMPTS as u64);
}

#[tokio::test]
async fn refusal_reason_is_recorded_and_truncated() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = observed(Arc::new(Refuse), &recorder);
    assert!(gate.acquire().await.is_err());
    let view = recorder.snapshot();
    assert_eq!(view.refused, 1);
    assert!(
        view.last_refusal
            .as_deref()
            .unwrap()
            .contains("Quarantined")
    );
    assert!(view.last_refusal.as_deref().unwrap().len() <= 200);
}

#[tokio::test]
async fn a_dropped_permit_is_a_cancellation_and_a_finished_one_a_completion() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = observed(Arc::new(Grant), &recorder);
    let permit = gate.acquire().await.unwrap();
    assert_eq!(recorder.snapshot().admitted, 1);
    drop(permit);
    let dropped = recorder.snapshot();
    assert_eq!(
        (dropped.admitted, dropped.cancelled, dropped.completed),
        (0, 1, 0)
    );
    let mut permit = gate.acquire().await.unwrap();
    permit.feedback(ThrottleFeedback::NoHint { jitter: 1 });
    assert_eq!(
        recorder.snapshot().cooldown_until_ms,
        None,
        "no hint, no visible deadline"
    );
    permit.finish(Feedback::Throttle { delay_ms: 5_000 });
    let finished = recorder.snapshot();
    assert_eq!((finished.admitted, finished.completed), (0, 1));
    assert!(
        finished
            .cooldown_until_ms
            .is_some_and(|until| until > finished.observed_at_ms)
    );
}
