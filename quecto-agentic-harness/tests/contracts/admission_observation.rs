//! AdmissionObservation contract (#1679 P4): every gate transition is visible
//! as an orthogonal admission state with group, reason, elapsed wait and a
//! bounded, fresh aggregate. Proven over the real remote gate and authority.
use quecto::application::ports::{AdmissionObservation, AttemptAdmission};
use quecto::domain::inference_admission::{
    AdmissionConfig, AdmissionPhase, Feedback, GroupId, GroupPolicy, ThrottleFeedback,
    WorkloadClass,
};
use quecto::infrastructure::admission::{
    AdmissionRecorder, AuthorityConnection, AuthorityDirectory, AuthorityServer, ObservedAdmission,
};
use quecto::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

fn proposal() -> AdmissionRuntimeProposal {
    let g = GroupId::new("g").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                g.clone(),
                GroupPolicy {
                    capacity: 1,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 4,
                    queue_timeout_ms: 400,
                    attempt_timeout_ms: 60_000,
                    fallback_base_ms: 100,
                    max_cooldown_ms: 10_000,
                },
            )]),
            aliases: BTreeMap::from([("acct".into(), g)]),
            max_scopes: 8,
            terminal_capacity: 16,
        },
        bindings: BTreeMap::from([("openai".into(), "acct".into())]),
    }
}

async fn root(server: &AuthorityServer) -> AuthorityConnection {
    let c = AuthorityConnection::connect(&server.directory().client_socket())
        .await
        .unwrap();
    let credential = c.register_root(WorkloadClass::Interactive).await.unwrap();
    c.bind(credential).await.unwrap();
    c
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waiting_admitted_released_and_cooldown_are_observable_with_freshness() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(),
    )
    .await
    .unwrap();
    let holder = root(&server).await;
    let held = holder.gate("acct").unwrap().acquire().await.unwrap();
    let observer = root(&server).await;
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = Arc::new(ObservedAdmission::new(
        observer.gate("acct").unwrap(),
        "acct",
        GroupId::new("g").unwrap(),
        recorder.clone(),
    ));
    let idle = recorder.snapshot();
    assert_eq!((idle.waiting, idle.admitted), (0, 0));
    assert!(
        idle.attempts.is_empty(),
        "nothing to observe before an attempt"
    );

    let waiting_gate = gate.clone();
    let pending = tokio::spawn(async move { waiting_gate.acquire().await });
    tokio::time::sleep(Duration::from_millis(60)).await;
    let waiting = recorder.snapshot();
    assert_eq!(waiting.waiting, 1, "queued attempt counted as waiting");
    let attempt = waiting.attempts.values().next().unwrap();
    assert_eq!(attempt.alias, "acct");
    assert_eq!(attempt.group, GroupId::new("g").unwrap());
    assert!(matches!(attempt.phase, AdmissionPhase::Waiting { .. }));
    assert!(
        attempt.elapsed_ms >= 50,
        "elapsed wait is reported: {attempt:?}"
    );
    assert!(
        waiting.observed_at_ms >= attempt.elapsed_ms,
        "freshness is monotonic"
    );

    held.finish(Feedback::Success);
    let mut permit = pending.await.unwrap().expect("granted after release");
    let admitted = recorder.snapshot();
    assert_eq!((admitted.waiting, admitted.admitted), (0, 1));
    assert!(matches!(
        admitted.attempts.values().next().unwrap().phase,
        AdmissionPhase::Admitted { .. }
    ));

    let (receipt_ms, _) = permit.receipt_clock();
    permit.feedback(ThrottleFeedback::Until(receipt_ms + 3_000));
    let cooling = recorder.snapshot();
    // The authority's deadline is translated onto the process clock through
    // the permit receipt, so it reads as roughly three seconds ahead of now.
    assert!(
        matches!(cooling.cooldown_until_ms, Some(until) if until >= cooling.observed_at_ms + 2_900),
        "group cooldown is visible on the process clock: {cooling:?}"
    );
    let _ = receipt_ms;
    permit.finish(Feedback::Throttle { delay_ms: 3_000 });
    let released = recorder.snapshot();
    assert_eq!((released.waiting, released.admitted), (0, 0));
    assert!(
        released.attempts.is_empty(),
        "released attempts leave the live view"
    );
    assert_eq!(released.completed, 1);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusals_are_observed_with_their_reason_and_the_view_stays_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(),
    )
    .await
    .unwrap();
    let holder = root(&server).await;
    let held = holder.gate("acct").unwrap().acquire().await.unwrap();
    let observer = root(&server).await;
    let recorder = Arc::new(AdmissionRecorder::new());
    let gate = ObservedAdmission::new(
        observer.gate("acct").unwrap(),
        "acct",
        GroupId::new("g").unwrap(),
        recorder.clone(),
    );
    // Queue deadline (400 ms) elapses: an explicit refusal, never a bypass.
    let refused = gate.acquire().await;
    assert!(refused.is_err());
    let after = recorder.snapshot();
    assert_eq!((after.waiting, after.admitted), (0, 0));
    assert_eq!(after.refused, 1);
    assert!(
        after
            .last_refusal
            .as_deref()
            .is_some_and(|r| r.contains("deadline")),
        "refusal reason names the cause: {after:?}"
    );
    // A dropped wait is a cancellation, also counted and never left live.
    let dropped = tokio::time::timeout(Duration::from_millis(30), gate.acquire()).await;
    assert!(dropped.is_err());
    tokio::time::sleep(Duration::from_millis(20)).await;
    let cancelled = recorder.snapshot();
    assert_eq!(cancelled.waiting, 0, "a dropped wait is not a live attempt");
    assert_eq!(cancelled.cancelled, 1);
    // The live view is bounded: at most 64 attempts are retained per process.
    assert!(cancelled.attempts.len() <= 64);
    held.finish(Feedback::Success);
    server.shutdown().await;
}
