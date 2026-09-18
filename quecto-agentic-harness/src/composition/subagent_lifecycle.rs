//! The concrete graph of the launch-side lifecycle (#1936): the exit
//! observation the reaper and monitor of every direct child report to, and
//! the launch rollback, over the registry-backed compensation, the
//! one-edge UDS routing adapter and the supervised owned-handle fallback.
//! Built once per harness beside its agent-control tools and installed in
//! the spawn tool's lifecycle slot; fixtures that launch through their own
//! `SpawnTool` build the same graph over their own registry.
use std::path::Path;
use std::sync::Arc;

use crate::application::configuration::dto::{
    ConfigLayers, ConfigSelection, ConfigSelectionRequest,
};
use crate::application::configuration::use_cases::SelectConfig;
use crate::application::subagents::use_cases::{
    CompensateFailedLaunch, CompensateFailedLaunchPorts, ObserveOwnedChildExit,
};
use crate::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use crate::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use crate::infrastructure::tools::spawn::SpawnTool;
use crate::infrastructure::tools::subagent_registry::{NotificationTx, SubagentRegistry};
use crate::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;
pub use crate::infrastructure::tools::subagent_teardown_wiring::SubagentLifecycleUseCases;

/// The lifecycle use cases a launcher hands its reaper, monitor and
/// rollback, over the launcher's registry, event stream and notification
/// channel.
pub fn build_lifecycle_use_cases(
    registry: SubagentRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    notify_tx: Option<NotificationTx>,
) -> SubagentLifecycleUseCases {
    let agents = Arc::new(RegistryDelegatedAgents::new(
        registry.clone(),
        broadcast_tx,
        notify_tx,
        super::environments::build_member_finalizer,
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

/// Install the lifecycle use cases in a launcher over its own registry,
/// event stream and notification channel, and the container-config
/// selection (#2024 S4a) over the launcher's parent config path with no
/// checkout overlay: integration launchers and BDD fixtures that build a
/// `SpawnTool` outside the agent-control build. A launcher with no parent
/// config path gets a selection with no launching-agent source, so a
/// container spawn that names no file fails with the clear error.
pub fn compose_launcher(tool: SpawnTool) -> SpawnTool {
    let selection = tool.parent_config_path().map(|global| {
        ConfigSelection::Layered(ConfigLayers {
            global: global.to_path_buf(),
            overlay: None,
            legacy_local: None,
        })
    });
    compose_launcher_with_selection(tool, selection)
}

/// As [`compose_launcher`], for a launcher whose agent works in
/// `checkout`: the parent config path is the global file and
/// `<checkout>/.quecto/config.json` its overlay, discovered as the agent
/// build discovers them for the working directory.
pub fn compose_launcher_in_checkout(tool: SpawnTool, checkout: &Path) -> SpawnTool {
    let selection = tool.parent_config_path().map(|global| {
        SelectConfig::new().execute(ConfigSelectionRequest {
            explicit: None,
            working_directory: Some(checkout.to_path_buf()),
            global: global.to_path_buf(),
        })
    });
    compose_launcher_with_selection(tool, selection)
}

fn compose_launcher_with_selection(
    tool: SpawnTool,
    selection: Option<ConfigSelection>,
) -> SpawnTool {
    let use_cases = build_lifecycle_use_cases(
        tool.registry().clone(),
        tool.broadcast_tx().cloned(),
        tool.notify_tx().cloned(),
    );
    let container_configs =
        super::container_configs::build_container_config_handles(tool.base_dir(), selection);
    tool.with_lifecycle_use_cases(use_cases)
        .with_container_config_selection(Some(container_configs.selection))
        .with_container_config_roster(Some(container_configs.roster))
}
