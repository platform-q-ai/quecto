//! Swarm-specific execution ports and cancellation wiring at the CLI boundary.
use crate::application::agent_loop::AgentLoopImpl;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use std::sync::Arc;

pub(super) fn wire_agent(agent: AgentLoopImpl, context: Option<SwarmContext>) -> AgentLoopImpl {
    let context = context.map(Arc::new);
    agent
        .with_tool_admission(
            context
                .clone()
                .map(|value| value as Arc<dyn crate::domain::tool::ToolExecutionAdmission>),
        )
        .with_request_accounting(
            context.clone().map(|value| {
                value as Arc<dyn crate::domain::request_observation::RequestAccounting>
            }),
        )
        .with_request_admission(
            context.map(|value| value as Arc<dyn crate::domain::provider::RequestAdmission>),
        )
}

pub(super) fn bind_suspension(
    cancel: &super::uds_cancel::CancelHandle,
    context: Option<SwarmContext>,
) {
    if let Some(context) = context {
        let cancel = cancel.clone();
        crate::infrastructure::tools::swarm_lifecycle::bind_local_suspension(Arc::new(
            move |status, generation| {
                super::uds_cancel::suspend_swarm_turn(&cancel, generation, || {
                    context
                        .control_status()
                        .and_then(|value| SwarmContext::decode_control_receipt(value, false))
                        .is_ok_and(|current| {
                            current.status == status && current.generation == generation
                        })
                });
            },
        ));
    }
}
