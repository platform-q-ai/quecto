//! The concrete graph of the subagent teardown capability for one harness
//! (#1935): the slice A use cases wired to the loop-bound adapters, the UDS
//! routing adapter to direct children, and the launch-bound parent binding
//! the connection layer consults. The interface receives this builder
//! through its `CliContext` (`run_composed`) and never names this module.
use std::sync::{Arc, Mutex};

use crate::application::subagents::use_cases::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    PrepareHarnessShutdown, TerminateDelegatedAgent,
};
use crate::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use crate::interface::cli::uds_teardown_adapters::{
    DeferredLoopPersistence, LoopExitReadiness, LoopTurnCancellation, MonotonicShutdownClock,
    RegistryLifecycleRepository, TokioShutdownRunSpawner,
};
use crate::interface::uds::subagent_teardown::controller::SubagentTeardownController;

pub use crate::interface::cli::uds_parent_control::{ConnectionRole, ConnectionTeardown};
pub use crate::interface::cli::uds_teardown_adapters::{
    DeferredLoopPersistence as LoopPersistenceAdapter, LoopExitReadiness as LoopExitAdapter,
    RegistryLifecycleRepository as LifecycleAdapter,
};
pub use crate::interface::cli::uds_teardown_graph::{
    ParentControlLaunch, TeardownGraph, TeardownGraphBuilder, TeardownGraphInputs,
};

pub fn build_teardown_graph(inputs: TeardownGraphInputs) -> TeardownGraph {
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        inputs.registry.clone(),
        inputs.owner,
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
    let execute = Arc::new(ExecuteHarnessShutdown::new(
        transaction.clone(),
        ExecuteHarnessShutdownPorts {
            routing: routing.clone(),
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
    // The receiver claims a selected target stopping before routing its
    // edge (#1936), so its reaper honours the intent when the child exits.
    let agents = Arc::new(
        crate::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents::new(
            registry_for_claims,
            None,
            None,
        ),
    );
    let terminate =
        Arc::new(TerminateDelegatedAgent::new(lifecycle, routing).with_registry(agents));
    let controller = Arc::new(SubagentTeardownController::new(prepare, execute, terminate));
    TeardownGraph {
        connections: Arc::new(ConnectionTeardown {
            binding: Arc::new(Mutex::new(inputs.binding)),
            controller,
            busy: inputs.busy,
        }),
        transaction,
        exit,
        persistence,
    }
}
