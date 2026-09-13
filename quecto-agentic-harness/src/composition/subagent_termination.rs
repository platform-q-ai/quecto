//! The concrete graph of operator-selected termination (#1936, #1882) and
//! the environment / swarm member terminations that delegate to it (#1939):
//! `agent_cmd kill` → the interface kill tool → `KillDelegatedAgent` over
//! the registry-backed lifecycle adapters, the one-edge routing adapter and
//! the supervised owned-handle fallback; `kill_container` → the
//! environments capability's member-shutdown port over the same per-child
//! settlement; a swarm settlement → the delegated-agent graph for members
//! this harness launched. Built once per harness beside its agent-control
//! tools and handed to `AgentCmdTool` as the owner of `kill`.
use std::sync::Arc;

use crate::application::environments::ports::EnvironmentMemberShutdown;
use crate::application::subagents::use_cases::{
    KillDelegatedAgent, KillDelegatedAgentPorts, OwnerConclusionPorts, SettleDelegatedChild,
    SettleDelegatedChildPorts, TerminateDelegatedAgent,
};
use crate::domain::tool::Tool;
use crate::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use crate::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use crate::infrastructure::tools::environment_member_shutdown::DelegatedMemberShutdown;
use crate::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;
use crate::infrastructure::tools::swarm_member_termination::DelegatedSwarmMemberTermination;
use crate::interface::cli::KillToolWiring;
use crate::interface::cli::uds_teardown_adapters::RegistryLifecycleRepository;
use crate::interface::tools::agent_cmd_kill::KillDelegatedAgentTool;

/// The parent-hand termination owners of one harness over one shared
/// delegated-agent graph (#1936, #1939): the `agent_cmd kill` tool, the
/// environment member shutdown `kill_container` asks before its retained
/// kill, and the swarm member termination a settling swarm run uses for
/// members this harness launched.
pub struct TerminationOwners {
    pub kill_tool: Arc<dyn Tool>,
    pub member_shutdown: Arc<dyn EnvironmentMemberShutdown>,
    pub swarm_member_termination: Arc<DelegatedSwarmMemberTermination>,
}

/// The `agent_cmd kill` owner alone (tests and fixtures that need only the
/// tool).
pub fn build_kill_tool(inputs: KillToolWiring) -> Arc<dyn Tool> {
    build_termination_owners(inputs).kill_tool
}

/// Build the owners over the wiring and install them where the built
/// agent-control tools read them: the kill and member-shutdown slots, and
/// the swarm member termination bound once for the settling runs. `true`
/// when every slot was empty and took its owner: one set per harness. The
/// `KillToolBuilder` the CLI context carries.
pub fn install_termination_owners(inputs: KillToolWiring) -> bool {
    let slots = inputs.slots.clone();
    let owners = build_termination_owners(inputs);
    let kill_installed = slots.kill.install(owners.kill_tool);
    let shutdown_installed = slots.member_shutdown.install(owners.member_shutdown);
    crate::infrastructure::tools::swarm_lifecycle::bind_member_termination(
        owners.swarm_member_termination,
    );
    kill_installed && shutdown_installed
}

/// Every parent-hand termination owner of one harness over one shared
/// graph: the operator kill tool, the environment member shutdown and the
/// swarm member termination.
pub fn build_termination_owners(inputs: KillToolWiring) -> TerminationOwners {
    let agents = Arc::new(RegistryDelegatedAgents::new(
        inputs.registry.clone(),
        inputs.broadcast_tx,
        inputs.notify_tx,
    ));
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        Some(inputs.registry.clone()),
        inputs.owner,
        inputs.harness_lifecycle,
    ));
    let routing = Arc::new(UdsDirectChildRouting::new(inputs.registry.clone()));
    let termination = Arc::new(SupervisedChildTermination::new(inputs.registry.clone()));
    // A selected direct child is concluded by the route itself, as this
    // harness's own: protocol, observed exit, owned-handle fallback only
    // when the protocol did not suffice, compensation. The kill above it
    // resolves, claims, routes and joins; it owns no termination port.
    let route = Arc::new(
        TerminateDelegatedAgent::new(lifecycle.clone(), routing.clone()).with_owner_conclusion(
            OwnerConclusionPorts {
                registry: agents.clone(),
                termination: termination.clone(),
                compensation: agents.clone(),
            },
        ),
    );
    let kill = Arc::new(KillDelegatedAgent::new(
        route,
        KillDelegatedAgentPorts {
            registry: agents.clone(),
            lifecycle,
            compensation: agents.clone(),
        },
    ));
    let settle = Arc::new(SettleDelegatedChild::new(SettleDelegatedChildPorts {
        registry: agents.clone(),
        routing,
        termination,
        compensation: agents.clone(),
    }));
    let member_shutdown: Arc<dyn EnvironmentMemberShutdown> =
        Arc::new(DelegatedMemberShutdown::new(agents, settle));
    TerminationOwners {
        kill_tool: Arc::new(KillDelegatedAgentTool::new(kill.clone())),
        member_shutdown,
        swarm_member_termination: Arc::new(DelegatedSwarmMemberTermination::new(
            inputs.registry,
            kill,
        )),
    }
}
