//! Recorder invariants that do not need an authority: exact counts with a
//! bounded sample, byte-bounded refusal reasons, grant-anchored cooldown
//! translation per group, and the cancellation/abandonment distinction.
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
struct Refuse(String);
impl AttemptAdmission for Refuse {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        let reason = self.0.clone();
        Box::pin(async move { Err(DomainError::Provider(reason)) })
    }
}

/// What the inner permit received, so forwarding is provable.
#[derive(Debug, Default)]
struct Forwarded {
    feedback: Vec<ThrottleFeedback>,
    no_hint: usize,
    finished: Vec<Feedback>,
}

/// Grants immediately; the permit reports a fixed authority receipt clock and
/// records everything forwarded to it.
#[derive(Debug)]
struct Grant {
    receipt_ms: u64,
    maximum_ms: u64,
    forwarded: Arc<Mutex<Forwarded>>,
}
impl Grant {
    fn new(receipt_ms: u64, maximum_ms: u64) -> Self {
        Self {
            receipt_ms,
            maximum_ms,
            forwarded: Arc::new(Mutex::new(Forwarded::default())),
        }
    }
}
#[derive(Debug)]
struct Plain {
    receipt_ms: u64,
    maximum_ms: u64,
    forwarded: Arc<Mutex<Forwarded>>,
}
impl AttemptPermit for Plain {
    fn receipt_clock(&self) -> (u64, SystemTime) {
        (self.receipt_ms, SystemTime::UNIX_EPOCH)
    }
    fn maximum_cooldown_ms(&self) -> u64 {
        self.maximum_ms
    }
    fn throttle_without_hint(&mut self) {
        self.forwarded.lock().unwrap().no_hint += 1;
    }
    fn feedback(&mut self, feedback: ThrottleFeedback) {
        self.forwarded.lock().unwrap().feedback.push(feedback);
    }
    fn finish(self: Box<Self>, feedback: Feedback) {
        self.forwarded.lock().unwrap().finished.push(feedback);
    }
}
impl AttemptAdmission for Grant {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        let permit = Plain {
            receipt_ms: self.receipt_ms,
            maximum_ms: self.maximum_ms,
            forwarded: self.forwarded.clone(),
        };
        Box::pin(async move { Ok(Box::new(permit) as Box<dyn AttemptPermit>) })
    }
}

fn g(name: &str) -> GroupId {
    GroupId::new(name).unwrap()
}

fn observed(
    inner: Arc<dyn AttemptAdmission>,
    alias: &str,
    group: &str,
    recorder: &Arc<AdmissionRecorder>,
) -> ObservedAdmission {
    ObservedAdmission::new(inner, alias, g(group), recorder.clone())
}

#[tokio::test]
async fn counts_stay_exact_beyond_the_bounded_sample() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = observed(Arc::new(Never), "acct", "g", &recorder);
    let total = AdmissionActivity::MAX_LIVE_ATTEMPTS + 8;
    let mut waits: Vec<Pin<Box<dyn Future<Output = _> + Send + '_>>> = Vec::new();
    for _ in 0..total {
        let mut wait = gate.acquire();
        let _ = futures::poll!(wait.as_mut());
        waits.push(wait);
    }
    let view = recorder.snapshot();
    assert_eq!(view.waiting, total, "counts are exact");
    assert_eq!(view.attempts.len(), AdmissionActivity::MAX_LIVE_ATTEMPTS);
    assert_eq!(view.hidden, 8, "the rest is reported as hidden");
    assert_eq!(
        *view.attempts.keys().next().unwrap(),
        1,
        "the sample keeps the oldest"
    );
    drop(waits);
    let after = recorder.snapshot();
    assert_eq!((after.waiting, after.hidden), (0, 0));
    assert_eq!(
        after.cancelled, total as u64,
        "every dropped wait is counted"
    );
}

#[tokio::test]
async fn refusal_reason_is_recorded_per_group_and_byte_bounded() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let long = "é".repeat(150); // 300 bytes, 150 chars
    let gate = observed(Arc::new(Refuse(long)), "acct", "g", &recorder);
    assert!(gate.acquire().await.is_err());
    let other = observed(Arc::new(Never), "b", "h", &recorder);
    let mut wait = other.acquire();
    let _ = futures::poll!(wait.as_mut());
    let view = recorder.snapshot();
    assert_eq!(view.refused, 1);
    let reason = view.groups[&g("g")].last_refusal.as_deref().unwrap();
    assert!(reason.len() <= AdmissionActivity::MAX_REFUSAL_BYTES);
    assert!(
        reason.len() >= AdmissionActivity::MAX_REFUSAL_BYTES - 1,
        "cut at a char boundary"
    );
    assert!(
        reason.ends_with('é'),
        "the multibyte tail survived intact: {reason:?}"
    );
    assert!(reason.chars().filter(|c| *c == 'é').count() >= 60);
    assert_eq!(
        view.groups[&g("h")].last_refusal,
        None,
        "other groups untouched"
    );
}

#[tokio::test]
async fn dropped_permit_is_abandonment_and_finish_is_completion() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = observed(Arc::new(Grant::new(1_000, 10_000)), "acct", "g", &recorder);
    let permit = gate.acquire().await.unwrap();
    assert_eq!(recorder.snapshot().admitted, 1);
    drop(permit);
    let dropped = recorder.snapshot();
    assert_eq!(
        (
            dropped.admitted,
            dropped.abandoned,
            dropped.cancelled,
            dropped.completed
        ),
        (0, 1, 0, 0),
        "a dropped permit is not a cancelled wait"
    );
    let permit = gate.acquire().await.unwrap();
    permit.finish(Feedback::Throttle { delay_ms: 5_000 });
    let finished = recorder.snapshot();
    assert_eq!((finished.admitted, finished.completed), (0, 1));
    assert!(matches!(
        finished.groups[&g("g")].cooldown,
        Some(CooldownState::Until { until_ms }) if until_ms > finished.observed_at_ms
    ));
}

#[tokio::test]
async fn cooldown_is_anchored_at_the_grant_and_kept_per_group() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let inner = Arc::new(Grant::new(5_000, 10_000));
    let forwarded = inner.forwarded.clone();
    let gate = observed(inner, "acct", "g", &recorder);
    let mut permit = gate.acquire().await.unwrap();
    let AdmissionPhase::Admitted { since_ms } = recorder.snapshot().attempts[&1].phase else {
        panic!("granted")
    };
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    // Authority says "until receipt + 3000": locally that is exactly grant +
    // 3000, not "now + 3000" (which would drift later by the transport time).
    permit.feedback(ThrottleFeedback::Until(5_000 + 3_000));
    let view = recorder.snapshot();
    assert_eq!(
        view.groups[&g("g")].cooldown,
        Some(CooldownState::Until {
            until_ms: since_ms + 3_000
        }),
        "{view:?}"
    );
    // Shorter advice never shortens; longer advice extends (max-merge).
    permit.feedback(ThrottleFeedback::Until(5_000 + 1_000));
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Until {
            until_ms: since_ms + 3_000
        })
    );
    permit.feedback(ThrottleFeedback::Until(5_000 + 4_000));
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Until {
            until_ms: since_ms + 4_000
        })
    );
    // Advice is judged by the delay remaining when it arrives, as the
    // authority does: 40 ms into the grant, "max + 10 ms" is still acceptable.
    permit.feedback(ThrottleFeedback::Until(5_000 + 10_010));
    assert!(matches!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Until { .. })
    ));
    assert_eq!(
        forwarded.lock().unwrap().feedback.len(),
        4,
        "every advice reached the inner permit"
    );
    // Another group is unaffected.
    let other = observed(Arc::new(Never), "b", "h", &recorder);
    let mut wait = other.acquire();
    let _ = futures::poll!(wait.as_mut());
    assert_eq!(recorder.snapshot().groups[&g("h")].cooldown, None);
    permit.finish(Feedback::Success);
}

#[tokio::test]
async fn no_hint_and_unavailable_throttles_are_visible_states() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let inner = Arc::new(Grant::new(0, 1_000));
    let forwarded = inner.forwarded.clone();
    let gate = observed(inner, "acct", "g", &recorder);
    let mut permit = gate.acquire().await.unwrap();
    permit.throttle_without_hint();
    assert_eq!(
        forwarded.lock().unwrap().no_hint,
        1,
        "forwarded to the inner permit"
    );
    assert!(matches!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Unknown { .. })
    ));
    permit.finish(Feedback::Success);
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        None,
        "success ends an open-ended throttle"
    );
    let mut permit = gate.acquire().await.unwrap();
    // Advice beyond the maximum is what the authority treats as unavailable.
    permit.feedback(ThrottleFeedback::Until(5_000));
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Unavailable)
    );
    permit.feedback(ThrottleFeedback::Until(10));
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Unavailable),
        "unavailable is sticky against shorter advice"
    );
    permit.finish(Feedback::Failure);
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Unavailable),
        "a failure does not clear the group state"
    );
    let permit = gate.acquire().await.unwrap();
    let mut permit = permit;
    permit.feedback(ThrottleFeedback::Unavailable);
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Unavailable)
    );
    permit.throttle_without_hint();
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Unavailable),
        "no-hint advice never downgrades unavailable"
    );
    permit.finish(Feedback::Failure);
    assert_eq!(
        forwarded.lock().unwrap().finished,
        vec![Feedback::Success, Feedback::Failure, Feedback::Failure],
        "every completion reached the inner permit"
    );
}

#[tokio::test]
async fn completion_throttle_is_clamped_and_expired_cooldowns_read_as_none() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = observed(Arc::new(Grant::new(0, 1_000)), "acct", "g", &recorder);
    let permit = gate.acquire().await.unwrap();
    permit.finish(Feedback::Throttle { delay_ms: 5 });
    assert!(matches!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Until { .. })
    ));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        None,
        "an elapsed cooldown is not reported"
    );
    let mut permit = gate.acquire().await.unwrap();
    permit.throttle_without_hint();
    let unknown = recorder.snapshot().groups[&g("g")].cooldown;
    assert!(matches!(unknown, Some(CooldownState::Unknown { .. })));
    permit.feedback(ThrottleFeedback::Until(500));
    assert!(
        matches!(
            recorder.snapshot().groups[&g("g")].cooldown,
            Some(CooldownState::Until { .. })
        ),
        "dated advice replaces an open-ended throttle"
    );
    permit.throttle_without_hint();
    assert!(
        matches!(
            recorder.snapshot().groups[&g("g")].cooldown,
            Some(CooldownState::Until { .. })
        ),
        "no-hint advice keeps an unexpired dated cooldown"
    );
    permit.finish(Feedback::Throttle { delay_ms: 20_000 });
    assert_eq!(
        recorder.snapshot().groups[&g("g")].cooldown,
        Some(CooldownState::Unavailable),
        "completion advice beyond the maximum is unavailable, as at the authority"
    );
}

#[tokio::test]
async fn transitions_bump_the_revision_and_notify_the_hook() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let seen = Arc::new(Mutex::new(Vec::<AdmissionActivity>::new()));
    let sink = seen.clone();
    recorder.set_hook(Arc::new(move |activity: &AdmissionActivity| {
        sink.lock().unwrap().push(activity.clone());
    }));
    let gate = observed(Arc::new(Grant::new(0, 10_000)), "acct", "g", &recorder);
    let before = recorder.snapshot().revision;
    let permit = gate.acquire().await.unwrap();
    let admitted = recorder.snapshot().revision;
    assert!(admitted > before, "begin and grant advance the revision");
    permit.finish(Feedback::Success);
    let finished = recorder.snapshot().revision;
    assert!(finished > admitted);
    assert_eq!(
        recorder.snapshot().revision,
        finished,
        "reading does not advance it"
    );
    let seen = seen.lock().unwrap();
    let phases: Vec<(usize, usize, u64)> = seen
        .iter()
        .map(|a| (a.waiting, a.admitted, a.completed))
        .collect();
    assert_eq!(
        phases,
        vec![(1, 0, 0), (0, 1, 0), (0, 0, 1)],
        "the hook sees every transition in order with a fresh view"
    );
}

/// Transitions on many workers still reach the hook in revision order, so a
/// client that simply replaces its view never ends up holding a stale one.
/// A probabilistic detector (8 workers × 25 transitions, a yield inside the
/// hook): an unserialized delivery showed up on every run tried.
#[test]
fn concurrent_transitions_reach_the_hook_in_revision_order() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    recorder.set_hook(Arc::new(move |activity| {
        // Widen the window in which an unordered delivery would show.
        std::thread::yield_now();
        sink.lock().unwrap().push(activity.revision);
    }));
    let group = GroupId::new("g").unwrap();
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let recorder = recorder.clone();
            let group = group.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    let id = recorder.begin("acct", &group);
                    recorder.cancelled(id);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 400);
    assert!(
        seen.windows(2).all(|w| w[1] == w[0] + 1),
        "deliveries are in revision order"
    );
}

/// A snapshot's revision describes exactly its contents: the counters and the
/// revision move in one critical section, so a reader can never see new
/// contents under an old revision (review 2, L1).
#[test]
fn a_snapshot_revision_describes_exactly_its_contents() {
    let recorder = Arc::new(AdmissionRecorder::new());
    let group = GroupId::new("g").unwrap();
    let observer = recorder.clone();
    recorder.set_hook(Arc::new(move |view| {
        // Inside a transition the hook already sees the bumped revision
        // together with the new contents.
        let read = observer.snapshot();
        assert_eq!(read.revision, view.revision);
        assert_eq!(read.waiting, view.waiting);
    }));
    let id = recorder.begin("acct", &group);
    let queued = recorder.snapshot();
    assert_eq!((queued.revision, queued.waiting), (1, 1));
    recorder.cancelled(id);
    let cancelled = recorder.snapshot();
    assert_eq!((cancelled.revision, cancelled.cancelled), (2, 1));
    // No-op transitions bump nothing.
    recorder.cancelled(id);
    recorder.admitted(id);
    assert_eq!(recorder.snapshot().revision, 2);
}
