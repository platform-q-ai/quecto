use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::application::subagents::ports::ConclusionBudget;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::infrastructure::processes::owned_child_supervisor::{ProcessGroup, SentSignal};
use crate::infrastructure::tools::subagent_registry::SubagentEntry;

const FAST: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::from_millis(300),
    term_grace: Duration::from_millis(300),
    kill_grace: Duration::from_secs(2),
};

async fn sleeper(supervisor: &Arc<OwnedChildSupervisor>) -> ChildHandleId {
    let mut command = tokio::process::Command::new("sleep");
    command
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    supervisor
        .spawn(command, ProcessGroup::Inherited)
        .await
        .expect("spawn sleep")
        .handle
}

fn owned_row(
    supervisor: &Arc<OwnedChildSupervisor>,
    handle: ChildHandleId,
    generation: u64,
) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new("A"),
        "alpha".into(),
        std::path::PathBuf::from("/tmp/none.sock"),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(generation));
    entry.owned_child = Some(handle);
    entry.owned_child_supervisor = Some(Arc::clone(supervisor));
    entry
}

fn registry(entry: SubagentEntry) -> SubagentRegistry {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    registry.lock().unwrap().insert("A".into(), entry);
    registry
}

fn identity(generation: u64) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new("A", LaunchGeneration::new(generation))
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("conclusion within the bound")
}

#[tokio::test]
async fn an_acknowledged_child_that_does_not_exit_falls_back_after_its_budget() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = sleeper(&supervisor).await;
    let port = SupervisedChildTermination::new(registry(owned_row(&supervisor, handle, 1)))
        .with_budgets(FAST, FAST);
    // `sleep 30` will not exit on its own within the fast budget: the ACK
    // alone authorises nothing, the exit timeout authorises TERM.
    let conclusion = bounded(port.conclude(
        &identity(1),
        ProtocolAttempt::Acknowledged,
        ConclusionBudget::Standard,
    ))
    .await;
    assert_eq!(conclusion, TerminationConclusion::ExitedAfterFallback);
    assert_eq!(supervisor.signals_sent(handle), [SentSignal::Term]);
    assert!(
        supervisor
            .fallback_authorised_by(handle)
            .is_some_and(|why| why.contains("acknowledged but not exited")),
        "the fallback was authorised by the exit timeout, never by the ACK"
    );
    supervisor.retire_when_reaped(handle);
}

#[tokio::test]
async fn a_negative_attempt_authorises_the_fallback_at_once() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = sleeper(&supervisor).await;
    let port = SupervisedChildTermination::new(registry(owned_row(&supervisor, handle, 3)))
        .with_budgets(FAST, FAST);
    let conclusion = bounded(port.conclude(
        &identity(3),
        ProtocolAttempt::Negative("connection refused".into()),
        ConclusionBudget::Rollback,
    ))
    .await;
    assert_eq!(conclusion, TerminationConclusion::ExitedAfterFallback);
    assert_eq!(
        supervisor.fallback_authorised_by(handle).as_deref(),
        Some("connection refused")
    );
    supervisor.retire_when_reaped(handle);
}

#[tokio::test]
async fn rows_without_a_retained_handle_conclude_without_a_signal() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = sleeper(&supervisor).await;
    // Wrong generation: the row is not the child this identity names.
    let port = SupervisedChildTermination::new(registry(owned_row(&supervisor, handle, 2)));
    for (why, id) in [("stale generation", identity(1)), ("future", identity(3))] {
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
    // A merged row: uuid known, no handle at all.
    let mut merged = SubagentEntry::with_identity(
        AgentUuid::new("B"),
        "bravo".into(),
        std::path::PathBuf::new(),
        4242,
    );
    merged.reported_generation = Some(LaunchGeneration::new(1));
    let port = SupervisedChildTermination::new(registry(merged));
    assert_eq!(
        bounded(port.conclude(
            &DelegatedAgentIdentity::new("B", LaunchGeneration::new(1)),
            ProtocolAttempt::Negative("gone".into()),
            ConclusionBudget::Standard
        ))
        .await,
        TerminationConclusion::NoRetainedHandle
    );
    // An unknown uuid.
    assert_eq!(
        bounded(port.conclude(
            &DelegatedAgentIdentity::new("ghost", LaunchGeneration::new(1)),
            ProtocolAttempt::Acknowledged,
            ConclusionBudget::Standard
        ))
        .await,
        TerminationConclusion::NoRetainedHandle
    );
    assert!(
        supervisor.signals_sent(handle).is_empty(),
        "nothing signalled"
    );
    assert!(supervisor.retains(handle));
    // End the sleeper through its owner so the test leaves nothing behind.
    let _ = supervisor
        .terminate(
            handle,
            async { ProtocolOutcome::Negative("test cleanup".into()) },
            FAST,
        )
        .await;
    supervisor.retire_when_reaped(handle);
}

/// A child that was alive to acknowledge and is reaped before the
/// supervisor looks exited by the protocol; one that was gone before a
/// negative attempt had already exited.
#[tokio::test]
async fn an_already_reaped_child_is_graceful_after_an_ack_and_already_exited_otherwise() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let mut command = tokio::process::Command::new("true");
    command.stdout(std::process::Stdio::null());
    let handle = supervisor
        .spawn(command, ProcessGroup::Inherited)
        .await
        .unwrap()
        .handle;
    bounded(supervisor.wait_exit(handle)).await;
    let port = SupervisedChildTermination::new(registry(owned_row(&supervisor, handle, 1)));
    assert_eq!(
        bounded(port.conclude(
            &identity(1),
            ProtocolAttempt::Acknowledged,
            ConclusionBudget::Standard
        ))
        .await,
        TerminationConclusion::ExitedAfterProtocol
    );
    assert_eq!(
        bounded(port.conclude(
            &identity(1),
            ProtocolAttempt::Negative("gone".into()),
            ConclusionBudget::Standard
        ))
        .await,
        TerminationConclusion::AlreadyExited
    );
    assert!(supervisor.signals_sent(handle).is_empty());
}

#[test]
fn every_supervisor_outcome_maps_to_one_conclusion() {
    use crate::infrastructure::processes::owned_child_supervisor::ChildExit;
    let exit = ChildExit::Code(0);
    assert_eq!(
        conclusion_of(TerminationOutcome::NoRetainedHandle),
        TerminationConclusion::NoRetainedHandle
    );
    assert_eq!(
        conclusion_of(TerminationOutcome::AlreadyExited(exit.clone())),
        TerminationConclusion::AlreadyExited
    );
    assert_eq!(
        conclusion_of(TerminationOutcome::ExitedAfterProtocol(exit.clone())),
        TerminationConclusion::ExitedAfterProtocol
    );
    assert_eq!(
        conclusion_of(TerminationOutcome::ExitedAfterTerm {
            negative: "n".into(),
            exit: exit.clone()
        }),
        TerminationConclusion::ExitedAfterFallback
    );
    assert_eq!(
        conclusion_of(TerminationOutcome::ExitedAfterKill {
            negative: "n".into(),
            exit
        }),
        TerminationConclusion::ExitedAfterFallback
    );
    assert!(matches!(
        conclusion_of(TerminationOutcome::StillRunning {
            negative: "n".into()
        }),
        TerminationConclusion::StillRunning(detail) if detail.contains("'n'")
    ));
}
