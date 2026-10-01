//! Swarm-specific execution ports and cancellation wiring at the CLI boundary.
use crate::application::agent_loop::AgentLoopImpl;
use crate::infrastructure::tools::swarm_bridge::{Participation, SwarmContext};
use std::sync::Arc;

/// A member's tool gate, request usage record and model gates, all over
/// its `SwarmContext`; none without one (#2339).
pub fn wire_agent(agent: AgentLoopImpl, context: Option<SwarmContext>) -> AgentLoopImpl {
    let context = context.map(Arc::new);
    agent
        .with_tool_admission(context.clone().map(|value| {
            value as Arc<dyn crate::application::tools::ports::ToolExecutionAdmission>
        }))
        .with_request_accounting(
            context.clone().map(|value| {
                value as Arc<dyn crate::application::providers::ports::RequestAccounting>
            }),
        )
        .with_request_admission(
            context.map(|value| {
                value as Arc<dyn crate::application::providers::ports::RequestAdmission>
            }),
        )
}

/// A built member: [`wire_agent`] over this process's swarm context, and
/// its pruning ceiling lowered to `swarm_max_context_tokens` the moment the
/// process takes part in a swarm (#2342), or at once if it already does.
/// A process that never joins keeps its configured budget. Every agent
/// runs in the configured context mode (#2403): a member loads the
/// parent's configuration, so it inherits the parent's mode unless its own
/// configuration sets one.
pub(super) fn wire_member(
    mut agent: AgentLoopImpl,
    participation: &Participation,
    defaults: &crate::infrastructure::config::AgentDefaults,
) -> AgentLoopImpl {
    agent.set_context_mode(defaults.context_mode());
    let swarm_ceiling_tokens = defaults.swarm_max_context_tokens;
    let large_results = defaults.swarm_large_result_collapse();
    let cap = agent.context_ceiling_cap();
    let switch = agent.large_result_switch();
    participation.on_participation(move || {
        cap.lower_to(swarm_ceiling_tokens);
        // #2348 review M1: the size rule's evidence is swarm-only, so it is
        // on by default only here.
        switch.engage(large_results);
        tracing::info!(
            target: "quecto::swarm_board",
            ceiling_tokens = swarm_ceiling_tokens,
            large_result_tokens = large_results.over_tokens,
            large_result_after_turns = large_results.after_turns,
            "swarm member context ceiling engaged"
        );
    });
    wire_agent(agent, crate::interface::tool_runtime::swarm_context())
}

pub(super) fn bind_suspension(
    cancel: &super::uds_cancel::CancelHandle,
    context: Option<SwarmContext>,
) {
    if let Some(context) = context {
        crate::infrastructure::tools::swarm_lifecycle::bind_local_suspension(suspension_callback(
            cancel, context,
        ));
    }
}

pub(super) fn suspension_callback(
    cancel: &super::uds_cancel::CancelHandle,
    context: SwarmContext,
) -> Arc<dyn Fn(crate::domain::swarm::RunStatus, u64) + Send + Sync> {
    let cancel = cancel.clone();
    Arc::new(move |status, generation| {
        super::uds_cancel::suspend_swarm_turn(&cancel, generation, &|| {
            context
                .control_status()
                .and_then(|value| SwarmContext::decode_control_receipt(value, false))
                .is_ok_and(|current| current.status == status && current.generation == generation)
        });
    })
}

#[cfg(test)]
#[path = "swarm_composition_tests.rs"]
mod tests;
