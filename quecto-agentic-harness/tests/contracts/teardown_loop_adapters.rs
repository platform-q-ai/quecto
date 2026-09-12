//! Contract for the composed teardown graph of one harness (#1935): the
//! loop-bound adapters (turn cancellation over the cancel slot, exit
//! readiness over the loop's `Notify`, deferred persistence, registry-backed
//! lineage) behind the slice A use cases, driven through the controller the
//! connection layer uses. Built only from the public crate surface.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::subagents::ports::ExitReadiness;
use quecto::composition::subagent_teardown::{TeardownGraphInputs, build_teardown_graph};
use quecto::domain::ids::AgentUuid;
use quecto::domain::parent_control::{
    BindingState, ParentControlBinding, ParentControlCapability, ParentControlCredential,
};
use quecto::domain::subagent_teardown::{HarnessLifecycleState, LaunchGeneration, ShutdownReason};
use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};
use quecto::interface::cli::uds_cancel::{CancelSlot, TurnControl};
use quecto::interface::uds::subagent_teardown::controller::{ControllerOutcome, DeliveryState};

struct Graph {
    graph: quecto::composition::subagent_teardown::TeardownGraph,
    notify: Arc<tokio::sync::Notify>,
    cancel: Arc<Mutex<CancelSlot>>,
    turn_control: Arc<TurnControl>,
}

fn credential() -> ParentControlCredential {
    ParentControlCredential {
        generation: LaunchGeneration::new(1),
        capability: ParentControlCapability::from_random_bytes(&[7; 32]),
    }
}

fn graph(registry: Option<SubagentRegistry>, busy: bool) -> Graph {
    let notify = Arc::new(tokio::sync::Notify::new());
    let cancel: Arc<Mutex<CancelSlot>> = Arc::new(Mutex::new(CancelSlot::Idle));
    let turn_control: Arc<TurnControl> = Arc::new(TurnControl::default());
    let graph = build_teardown_graph(TeardownGraphInputs {
        owner: AgentUuid::new("me"),
        registry,
        cancel_handle: cancel.clone(),
        turn_control: turn_control.clone(),
        busy: Arc::new(std::sync::atomic::AtomicBool::new(busy)),
        exit_notify: notify.clone(),
        binding: ParentControlBinding::launched(credential()),
    });
    Graph {
        graph,
        notify,
        cancel,
        turn_control,
    }
}

#[tokio::test]
async fn parent_loss_runs_the_common_shutdown_through_the_loop_adapters() {
    let g = graph(None, true);
    let outcome = g
        .graph
        .connections
        .controller
        .parent_connection_lost(DeliveryState::Busy)
        .await;
    let ControllerOutcome::ShutdownExecuted { delivery, outcome } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(delivery, DeliveryState::Busy);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.reason, ShutdownReason::ParentConnectionLost);
    assert!(
        outcome.turn_cancelled,
        "the busy flag reports an interrupted turn"
    );
    assert!(outcome.exit_signalled);
    // Turn cancellation: abort intent recorded, cancel slot fired.
    assert!(g.turn_control.is_abort_pending());
    assert!(matches!(*g.cancel.lock().unwrap(), CancelSlot::Fired));
    // Exit readiness: the loop's notify carries a permit and the readiness
    // is recorded for the loop to read.
    tokio::time::timeout(Duration::from_secs(1), g.notify.notified())
        .await
        .expect("the dispatch loop is woken");
    assert_eq!(
        g.graph.exit.signalled(),
        Some(ExitReadiness::Completed(
            ShutdownReason::ParentConnectionLost
        ))
    );
    // Persistence is deferred to the loop's exit path, for this reason.
    assert_eq!(
        g.graph.persistence.recorded_reason(),
        Some(ShutdownReason::ParentConnectionLost)
    );
    assert!(!g.graph.transaction.accepts_new_work());
}

#[tokio::test]
async fn lineage_comes_from_the_registry_and_only_launched_rows_are_children() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    {
        let mut entries = registry.lock().unwrap();
        let mut launched = SubagentEntry::with_identity(
            AgentUuid::new("kid"),
            "kid".into(),
            std::env::temp_dir().join("q-ct-no-such-child.sock"),
            0,
        );
        launched.launch_generation = Some(LaunchGeneration::new(3));
        entries.insert("kid".into(), launched);
        entries.insert(
            "restored".into(),
            SubagentEntry::new("/tmp/restored.sock".into(), 99),
        );
    }
    let g = graph(Some(registry), false);
    let outcome = g
        .graph
        .connections
        .controller
        .parent_connection_lost(DeliveryState::Idle)
        .await;
    let ControllerOutcome::ShutdownExecuted { outcome, .. } = outcome else {
        panic!("{outcome:?}");
    };
    let outcome = outcome.unwrap();
    // The launched child was addressed (its endpoint is dead, so it is
    // recorded as failed rather than ignored); the restored row never was.
    assert!(outcome.children_shut_down.is_empty());
    assert_eq!(outcome.children_failed.len(), 1);
    assert_eq!(outcome.children_failed[0].0, AgentUuid::new("kid"));
    assert!(!outcome.turn_cancelled);
}

#[tokio::test]
async fn the_binding_and_lifecycle_are_the_graphs_single_source_of_truth() {
    let g = graph(None, false);
    assert_eq!(g.graph.connections.binding_state(), BindingState::Unbound);
    assert!(g.graph.transaction.accepts_new_work());
    g.graph
        .connections
        .binding
        .lock()
        .unwrap()
        .present(LaunchGeneration::new(1), &credential().capability)
        .unwrap();
    assert_eq!(g.graph.connections.binding_state(), BindingState::Bound);
    let outcome = g
        .graph
        .connections
        .controller
        .parent_connection_lost(DeliveryState::Idle)
        .await;
    let ControllerOutcome::ShutdownExecuted { outcome: first, .. } = outcome else {
        panic!("{outcome:?}");
    };
    // A repeated loss report joins the completed admission: the same
    // outcome comes back and nothing runs twice.
    let again = g
        .graph
        .connections
        .controller
        .parent_connection_lost(DeliveryState::Idle)
        .await;
    let ControllerOutcome::ShutdownExecuted {
        outcome: second, ..
    } = again
    else {
        panic!("{again:?}");
    };
    assert_eq!(first, second);
    assert_eq!(
        g.graph.exit.signalled(),
        Some(ExitReadiness::Completed(
            ShutdownReason::ParentConnectionLost
        ))
    );
    let _ = HarnessLifecycleState::Terminated;
}
