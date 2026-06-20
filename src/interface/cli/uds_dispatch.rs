use super::*;
use crate::interface::cli::uds_ext_protocol;

pub(crate) async fn dispatch_command(cmd: AgentCommand, ctx: &mut DispatchCtx<'_>) -> bool {
    // Fast path: queries + clear_history (defers id/type_name clones).
    if let Some(result) = dispatch_fieldless_command(&cmd, ctx).await {
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
            handle_prompt(
                ctx,
                PromptCommand {
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
        AgentCommand::RewindTo { message_index, .. } => {
            handle_rewind_to(ctx, id.as_deref(), &type_name, message_index).await
        }
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
        AgentCommand::NewSession { .. } => handle_new_session(ctx, id.as_deref(), &type_name).await,
        AgentCommand::ResumeSession { session, .. } => {
            handle_resume_session(ctx, id.as_deref(), &type_name, session).await
        }
        AgentCommand::ReloadExtensions { .. }
        | AgentCommand::RegisterTools { .. }
        | AgentCommand::UnregisterTools { .. }
        | AgentCommand::ToolResult { .. } => {
            dispatch_ext_command(cmd, ctx, id.as_deref(), &type_name).await
        }
        // Exhaustive: variants handled by dispatch_fieldless_command above.
        AgentCommand::ClearHistory { .. }
        | AgentCommand::GetExtensions { .. }
        | AgentCommand::GetSubagents { .. }
        | AgentCommand::GetState { .. }
        | AgentCommand::GetMessages { .. }
        | AgentCommand::GetMessagesTail { .. }
        | AgentCommand::GetSessionStats { .. }
        | AgentCommand::ListSessions { .. } => {
            tracing::error!(command = %type_name, "fieldless variant reached dispatch fallback");
            let ev = AgentEvent::err(id.as_deref(), &type_name, "internal: unhandled command");
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            false
        }
    }
}

pub(super) async fn persist_current_session(
    ctx: &mut DispatchCtx<'_>,
) -> Result<(), crate::domain::error::DomainError> {
    if ctx.ephemeral || ctx.session_key.is_empty() {
        return Ok(());
    }
    remove_injected_system_prompt(ctx.messages, ctx.system_prompt);
    let session = Session {
        key: ctx.session_key.clone(),
        messages: ctx.messages.clone(),
        workflow_run: ctx
            .workflow_state
            .as_ref()
            .and_then(|ws| ws.lock().ok().and_then(|engine| engine.persisted_run())),
    };
    inject_system_prompt(ctx.messages, ctx.system_prompt);
    ctx.session_store.save(&session).await
}

pub(super) async fn handle_new_session(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
) -> bool {
    if ctx.session.is_streaming() {
        let ev = AgentEvent::err(
            id,
            type_name,
            "cannot start a new session while agent is running",
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    clear_conversation(ctx.messages);
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    let key = crate::domain::session::new_user_chat_key();
    ctx.session_key.clear();
    ctx.session_key.push_str(&key);
    ctx.session.set_session_key(key.clone());
    if let Some(spill) = ctx.agent.spill_store() {
        if let Err(e) = spill.clear(ctx.session_key).await {
            tracing::warn!("new_session: failed to clear spill store: {e}");
        }
    }
    let ev = AgentEvent::ok(
        id,
        type_name,
        Some(serde_json::json!({ "sessionKey": key })),
    );
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

pub(super) async fn handle_resume_session(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    session: String,
) -> bool {
    if ctx.session.is_streaming() {
        let ev = AgentEvent::err(
            id,
            type_name,
            "cannot resume a session while agent is running",
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    if ctx.ephemeral {
        let ev = AgentEvent::err(id, type_name, "cannot resume sessions in ephemeral mode");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let name = session.trim();
    if !crate::interface::cli::is_valid_session_name(name) {
        let ev = AgentEvent::err(
            id,
            type_name,
            "session name must contain only alphanumeric, '-', or '_'",
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let new_key = Session::build_key("cli", name);
    if let Err(err) = persist_current_session(ctx).await {
        let ev = AgentEvent::err(
            id,
            type_name,
            format!("failed to save current session: {err}"),
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let loaded = match ctx.session_store.load(&new_key).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            let ev = AgentEvent::err(id, type_name, format!("session not found: {name}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
        Err(err) => {
            let ev = AgentEvent::err(id, type_name, format!("failed to load session: {err}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    *ctx.session_key = new_key.clone();
    ctx.session.set_session_key(new_key.clone());
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    *ctx.messages = loaded.messages;
    inject_system_prompt(ctx.messages, ctx.system_prompt);
    let ev = AgentEvent::ok(
        id,
        type_name,
        Some(serde_json::json!({
            "session": name,
            "sessionKey": new_key,
            "messageCount": ctx.messages.len(),
        })),
    );
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
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
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

pub(super) async fn handle_follow_up(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
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
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}
pub(super) async fn handle_clear_history(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
) -> bool {
    if ctx.session.is_streaming() {
        let ev = AgentEvent::err(id, tn, "cannot clear history while agent is running");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    clear_conversation(ctx.messages);
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    // Also clear spill store so stale context isn't re-injected (#412).
    if let Some(spill) = ctx.agent.spill_store() {
        if let Err(e) = spill.clear(ctx.session_key).await {
            tracing::warn!("clear_history: failed to clear spill store: {e}");
        }
    }
    let ev = AgentEvent::ok(id, tn, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

pub(super) async fn handle_rewind_to(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
    message_index: usize,
) -> bool {
    if ctx.session.is_streaming() {
        let ev = AgentEvent::err(id, tn, "cannot rewind while agent is running");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }

    if !rewind_to_message_index(ctx.messages, message_index) {
        let ev = AgentEvent::err(id, tn, "invalid rewind target");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }

    ctx.session.clear_usage();
    ctx.session.drain_pending();
    // Clear spill store and remove retained spill references so stale truncated
    // tool output is not recallable or re-injected.
    if let Some(spill) = ctx.agent.spill_store() {
        if let Err(e) = spill.clear(ctx.session_key).await {
            tracing::warn!("rewind_to: failed to clear spill store: {e}");
        }
    }
    if let Err(err) = persist_current_session(ctx).await {
        let ev = AgentEvent::err(id, tn, format!("failed to save rewound session: {err}"));
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }

    let ev = AgentEvent::ok(
        id,
        tn,
        Some(serde_json::json!({
            "rewound": true,
            "messageIndex": message_index,
        })),
    );
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}
use crate::interface::cli::uds_extensions::handle_reload_extensions;
pub(super) async fn dispatch_ext_command(
    cmd: AgentCommand,
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
) -> bool {
    match cmd {
        AgentCommand::ReloadExtensions { .. } => handle_reload_extensions(ctx, id, tn).await,
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
#[path = "uds_dispatch_cov_tests.rs"]
mod cov_tests;
