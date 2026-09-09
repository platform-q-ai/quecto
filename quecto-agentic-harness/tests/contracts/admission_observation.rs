//! AdmissionObservation contract (#1679 P4): every gate transition is visible
//! as an orthogonal admission state with group, reason, elapsed wait and a
//! bounded, fresh aggregate. Proven over the real remote gate and authority.
use quecto::application::ports::{AdmissionObservation, AttemptAdmission};
use quecto::domain::inference_admission::{
    AdmissionActivity, AdmissionConfig, AdmissionPhase, CooldownState, Feedback, GroupId,
    GroupPolicy, ThrottleFeedback, WorkloadClass,
};
use quecto::infrastructure::admission::{
    AdminConnection, AdmissionRecorder, AuthorityConnection, AuthorityDirectory, AuthorityServer,
    ObservedAdmission,
};
use quecto::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(5);

fn proposal() -> AdmissionRuntimeProposal {
    let g = GroupId::new("g").unwrap();
    let h = GroupId::new("h").unwrap();
    let policy = |capacity| GroupPolicy {
        capacity,
        reserve: 0,
        min_interval_ms: 1,
        queue_capacity: 4,
        queue_timeout_ms: 400,
        attempt_timeout_ms: 60_000,
        fallback_base_ms: 100,
        max_cooldown_ms: 10_000,
    };
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(g.clone(), policy(1)), (h.clone(), policy(1))]),
            aliases: BTreeMap::from([("acct".into(), g), ("other".into(), h)]),
            max_scopes: 8,
            terminal_capacity: 16,
        },
        bindings: BTreeMap::from([
            ("openai".into(), "acct".into()),
            ("anthropic".into(), "other".into()),
        ]),
    }
}

fn group(name: &str) -> GroupId {
    GroupId::new(name).unwrap()
}

async fn root(server: &AuthorityServer) -> AuthorityConnection {
    let c = AuthorityConnection::connect(&server.directory().client_socket())
        .await
        .unwrap();
    let credential = c.register_root(WorkloadClass::Interactive).await.unwrap();
    c.bind(credential).await.unwrap();
    c
}

/// Poll the recorder until `expected` holds (bounded), returning that view.
async fn view_when(
    recorder: &AdmissionRecorder,
    what: &str,
    expected: impl Fn(&AdmissionActivity) -> bool,
) -> AdmissionActivity {
    let deadline = tokio::time::Instant::now() + LIMIT;
    loop {
        let view = recorder.snapshot();
        if expected(&view) {
            return view;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never observed {what}: {view:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn view_when_authority(
    admin: &AdminConnection,
    what: &str,
    expected: impl Fn(&quecto::application::ports::AuthorityStatus) -> bool,
) -> quecto::application::ports::AuthorityStatus {
    let deadline = tokio::time::Instant::now() + LIMIT;
    loop {
        let status = admin.inspect().await.unwrap();
        if expected(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never observed {what}: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
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
        group("g"),
        recorder.clone(),
    ));
    let idle = recorder.snapshot();
    assert_eq!((idle.waiting, idle.admitted, idle.hidden), (0, 0, 0));
    assert!(
        idle.attempts.is_empty(),
        "nothing to observe before an attempt"
    );

    let waiting_gate = gate.clone();
    let pending = tokio::spawn(async move { waiting_gate.acquire().await });
    let first = view_when(&recorder, "a waiting attempt", |v| v.waiting == 1).await;
    let attempt = first.attempts.values().next().unwrap();
    assert_eq!(attempt.alias, "acct");
    assert_eq!(attempt.group, group("g"));
    assert!(matches!(attempt.phase, AdmissionPhase::Waiting { .. }));
    tokio::time::sleep(Duration::from_millis(20)).await;
    let later = recorder.snapshot();
    assert!(
        later.attempts.values().next().unwrap().elapsed_ms > attempt.elapsed_ms,
        "elapsed wait keeps growing"
    );
    assert!(
        later.observed_at_ms > first.observed_at_ms,
        "freshness advances"
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
    let AdmissionPhase::Admitted { since_ms } = admitted.attempts.values().next().unwrap().phase
    else {
        panic!("admitted")
    };
    permit.feedback(ThrottleFeedback::Until(receipt_ms + 3_000));
    let cooling = recorder.snapshot();
    // Anchored exactly at the grant on the process clock, for group g only.
    assert_eq!(
        cooling.groups[&group("g")].cooldown,
        Some(CooldownState::Until {
            until_ms: since_ms + 3_000
        }),
        "{cooling:?}"
    );
    assert_eq!(
        cooling.groups.get(&group("h")).and_then(|g| g.cooldown),
        None
    );
    // The advice reached the authority through the wrapped permit.
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    let authority = view_when_authority(&admin, "authority cooldown", |s| {
        s.groups[&group("g")].cooldown_until >= receipt_ms + 3_000
    })
    .await;
    assert_eq!(authority.groups[&group("g")].active, 1);
    permit.finish(Feedback::Throttle { delay_ms: 3_000 });
    let released = recorder.snapshot();
    assert_eq!((released.waiting, released.admitted), (0, 0));
    assert!(
        released.attempts.is_empty(),
        "released attempts leave the live view"
    );
    assert_eq!(released.completed, 1);
    // The release reached the authority: the slot is free (the group is in
    // the 3 s cooldown the completion advised, so no new grant is expected).
    let after = view_when_authority(&admin, "slot released at the authority", |s| {
        s.groups[&group("g")].active == 0
    })
    .await;
    assert!(after.groups[&group("g")].cooldown_until > 0);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusals_and_cancellations_are_observed_per_group_with_reasons() {
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
        group("g"),
        recorder.clone(),
    );
    let free = ObservedAdmission::new(
        observer.gate("other").unwrap(),
        "other",
        group("h"),
        recorder.clone(),
    );
    // Queue deadline (400 ms) elapses: an explicit refusal, never a bypass.
    assert!(gate.acquire().await.is_err());
    let after = recorder.snapshot();
    assert_eq!((after.waiting, after.admitted, after.refused), (0, 0, 1));
    assert!(
        after.groups[&group("g")]
            .last_refusal
            .as_deref()
            .is_some_and(|r| r.contains("deadline")),
        "refusal reason names the cause: {after:?}"
    );
    // A dropped wait is a cancellation, counted and never left live.
    let dropped = tokio::time::timeout(Duration::from_millis(30), gate.acquire()).await;
    assert!(dropped.is_err());
    let cancelled = recorder.snapshot();
    assert_eq!((cancelled.waiting, cancelled.cancelled), (0, 1));
    // The other group is independent and untouched by g's refusal.
    let permit = free.acquire().await.expect("group h has capacity");
    let both = recorder.snapshot();
    assert_eq!(both.admitted, 1);
    assert_eq!(both.groups[&group("h")].last_refusal, None);
    permit.finish(Feedback::Success);
    held.finish(Feedback::Success);
    server.shutdown().await;
}
