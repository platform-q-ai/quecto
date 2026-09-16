use super::{AgentEvent, DispatchCtx, emit_event_to_broadcast_or_writer};
use crate::application::sessions::resume_endpoint::{
    ResumeEndpointAction, ResumeEndpointRequest, execute_resume_endpoint,
};
use crate::interface::cli::protocol::ResumeDecisionActionCommand;

pub(super) async fn handle_resume_decision(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    session: String,
    action: ResumeDecisionActionCommand,
    location: Option<String>,
) -> bool {
    if ctx.session.is_streaming() {
        emit_event_to_broadcast_or_writer(
            ctx,
            &AgentEvent::err(id, type_name, "cannot decide resume while agent is running"),
        )
        .await;
        return false;
    }
    let action = match action {
        ResumeDecisionActionCommand::OpenOriginal => ResumeEndpointAction::OpenOriginal,
        ResumeDecisionActionCommand::ForkCurrent => ResumeEndpointAction::ForkCurrent,
        ResumeDecisionActionCommand::Locate => ResumeEndpointAction::Locate,
        ResumeDecisionActionCommand::Cancel => ResumeEndpointAction::Cancel,
    };
    let request = ResumeEndpointRequest {
        persisted_key: session,
        action,
        location,
    };
    let event = match execute_resume_endpoint(ctx.resume_decision.as_ref(), request).await {
        Ok(value) => AgentEvent::ok(
            id,
            type_name,
            Some(serde_json::json!({"outcome": format!("{value:?}")})),
        ),
        Err(error) => AgentEvent::err(id, type_name, error),
    };
    emit_event_to_broadcast_or_writer(ctx, &event).await;
    false
}
