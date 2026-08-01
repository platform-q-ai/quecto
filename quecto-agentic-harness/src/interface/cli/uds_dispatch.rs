#[cfg(test)]
pub(super) use super::persist_user_prompt_before_run;
#[cfg(test)]
pub(super) use super::uds_dispatch_forwarding::forward_subagent_get_messages;
use super::uds_dispatch_forwarding::try_forward_subagent_targeted_command;
#[cfg(test)]
pub(super) use super::uds_dispatch_get_message_forward::{
    ForwardGetMessage, forward_subagent_get_message,
};
use super::uds_dispatch_runtime::{SetModelArgs, handle_set_effort, handle_set_model};
#[cfg(test)]
pub(super) use super::uds_dispatch_session::{
    handle_clear_history, handle_new_session, handle_resume_session, handle_rewind_to,
    persist_current_session,
};
#[cfg(not(test))]
use super::uds_dispatch_session::{handle_new_session, handle_resume_session, handle_rewind_to};
use super::{AgentCommand, AgentEvent};
use super::{DispatchCtx, emit_event_to_broadcast_or_writer};
use crate::interface::cli::uds_ext_protocol;

pub(crate) async fn dispatch_command(cmd: AgentCommand, ctx: &mut DispatchCtx<'_>) -> bool {
    if let Some(result) = try_forward_subagent_targeted_command(&cmd, ctx).await {
        return result;
    }
    // Fast path: queries + clear_history (defers id/type_name clones).
    if let Some(result) = super::uds_dispatch_query::dispatch_fieldless_command(&cmd, ctx).await {
        return result;
    }

    let id = cmd.id().map(str::to_owned);
    let type_name = cmd.type_name().to_owned();

    match cmd {
        AgentCommand::Prompt {
            message,
            streaming_behavior,
            ..
        } => {
            super::handle_prompt(
                ctx,
                super::PromptCommand {
                    id,
                    type_name,
                    message,
                    streaming_behavior,
                },
            )
            .await
        }
        AgentCommand::Steer { message, .. } => {
            handle_steer(ctx, id.as_deref(), &type_name, message).await
        }
        AgentCommand::FollowUp { message, .. } => {
            handle_follow_up(ctx, id.as_deref(), &type_name, message).await
        }
        AgentCommand::Abort { .. } => handle_abort(ctx, id.as_deref(), &type_name).await,
        AgentCommand::RewindTo {
            message_index,
            message_id,
            ..
        } => handle_rewind_to(ctx, id.as_deref(), &type_name, message_index, message_id).await,
        AgentCommand::SetWorkflowAutomation {
            auto_continue,
            completion_nudge,
            ..
        } => {
            handle_set_workflow_automation(
                ctx,
                id.as_deref(),
                &type_name,
                auto_continue,
                completion_nudge,
            )
            .await
        }
        AgentCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            handle_set_model(
                SetModelArgs {
                    id,
                    type_name,
                    model,
                    provider,
                    model_id,
                },
                ctx,
            )
            .await
        }
        AgentCommand::SetEffort { effort, .. } => {
            handle_set_effort(ctx, id.as_deref(), &type_name, &effort).await
        }
        AgentCommand::Reload { .. } => {
            super::super::uds_reload::handle_reload(ctx, id.as_deref(), &type_name).await
        }
        AgentCommand::NewSession { .. } => handle_new_session(ctx, id.as_deref(), &type_name).await,
        AgentCommand::ResumeSession { session, .. } => {
            handle_resume_session(ctx, id.as_deref(), &type_name, session).await
        }
        AgentCommand::RegisterTools { .. }
        | AgentCommand::UnregisterTools { .. }
        | AgentCommand::ToolResult { .. } => {
            dispatch_ext_command(cmd, ctx, id.as_deref(), &type_name).await
        }
        // Exhaustive: variants handled above; agent-targeted history/sync by pre-router.
        AgentCommand::ClearHistory { .. }
        | AgentCommand::DeleteAllSubagents { .. }
        | AgentCommand::ListModels { .. }
        | AgentCommand::GetToolCatalogue { .. }
        | AgentCommand::GetSubagents { .. }
        | AgentCommand::GetMessage { .. }
        | AgentCommand::GetState { .. }
        | AgentCommand::GetMessages { .. }
        | AgentCommand::GetMessagesTail { .. }
        | AgentCommand::Sync { .. }
        | AgentCommand::GetSessionStats { .. }
        | AgentCommand::ListSessions { .. } => {
            tracing::error!(command = %type_name, "fieldless variant reached dispatch fallback");
            let ev = AgentEvent::err(id.as_deref(), &type_name, "internal: unhandled command");
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            false
        }
    }
}

pub(super) async fn handle_steer(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
    if ctx.session.is_streaming() {
        // Reader task already fires cancel eagerly — do NOT fire again here (#512).
        ctx.session.prepend_pending(message);
    } else {
        ctx.session.enqueue_pending(message);
    }
    // #896: this steer is now being handled, so release the pending-steer gate —
    // the workflow auto-continue nudge may resume AFTER the steer runs, not
    // before it. Clearing here (rather than in the reader) keeps the nudge
    // suppressed across the post-cancel idle drain that ran just ahead of us.
    ctx.turn_control.clear_steer();
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    // When idle (e.g. the steered turn was just cancelled and unwound), drive the
    // steered instruction now so it isn't stranded waiting for the next prompt.
    if !ctx.session.is_streaming() {
        super::drain_pending_and_nudge(ctx).await;
    }
    false
}

pub(super) async fn handle_follow_up(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
    // A genuine `follow_up` clears any stale steer gate left by a loose
    // `"type":"steer"` substring match in the reader, so the auto-continue
    // nudge is not permanently suppressed (#896 AC3). See `handle_prompt`.
    ctx.turn_control.clear_steer();
    ctx.session.enqueue_pending(message);
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    if !ctx.session.is_streaming() {
        super::drain_pending_and_nudge(ctx).await;
    }
    false
}

pub(super) async fn handle_set_workflow_automation(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    auto_continue: Option<bool>,
    completion_nudge: Option<bool>,
) -> bool {
    let Some(config) = ctx.workflow_config.as_mut() else {
        let ev = AgentEvent::err(id, type_name, "workflow is not active");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    };
    if let Some(value) = auto_continue {
        config.auto_continue = value;
    }
    if let Some(value) = completion_nudge {
        config.completion_nudge = value;
    }
    if let Some(workflow) = &ctx.workflow_state
        && let Ok(mut engine) = workflow.lock()
    {
        engine.set_automation(config.auto_continue, config.completion_nudge);
    }
    let ev = AgentEvent::ok(
        id,
        type_name,
        Some(serde_json::json!({
            "autoContinue": config.auto_continue,
            "completionNudge": config.completion_nudge,
        })),
    );
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

pub(super) async fn handle_abort(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
) -> bool {
    // Reader task already fires cancel eagerly — do NOT fire again here (#512).
    // #895: abort = full stop. Discard any queued work and clear both control
    // flags so the bound workflow does not resume and the next idle drain does
    // not re-drive this agent. Suppression lasts until a fresh prompt re-arms the
    // loop. The in-flight prompt's idle drain may already have consumed the abort
    // flag; doing it again here is idempotent and covers the idle (no-run) case.
    ctx.session.drain_pending();
    ctx.turn_control.clear();
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

pub(super) async fn dispatch_ext_command(
    cmd: AgentCommand,
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    _tn: &str,
) -> bool {
    match cmd {
        AgentCommand::RegisterTools { tools, .. } => {
            uds_ext_protocol::dispatch_register_tools(ctx, id, &tools).await
        }
        AgentCommand::UnregisterTools { tools, .. } => {
            uds_ext_protocol::dispatch_unregister_tools(ctx, id, &tools).await
        }
        AgentCommand::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => uds_ext_protocol::dispatch_tool_result(ctx, &tool_call_id, &content, is_error),
        _ => {}
    };
    false
}

#[cfg(test)]
#[path = "uds_dispatch_935_clamp_tests.rs"]
mod clamp_935_tests;
#[cfg(test)]
#[path = "uds_dispatch_cov2_tests.rs"]
mod cov2_tests;
#[cfg(test)]
#[path = "uds_dispatch_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "uds_dispatch_1060_lifecycle_tests.rs"]
mod lifecycle_1060_tests;
#[cfg(test)]
#[path = "uds_dispatch_masked_pruning_tests.rs"]
mod masked_pruning_tests;
#[cfg(test)]
#[path = "uds_dispatch_resume_persist_tests.rs"]
mod resume_persist_tests;
#[cfg(test)]
#[path = "uds_dispatch_1093_tests.rs"]
mod tests_1093;
#[cfg(test)]
#[path = "uds_dispatch_843_tests.rs"]
mod tests_843;
#[cfg(test)]
#[path = "uds_dispatch_843_targeted_tests.rs"]
mod tests_843_targeted;
