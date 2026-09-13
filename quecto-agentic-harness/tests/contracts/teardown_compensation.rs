//! Contract for [`TeardownCompensation`] (#1936), proven on the production
//! registry adapter: a claimed row's terminal effects run once — the row
//! and its reported subtree leave live membership, exactly one survivor
//! broadcast goes out, exit signals carry the truthful kind, a natural exit
//! posts one passive note, an operator kill posts none — and the exit
//! observation use case makes concurrent observers join rather than repeat.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::subagents::dto::{ObserveOwnedChildExitRequest, ObservedExit};
use quecto::application::subagents::ports::{
    CompensationObservation, DelegatedAgentRegistry, ExitObservation, TeardownCompensation,
    TerminalClaim, TerminationCause,
};
use quecto::application::subagents::use_cases::ObserveOwnedChildExit;
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{DelegatedAgentIdentity, LaunchGeneration};
use quecto::infrastructure::tools::subagent_registry::{
    ExitSignalKind, SubagentEntry, SubagentNotification, SubagentRegistry, SubagentStatus,
    TeardownPhase, new_exit_signal_channel, new_notification_channel,
};
use quecto::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;

fn identity(uuid: &str) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new(uuid, LaunchGeneration::new(1))
}

fn row(uuid: &str, parent: Option<&str>, launched: bool) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(uuid),
        uuid.to_owned(),
        std::path::PathBuf::from(format!("/tmp/{uuid}.sock")),
        0,
    );
    if launched {
        entry.launch_generation = Some(LaunchGeneration::new(1));
    } else {
        entry.reported_generation = Some(LaunchGeneration::new(1));
    }
    entry.parent_id = parent.map(str::to_string);
    entry
}

/// A → B → C, plus an unrelated D.
fn tree() -> SubagentRegistry {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    {
        let mut entries = registry.lock().unwrap();
        entries.insert("A".into(), row("A", None, true));
        entries.insert("B".into(), row("B", Some("A"), false));
        entries.insert("C".into(), row("C", Some("B"), false));
        entries.insert("D".into(), row("D", None, true));
    }
    registry
}

#[tokio::test]
async fn a_natural_exit_compensates_the_subtree_once_with_one_broadcast_and_one_note() {
    let registry = tree();
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
    let (notify_tx, mut notify_rx) = new_notification_channel();
    let (c_exit_tx, _c_exit_rx) = new_exit_signal_channel();
    registry
        .lock()
        .unwrap()
        .get_mut("C")
        .unwrap()
        .exit_signal_tx = Some(c_exit_tx.clone());
    let adapter = Arc::new(RegistryDelegatedAgents::new(
        registry.clone(),
        Some(broadcast_tx),
        Some(notify_tx),
    ));
    let compensation: Arc<dyn TeardownCompensation> = adapter.clone();
    let claims: Arc<dyn DelegatedAgentRegistry> = adapter;
    assert_eq!(
        claims.claim_terminal(&identity("A")),
        TerminalClaim::Claimed
    );
    let compensated = compensation
        .compensate(
            &identity("A"),
            TerminationCause::Exit(ExitObservation::ProcessExited),
        )
        .await;
    assert_eq!(compensated.removed[0], AgentUuid::new("A"));
    assert_eq!(compensated.removed.len(), 3, "A, B and C");
    let event = broadcast_rx.try_recv().expect("one survivor broadcast");
    assert!(event.contains("\"agentUuid\":\"D\""));
    assert!(!event.contains("\"agentUuid\":\"A\""));
    assert!(!event.contains("\"agentUuid\":\"C\""));
    assert!(broadcast_rx.try_recv().is_err(), "exactly one");
    let note = notify_rx.try_recv().expect("one passive note");
    assert_eq!(
        note.notification,
        SubagentNotification::Exited {
            agent_id: "A".into(),
            reason: Some("process_exit".into()),
        }
    );
    assert!(notify_rx.try_recv().is_err());
    assert_eq!(
        c_exit_tx
            .borrow()
            .as_ref()
            .map(|s| (s.kind, s.signal, s.exit_code)),
        Some((ExitSignalKind::Terminated, None, None)),
        "a descendant carries no fabricated status"
    );
    let entries = registry.lock().unwrap();
    for key in ["A", "B", "C"] {
        assert_eq!(entries[key].status, SubagentStatus::Exited);
        assert_eq!(entries[key].teardown_phase(), TeardownPhase::Compensated);
    }
    assert_eq!(entries["D"].status, SubagentStatus::Starting);
    assert_eq!(entries["D"].teardown_phase(), TeardownPhase::Live);
}

#[tokio::test]
async fn a_selected_termination_posts_no_note_and_a_launch_rollback_neither() {
    for cause in [
        TerminationCause::SelectedTermination,
        TerminationCause::LaunchRollback {
            owns_environment: false,
        },
    ] {
        let registry = tree();
        let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
        let (notify_tx, mut notify_rx) = new_notification_channel();
        let (d_exit_tx, _d_exit_rx) = new_exit_signal_channel();
        registry
            .lock()
            .unwrap()
            .get_mut("D")
            .unwrap()
            .exit_signal_tx = Some(d_exit_tx.clone());
        let compensation: Arc<dyn TeardownCompensation> = Arc::new(RegistryDelegatedAgents::new(
            registry.clone(),
            Some(broadcast_tx),
            Some(notify_tx),
        ));
        let compensated = compensation.compensate(&identity("D"), cause).await;
        assert_eq!(compensated.removed, [AgentUuid::new("D")]);
        assert!(broadcast_rx.try_recv().is_ok());
        assert!(notify_rx.try_recv().is_err(), "{cause:?}: no passive note");
        assert_eq!(
            d_exit_tx.borrow().as_ref().map(|s| s.kind),
            Some(ExitSignalKind::Terminated)
        );
    }
}

/// Reaper, monitor EOF and a concurrent kill all observe the same end: the
/// first claims, the rest join, and the effects run once.
#[tokio::test]
async fn concurrent_observations_of_one_exit_compensate_exactly_once() {
    let registry = tree();
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(16);
    let adapter = Arc::new(
        RegistryDelegatedAgents::new(registry.clone(), Some(broadcast_tx), None)
            .with_compensation_wait(Duration::from_secs(5)),
    );
    let observer = Arc::new(ObserveOwnedChildExit::new(adapter.clone(), adapter.clone()));
    let mut tasks = Vec::new();
    for observation in [
        ExitObservation::ProcessExited,
        ExitObservation::ConnectionClosed,
        ExitObservation::ProcessExited,
    ] {
        let observer = observer.clone();
        tasks.push(tokio::spawn(async move {
            observer
                .execute(ObserveOwnedChildExitRequest {
                    child: identity("A"),
                    observation,
                })
                .await
        }));
    }
    let mut compensated = 0;
    let mut joined = 0;
    for task in tasks {
        match tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("bounded")
            .unwrap()
        {
            ObservedExit::Compensated { removed } => {
                compensated += 1;
                assert_eq!(removed.len(), 3);
            }
            ObservedExit::Joined(observation) => {
                joined += 1;
                assert_eq!(observation, CompensationObservation::Compensated);
            }
            ObservedExit::DeferredToProcessExit => panic!("no process is retained here"),
        }
    }
    assert_eq!((compensated, joined), (1, 2));
    assert!(broadcast_rx.try_recv().is_ok());
    assert!(
        broadcast_rx.try_recv().is_err(),
        "one broadcast for three observers"
    );
}
