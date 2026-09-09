use super::*;

pub(super) async fn arm_prompt_cancel(
    ctx: &mut DispatchCtx<'_>,
    is_steer_prompt: bool,
) -> Option<tokio::sync::oneshot::Receiver<()>> {
    if is_steer_prompt {
        ctx.turn_control.consume_steer();
    }
    match crate::interface::cli::uds_cancel::arm_swarm_cancel(&ctx.cancel_handle, &ctx.turn_control)
        .await
    {
        Some(rx) => Some(rx),
        None if is_steer_prompt => {
            crate::interface::cli::uds_cancel::arm_swarm_cancel(
                &ctx.cancel_handle,
                &ctx.turn_control,
            )
            .await
        }
        None => None,
    }
}

pub(super) fn control_status(
    outcome: &PromptOutcome,
) -> crate::interface::cli::protocol::ControlStatus {
    match outcome {
        PromptOutcome::Success => crate::interface::cli::protocol::ControlStatus::Completed,
        PromptOutcome::Error => crate::interface::cli::protocol::ControlStatus::Failed,
        PromptOutcome::Cancelled => crate::interface::cli::protocol::ControlStatus::Cancelled,
    }
}

pub(super) async fn emit_pre_cancelled(ctx: &mut DispatchCtx<'_>) {
    emit_event_to_broadcast_or_writer(ctx, &AgentEvent::AgentStart).await;
    emit_event_to_broadcast_or_writer(
        ctx,
        &AgentEvent::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        },
    )
    .await;
}

pub(super) async fn handle_busy_prompt(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
    streaming_behavior: Option<StreamingBehavior>,
) {
    match streaming_behavior {
        Some(behavior) => {
            pending::queue_prompt(
                ctx,
                id,
                type_name,
                message,
                matches!(behavior, StreamingBehavior::Steer),
            )
            .await;
        }
        None => {
            ctx.session.record_control(
                id,
                type_name,
                crate::interface::cli::protocol::ControlStatus::Rejected,
            );
            let msg = "agent is running; provide streamingBehavior";
            let ev = AgentEvent::err(id, type_name, msg);
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
        }
    }
}
