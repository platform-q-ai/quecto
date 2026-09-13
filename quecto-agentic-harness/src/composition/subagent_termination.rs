//! The concrete graph of operator-selected termination (#1936, #1882):
//! `agent_cmd kill` → the interface kill tool → `KillDelegatedAgent` over
//! the registry-backed lifecycle adapters, the one-edge routing adapter and
//! the supervised owned-handle fallback. Built once per harness beside its
//! agent-control tools and handed to `AgentCmdTool` as the owner of `kill`.
use std::sync::Arc;

use crate::application::subagents::use_cases::{
    KillDelegatedAgent, KillDelegatedAgentPorts, TerminateDelegatedAgent,
};
use crate::domain::tool::Tool;
use crate::infrastructure::extensions::native::KillToolWiring;
use crate::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use crate::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use crate::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;
use crate::interface::cli::uds_teardown_adapters::RegistryLifecycleRepository;
use crate::interface::tools::agent_cmd_kill::KillDelegatedAgentTool;

pub fn build_kill_tool(inputs: KillToolWiring) -> Arc<dyn Tool> {
    let agents = Arc::new(RegistryDelegatedAgents::new(
        inputs.registry.clone(),
        inputs.broadcast_tx,
        inputs.notify_tx,
    ));
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        Some(inputs.registry.clone()),
        inputs.owner,
    ));
    let routing = Arc::new(UdsDirectChildRouting::new(inputs.registry.clone()));
    let route = Arc::new(
        TerminateDelegatedAgent::new(lifecycle.clone(), routing).with_registry(agents.clone()),
    );
    let use_case = Arc::new(KillDelegatedAgent::new(
        route,
        KillDelegatedAgentPorts {
            registry: agents.clone(),
            lifecycle,
            termination: Arc::new(SupervisedChildTermination::new(inputs.registry)),
            compensation: agents,
        },
    ));
    Arc::new(KillDelegatedAgentTool::new(use_case))
}
