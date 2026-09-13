//! The concrete graph of the subagent teardown capability for one harness
//! (#1935, #1938): the slice A use cases wired to the loop-bound adapters,
//! the fleet teardown over the registry-backed claim adapters, the UDS
//! routing adapter to direct children, the supervised owned-handle fallback,
//! and the launch-bound parent binding the connection layer consults. The
//! interface receives this builder through its `CliContext` (`run_composed`)
//! and never names this module.
use std::sync::{Arc, Mutex};

use crate::application::subagents::use_cases::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    OwnerConclusionPorts, PrepareHarnessShutdown, TerminateAllDelegatedAgents,
    TerminateAllDelegatedAgentsPorts, TerminateDelegatedAgent,
};
use crate::domain::ids::AgentUuid;
use crate::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use crate::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use crate::infrastructure::tools::harness_lifecycle::{
    SharedHarnessLifecycle, new_shared_harness_lifecycle,
};
use crate::infrastructure::tools::subagent_registry::{NotificationTx, SubagentRegistry};
use crate::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;
use crate::interface::cli::uds_teardown_adapters::{
    DeferredLoopPersistence, LoopExitReadiness, LoopTurnCancellation, MonotonicShutdownClock,
    RegistryLifecycleRepository, TokioShutdownRunSpawner,
};
use crate::interface::uds::subagent_teardown::controller::SubagentTeardownController;

pub use crate::interface::cli::uds_parent_control::{ConnectionRole, ConnectionTeardown};
pub use crate::interface::cli::uds_teardown_adapters::{
    DeferredLoopPersistence as LoopPersistenceAdapter, HOLD_EXIT_AFTER_ACK_ENV,
    LoopExitReadiness as LoopExitAdapter, RegistryLifecycleRepository as LifecycleAdapter,
};
pub use crate::interface::cli::uds_teardown_graph::{
    ParentControlLaunch, TeardownGraph, TeardownGraphBuilder, TeardownGraphInputs,
};

/// What the fleet teardown of one harness is built over.
pub struct FleetTeardownWiring {
    pub owner: AgentUuid,
    pub registry: SubagentRegistry,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub notify_tx: Option<NotificationTx>,
    pub harness_lifecycle: SharedHarnessLifecycle,
}

/// The fleet teardown (#1938) over the production adapters: registry-backed
/// claims and compensation, one-edge UDS routing, the supervised fallback.
pub fn build_fleet_teardown(wiring: FleetTeardownWiring) -> Arc<TerminateAllDelegatedAgents> {
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        Some(wiring.registry.clone()),
        wiring.owner,
        wiring.harness_lifecycle,
    ));
    build_fleet_over(
        lifecycle,
        wiring.registry,
        wiring.broadcast_tx,
        wiring.notify_tx,
    )
}

fn build_fleet_over(
    lifecycle: Arc<RegistryLifecycleRepository>,
    registry: SubagentRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    notify_tx: Option<NotificationTx>,
) -> Arc<TerminateAllDelegatedAgents> {
    let agents = Arc::new(RegistryDelegatedAgents::new(
        registry.clone(),
        broadcast_tx,
        notify_tx,
    ));
    Arc::new(TerminateAllDelegatedAgents::new(
        TerminateAllDelegatedAgentsPorts {
            lifecycle,
            registry: agents.clone(),
            routing: Arc::new(UdsDirectChildRouting::new(registry.clone())),
            termination: Arc::new(SupervisedChildTermination::new(registry)),
            compensation: agents,
            spawner: Arc::new(TokioShutdownRunSpawner),
        },
    ))
}

pub fn build_teardown_graph(inputs: TeardownGraphInputs) -> TeardownGraph {
    let harness_lifecycle = inputs
        .harness_lifecycle
        .unwrap_or_else(new_shared_harness_lifecycle);
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        inputs.registry.clone(),
        inputs.owner,
        harness_lifecycle,
    ));
    let clock = Arc::new(MonotonicShutdownClock::default());
    let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), clock);
    let registry_for_claims = inputs
        .registry
        .unwrap_or_else(|| Arc::new(Mutex::new(Default::default())));
    let routing = Arc::new(UdsDirectChildRouting::new(registry_for_claims.clone()));
    let exit = Arc::new(LoopExitReadiness::new(inputs.exit_notify));
    let persistence = Arc::new(DeferredLoopPersistence::default());
    let prepare = Arc::new(PrepareHarnessShutdown::new(transaction.clone()));
    // The fleet's compensations run with the FleetTeardown cause, for which
    // the compensation emits no passive note (notes are for natural exits
    // only, like `agent_cmd kill`'s SelectedTermination), so the loop's
    // notifier is passed through only for the exits it joins.
    let fleet = build_fleet_over(
        lifecycle.clone(),
        registry_for_claims.clone(),
        inputs.broadcast_tx.clone(),
        inputs.notify_tx.clone(),
    );
    let execute = Arc::new(ExecuteHarnessShutdown::new(
        transaction.clone(),
        ExecuteHarnessShutdownPorts {
            children: fleet.clone(),
            cancellation: Arc::new(LoopTurnCancellation {
                cancel_handle: inputs.cancel_handle,
                turn_control: inputs.turn_control,
                busy: inputs.busy.clone(),
            }),
            persistence: persistence.clone(),
            exit: exit.clone(),
            spawner: Arc::new(TokioShutdownRunSpawner),
        },
    ));
    // The receiver of a selected termination is the direct owner of the
    // target it shuts down (#1936): it claims the row stopping before the
    // edge, observes the exit — the owned-handle fallback when the protocol
    // does not suffice — and runs the row's compensation, broadcasting the
    // survivor roster on this loop's event stream.
    let agents = Arc::new(RegistryDelegatedAgents::new(
        registry_for_claims.clone(),
        inputs.broadcast_tx,
        inputs.notify_tx,
    ));
    let terminate = Arc::new(
        TerminateDelegatedAgent::new(lifecycle, routing).with_owner_conclusion(
            OwnerConclusionPorts {
                registry: agents.clone(),
                termination: Arc::new(SupervisedChildTermination::new(registry_for_claims)),
                compensation: agents,
            },
        ),
    );
    let controller = Arc::new(SubagentTeardownController::new(prepare, execute, terminate));
    TeardownGraph {
        connections: Arc::new(ConnectionTeardown {
            binding: Arc::new(Mutex::new(inputs.binding)),
            controller: controller.clone(),
            fleet: fleet.clone(),
            busy: inputs.busy,
        }),
        controller,
        fleet,
        transaction,
        exit,
        persistence,
    }
}
