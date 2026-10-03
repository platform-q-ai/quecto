use super::*;

/// Images a reader admitted for the command it hands to dispatch (#2422):
/// dispatch runs them as admitted, so a message's images are decoded once.
pub(in crate::interface::cli) type AdmittedImages = Vec<crate::domain::message::UserImageBlock>;

/// Admit a message's images through `quecto_image`, the one place images
/// from outside are validated.
pub(in crate::interface::cli) fn admit_images(
    images: Vec<quecto_image::ImagePayload>,
) -> Result<AdmittedImages, quecto_image::ImagesRefusal> {
    quecto_image::validate_images(images)
        .map(|admitted| admitted.into_iter().map(Into::into).collect())
}

/// A steer's admitted images; `None` when `trimmed` is not a steer, or is
/// one whose images dispatch would refuse (#2422): such a steer must never
/// cancel the running turn before it is refused.
pub(in crate::interface::cli) fn steer_images(trimmed: &str) -> Option<AdmittedImages> {
    match serde_json::from_str::<AgentCommand>(trimmed) {
        Ok(
            AgentCommand::Steer { images, .. }
            | AgentCommand::Prompt {
                streaming_behavior: Some(StreamingBehavior::Steer),
                images,
                ..
            },
        ) => admit_images(images).ok(),
        _ => None,
    }
}

#[cfg(test)]
pub(in crate::interface::cli) fn is_steer_command(trimmed: &str) -> bool {
    steer_images(trimmed).is_some()
}

/// Record the `Rejected` receipt of a control the reader refused itself
/// (#2422), as dispatch does for one it refuses.
pub(in crate::interface::cli) async fn record_rejected(
    ctx: &mut DispatchCtx<'_>,
    id: &str,
    command: &str,
) {
    let _ = (ctx, id, command); // red (#2422 review round 1): no receipt
}

/// Run a parsed command, with the images its reader admitted, if any.
pub(in crate::interface::cli) async fn dispatch_parsed(
    cmd: AgentCommand,
    admitted: Option<AdmittedImages>,
    ctx: &mut DispatchCtx<'_>,
) -> bool {
    match admitted {
        Some(images) => dispatch_admitted(cmd, images, ctx).await,
        None => super::dispatch_command(cmd, ctx).await,
    }
}

/// Run a message command whose images a reader already admitted.
pub(in crate::interface::cli) async fn dispatch_admitted(
    cmd: AgentCommand,
    images: AdmittedImages,
    ctx: &mut DispatchCtx<'_>,
) -> bool {
    let id = cmd.id().map(str::to_owned);
    let type_name = cmd.type_name().to_owned();
    let (text, kind) = match cmd {
        AgentCommand::Prompt {
            message,
            streaming_behavior,
            ..
        } => (message, MessageKind::Prompt(streaming_behavior)),
        AgentCommand::Steer { message, .. } => (message, MessageKind::Steer),
        AgentCommand::FollowUp { message, .. } => (message, MessageKind::FollowUp),
        other => {
            assert!(images.is_empty(), "only a message command carries images");
            return super::dispatch_command(other, ctx).await;
        }
    };
    let body = crate::interface::cli::uds_session::PromptBody { text, images };
    run_message(ctx, id, type_name, body, kind).await
}

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
    message: crate::interface::cli::uds_session::PromptBody,
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

/// Which message command `dispatch_message` runs.
pub(super) enum MessageKind {
    Prompt(Option<StreamingBehavior>),
    Steer,
    FollowUp,
}

/// Run a `prompt`, `steer` or `follow_up` once its images are admitted.
pub(super) async fn dispatch_message(
    ctx: &mut DispatchCtx<'_>,
    id: Option<String>,
    type_name: String,
    (text, images): (String, Vec<quecto_image::ImagePayload>),
    kind: MessageKind,
) -> bool {
    let Some(body) = admit_prompt_body(ctx, id.as_deref(), &type_name, text, images).await else {
        return false;
    };
    run_message(ctx, id, type_name, body, kind).await
}

async fn run_message(
    ctx: &mut DispatchCtx<'_>,
    id: Option<String>,
    type_name: String,
    body: crate::interface::cli::uds_session::PromptBody,
    kind: MessageKind,
) -> bool {
    match kind {
        MessageKind::Prompt(streaming_behavior) => {
            let cmd = PromptCommand {
                id,
                type_name,
                message: body,
                streaming_behavior,
            };
            super::handle_prompt(ctx, cmd).await
        }
        MessageKind::Steer => {
            super::uds_dispatch::handle_steer(ctx, id.as_deref(), &type_name, body).await
        }
        MessageKind::FollowUp => {
            super::uds_dispatch::handle_follow_up(ctx, id.as_deref(), &type_name, body).await
        }
    }
}

/// Admit a `prompt` / `steer` / `follow_up`'s images (#2422), before the
/// command does anything else. `quecto_image` is the one place images are
/// validated; any refused image refuses the whole command with its exact
/// message, so nothing runs, queues or cancels, and `None` is returned.
async fn admit_prompt_body(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    text: String,
    images: Vec<quecto_image::ImagePayload>,
) -> Option<crate::interface::cli::uds_session::PromptBody> {
    match admit_images(images) {
        Ok(images) => Some(crate::interface::cli::uds_session::PromptBody { text, images }),
        Err(refusal) => {
            ctx.session.record_control(
                id,
                type_name,
                crate::interface::cli::protocol::ControlStatus::Rejected,
            );
            let ev = AgentEvent::err(id, type_name, refusal.to_string());
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            None
        }
    }
}
