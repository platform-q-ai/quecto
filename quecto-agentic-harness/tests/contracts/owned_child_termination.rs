//! Contract for [`OwnedChildTermination`] (#1936), proven on the production
//! supervised adapter with real processes: a direct success never consults
//! a signal; an unreachable, refused or late acknowledgement authorises the
//! fallback only for a directly owned local handle; every row without a
//! retained handle concludes with nothing signalled.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::subagents::ports::{
    ConclusionBudget, OwnedChildTermination, ProtocolAttempt, TerminationConclusion,
};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{DelegatedAgentIdentity, LaunchGeneration};
use quecto::infrastructure::processes::owned_child_supervisor::{
    ChildHandleId, OwnedChildSupervisor, ProcessGroup, SentSignal, TerminationBudget,
};
use quecto::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};

const FAST: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::from_millis(300),
    term_grace: Duration::from_millis(300),
    kill_grace: Duration::from_secs(2),
};

async fn spawn(supervisor: &Arc<OwnedChildSupervisor>, argv: &[&str]) -> ChildHandleId {
    let mut command = tokio::process::Command::new(argv[0]);
    command
        .args(&argv[1..])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    supervisor
        .spawn(command, ProcessGroup::Inherited)
        .await
        .unwrap()
        .handle
}

fn registry_with(
    supervisor: &Arc<OwnedChildSupervisor>,
    handle: Option<ChildHandleId>,
    generation: u64,
) -> SubagentRegistry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new("A"),
        "alpha".into(),
        std::path::PathBuf::from("/tmp/none.sock"),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(generation));
    entry.owned_child = handle;
    entry.owned_child_supervisor = Some(Arc::clone(supervisor));
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    registry.lock().unwrap().insert("A".into(), entry);
    registry
}

fn a(generation: u64) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new("A", LaunchGeneration::new(generation))
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("the conclusion is bounded")
}

#[tokio::test]
async fn direct_success_uses_no_fallback() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    // A child that exits by itself shortly after "acknowledging".
    let handle = spawn(&supervisor, &["sh", "-c", "sleep 0.2"]).await;
    let port: Arc<dyn OwnedChildTermination> = Arc::new(
        SupervisedChildTermination::new(registry_with(&supervisor, Some(handle), 1))
            .with_budgets(TerminationBudget::DEFAULT, FAST),
    );
    let conclusion = bounded(port.conclude(
        &a(1),
        ProtocolAttempt::Acknowledged,
        ConclusionBudget::Standard,
    ))
    .await;
    assert_eq!(conclusion, TerminationConclusion::ExitedAfterProtocol);
    assert!(
        supervisor.signals_sent(handle).is_empty(),
        "no signal at all"
    );
    assert!(supervisor.fallback_authorised_by(handle).is_none());
    supervisor.retire_when_reaped(handle);
}

#[tokio::test]
async fn a_negative_attempt_falls_back_on_the_owned_handle_only() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = spawn(&supervisor, &["sleep", "30"]).await;
    let port = SupervisedChildTermination::new(registry_with(&supervisor, Some(handle), 1))
        .with_budgets(FAST, FAST);
    for (attempt, why) in [
        (
            ProtocolAttempt::Negative("malformed ack".into()),
            "malformed",
        ),
        (
            ProtocolAttempt::Negative("mismatched ack".into()),
            "mismatched",
        ),
    ] {
        // Only the first call has a live handle; the second finds it
        // reaped and reports that truthfully (never a second signal).
        let conclusion = bounded(port.conclude(&a(1), attempt, ConclusionBudget::Standard)).await;
        assert!(
            matches!(
                conclusion,
                TerminationConclusion::ExitedAfterFallback | TerminationConclusion::AlreadyExited
            ),
            "{why}: {conclusion:?}"
        );
    }
    assert_eq!(supervisor.signals_sent(handle), [SentSignal::Term]);
    assert_eq!(
        supervisor.fallback_authorised_by(handle).as_deref(),
        Some("malformed ack")
    );
    supervisor.retire_when_reaped(handle);
}

#[tokio::test]
async fn an_acknowledged_child_that_never_exits_is_signalled_after_its_budget() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = spawn(&supervisor, &["sleep", "30"]).await;
    let port = SupervisedChildTermination::new(registry_with(&supervisor, Some(handle), 1))
        .with_budgets(FAST, FAST);
    let started = std::time::Instant::now();
    let conclusion = bounded(port.conclude(
        &a(1),
        ProtocolAttempt::Acknowledged,
        ConclusionBudget::Rollback,
    ))
    .await;
    assert_eq!(conclusion, TerminationConclusion::ExitedAfterFallback);
    assert!(
        started.elapsed() >= FAST.exit_after_ack,
        "the ACK bought the child its exit budget first"
    );
    assert!(
        supervisor
            .fallback_authorised_by(handle)
            .is_some_and(|why| why.contains("acknowledged but not exited"))
    );
    supervisor.retire_when_reaped(handle);
}

#[tokio::test]
async fn nothing_without_a_retained_handle_is_ever_signalled() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = spawn(&supervisor, &["sleep", "30"]).await;
    // Wrong generation, no handle, unknown uuid: three refusals.
    let stale = SupervisedChildTermination::new(registry_with(&supervisor, Some(handle), 2));
    let unowned = SupervisedChildTermination::new(registry_with(&supervisor, None, 1));
    for (port, id, why) in [
        (&stale, a(1), "stale generation"),
        (&unowned, a(1), "no handle (remote member)"),
        (
            &unowned,
            DelegatedAgentIdentity::new("ghost", LaunchGeneration::new(1)),
            "unknown",
        ),
    ] {
        assert_eq!(
            bounded(port.conclude(
                &id,
                ProtocolAttempt::Negative("gone".into()),
                ConclusionBudget::Standard
            ))
            .await,
            TerminationConclusion::NoRetainedHandle,
            "{why}"
        );
    }
    assert!(supervisor.signals_sent(handle).is_empty());
    assert!(supervisor.retains(handle));
    let _ = supervisor
        .terminate(
            handle,
            async {
                quecto::infrastructure::processes::owned_child_supervisor::ProtocolOutcome::Negative(
                    "test cleanup".into(),
                )
            },
            FAST,
        )
        .await;
    supervisor.retire_when_reaped(handle);
}
