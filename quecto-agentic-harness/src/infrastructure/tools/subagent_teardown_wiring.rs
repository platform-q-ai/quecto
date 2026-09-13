//! The launch-side lifecycle use cases of one registry (#1936): what the
//! reaper, the monitor and the launch rollback invoke when a direct child
//! ends. Built from the same registry, event stream and supervisor the
//! `SpawnTool` holds, so every launcher of this process — production,
//! integration launchers, BDD fixtures — observes exits through one path.
//! The operator-facing kill tool is composed separately
//! (`composition::subagent_termination`) over the same adapters.
use std::sync::Arc;

use crate::application::subagents::use_cases::{
    CompensateFailedLaunch, CompensateFailedLaunchPorts, ObserveOwnedChildExit,
};
use crate::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use crate::infrastructure::processes::owned_child_termination::SupervisedChildTermination;

use super::subagent_registry::{NotificationTx, SubagentRegistry};
use super::subagent_teardown_registry::RegistryDelegatedAgents;

/// The use cases a launcher hands its reaper, monitor and rollback.
#[derive(Clone)]
pub struct SubagentLifecycleUseCases {
    pub observe_exit: Arc<ObserveOwnedChildExit>,
    pub compensate_launch: Arc<CompensateFailedLaunch>,
}

pub fn build_lifecycle_use_cases(
    registry: SubagentRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    notify_tx: Option<NotificationTx>,
) -> SubagentLifecycleUseCases {
    let agents = Arc::new(RegistryDelegatedAgents::new(
        registry.clone(),
        broadcast_tx,
        notify_tx,
    ));
    let termination = Arc::new(SupervisedChildTermination::new(registry.clone()));
    let routing = Arc::new(UdsDirectChildRouting::new(registry));
    SubagentLifecycleUseCases {
        observe_exit: Arc::new(ObserveOwnedChildExit::new(agents.clone(), agents.clone())),
        compensate_launch: Arc::new(CompensateFailedLaunch::new(CompensateFailedLaunchPorts {
            registry: agents.clone(),
            routing,
            termination,
            compensation: agents,
        })),
    }
}
