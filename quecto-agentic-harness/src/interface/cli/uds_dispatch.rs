use super::uds_dispatch_get_message_forward::{ForwardGetMessage, forward_subagent_get_message};
use super::*;
use crate::interface::cli::uds_ext_protocol;

pub(crate) async fn dispatch_command(cmd: AgentCommand, ctx: &mut DispatchCtx<'_>) -> bool {
    // Agent-targeted get_messages/tail forwards to the named sub-agent's own UDS
    // (with OR without a count) before the local fast path — which ignores
    // agent_id and would return the parent's history — can answer it
    // (#795/#837/#843).
    if let AgentCommand::GetMessages {
        count,
        before,
        agent_id: Some(agent_id),
        id,
    } = &cmd
    {
        let tn = cmd.type_name();
        let ev = forward_subagent_get_messages(
            ctx,
            id.as_deref(),
            tn,
            agent_id,
            *count,
            before.as_deref(),
        )
        .await;
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    if let AgentCommand::GetMessagesTail {
        count,
        agent_id: Some(agent_id),
        id,
    } = &cmd
    {
        let tn = cmd.type_name();
        let ev =
            forward_subagent_get_messages(ctx, id.as_deref(), tn, agent_id, Some(*count), None)
                .await;
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    if let AgentCommand::Sync {
        epoch,
        since_rev,
        agent_id: Some(agent_id),
        id,
    } = &cmd
    {
        let tn = cmd.type_name();
        let ev = super::uds_dispatch_sync_forward::forward_subagent_sync(
            ctx,
            id.as_deref(),
            tn,
            agent_id,
            *epoch,
            *since_rev,
        )
        .await;
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    // #1060: agent-targeted get_message — resolve the message on the child session.
    if let AgentCommand::GetMessage {
        message_id,
        agent_id: Some(agent_id),
        tool_call_id,
        offset,
        limit,
        id,
    } = &cmd
    {
        let tn = cmd.type_name();
        let ev = forward_subagent_get_message(
            ctx,
            id.as_deref(),
            tn,
            ForwardGetMessage {
                agent_id,
                message_id,
                tool_call_id: tool_call_id.as_deref(),
                offset: *offset,
                limit: *limit,
            },
        )
        .await;
        // The child sizes the page with the forwarded correlation id, but
        // still guard the final parent envelope so no response can disappear
        // through the generic oversized-event drop path.
        super::emit_response_or_frame_limit_error(ctx, id.as_deref(), tn, ev).await;
        return false;
    }
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
        AgentCommand::ReloadExtensions { .. }
        | AgentCommand::RegisterTools { .. }
        | AgentCommand::UnregisterTools { .. }
        | AgentCommand::ToolResult { .. } => {
            dispatch_ext_command(cmd, ctx, id.as_deref(), &type_name).await
        }
        // Exhaustive: variants handled above (queries by dispatch_fieldless_command,
        // agent-targeted tails by the early intercept at the top).
        AgentCommand::ClearHistory { .. }
        | AgentCommand::ListModels { .. }
        | AgentCommand::GetExtensions { .. }
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

/// Forward a `get_messages` request to a spawned sub-agent and wrap its
/// response as this command's reply (#795/#837/#843). With `count: Some(n)` the
/// child returns its last-N tail; with `None` it returns its full history.
///
/// Reuses the shared sub-agent socket lookup and UDS round-trip helpers rather
/// than re-deriving anything locally — the sub-agent answers from its own
/// conversation history. The child mapping always sends `get_messages` (the
/// optional `count` selects tail-vs-full); "tail" is an implementation detail of
/// that mapping, not this function's contract — hence the name covers both.
async fn forward_subagent_get_messages(
    ctx: &DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
    agent_id: &str,
    count: Option<usize>,
    before: Option<&str>,
) -> AgentEvent {
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, lookup_subagent_socket, send_subagent_uds_command_with_timeout,
    };
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id, tn, "no sub-agent registry available");
    };
    let socket_path = match lookup_subagent_socket(registry, agent_id) {
        Ok(path) => path,
        Err(e) => return AgentEvent::err(id, tn, e),
    };
    // Omit `count` entirely when None so the child returns its FULL history; a
    // present count requests just the tail (#843).
    let mut cmd = serde_json::json!({ "type": "get_messages" });
    if let Some(count) = count {
        cmd["count"] = serde_json::json!(count);
    }
    if let Some(before) = before {
        cmd["before"] = serde_json::json!(before);
    }
    let cmd = cmd.to_string();
    // This forward is awaited inline in the single shared dispatch loop, so it
    // uses the short interactive timeout — a slow/hung sub-agent must not stall
    // steer/abort/new-message for any client for the full agent_cmd 300s (#795).
    match send_subagent_uds_command_with_timeout(&socket_path, &cmd, INSPECTOR_RESPONSE_TIMEOUT)
        .await
    {
        // Preserve child failures instead of rewriting them as parent success.
        Ok(line) => match super::uds_forward_response::parse_forwarded_get_messages(&line) {
            Ok(data) => AgentEvent::ok(id, tn, Some(data)),
            Err(error) => AgentEvent::err(id, tn, error),
        },
        Err(e) => AgentEvent::err(id, tn, e.to_string()),
    }
}

fn set_workflow_run(
    ctx: &mut DispatchCtx<'_>,
    workflow_run: Option<crate::domain::workflow::WorkflowRunPersisted>,
) {
    if let Some(workflow) = &ctx.workflow_state
        && let Ok(mut engine) = workflow.lock()
    {
        if let Some(run) = workflow_run {
            engine.restore_run(run);
        } else {
            engine.reset();
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
    // #1072/#1073 review: drain the agent's durable-prefix dirty latch HERE,
    // at the single sink that acts on it, instead of at every agent-running
    // dispatch site. The latch is sticky and outcome-independent (Success,
    // Error, Cancelled), so any run that mutated pre-existing history —
    // including drained steer follow-ups, workflow auto-continue turns and
    // coalesced sub-agent notes — is reconciled by the next persist, and a
    // future dispatch path cannot forget to propagate it.
    ctx.durable_prefix_dirty |= ctx.agent.take_durable_prefix_dirty();
    if ctx.messages.len() < ctx.last_persisted_message_index {
        ctx.last_persisted_message_index = 0;
    }
    let workflow_run = ctx
        .workflow_state
        .as_ref()
        .and_then(|ws| ws.lock().ok().and_then(|engine| engine.persisted_run()));
    let result = if ctx.durable_prefix_dirty {
        ctx.session_store
            .save_delta(ctx.session_key, ctx.messages, 0, workflow_run)
            .await
    } else {
        ctx.session_store
            .save_clean_delta(
                ctx.session_key,
                ctx.messages,
                ctx.last_persisted_message_index,
                workflow_run,
            )
            .await
    };
    let persisted_len = ctx.messages.len();
    inject_system_prompt(ctx.messages, ctx.system_prompt);
    if result.is_ok() {
        ctx.last_persisted_message_index = persisted_len;
        ctx.durable_prefix_dirty = false;
    }
    result
}

/// Switch the session's reasoning effort at runtime (#1067).
///
/// The level is validated against the ACTIVE model's provider vocabulary
/// (OpenAI-shaped: none/low/medium/high/xhigh; Anthropic: low/medium/high/max)
/// — never the cross-provider union — so a level another provider accepts is
/// rejected here, listing exactly the levels this session can use. On
/// rejection the previous setting stays in effect.
pub(super) async fn handle_set_effort(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    effort: &str,
) -> bool {
    use crate::domain::provider::EffortLevel;
    let valid = EffortLevel::levels_for_model(ctx.session.model());
    let ev = match EffortLevel::parse(effort).filter(|level| valid.contains(level)) {
        Some(level) => {
            ctx.agent.set_effort(level);
            tracing::debug!(effort = level.as_str(), "UDS: effort switched");
            AgentEvent::ok(
                id,
                type_name,
                Some(serde_json::json!({ "effort": level.as_str() })),
            )
        }
        None => AgentEvent::err(
            id,
            type_name,
            format!(
                "invalid effort level \"{effort}\"; valid levels: {}",
                EffortLevel::levels_list(valid)
            ),
        ),
    };
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
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
    if let Err(err) = persist_current_session(ctx).await {
        let ev = AgentEvent::err(
            id,
            type_name,
            format!("failed to save current session: {err}"),
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    clear_conversation(ctx.messages);
    ctx.last_persisted_message_index = 0;
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    let key = crate::interface::shared::generate_chat_key();
    ctx.session_key.clear();
    ctx.session_key.push_str(&key);
    ctx.session.set_session_key(key.clone());
    ctx.agent.set_session_key(key.clone());
    // Replace history and its spill namespace in one snapshot write so busy
    // readers can observe neither old refs under the new key nor new history
    // under the old key.
    let advance = ctx
        .conversation_snapshot
        .write()
        .await
        .reset_to_with_spill_store(ctx.messages, ctx.agent.spill_store().cloned(), key.clone());
    emit_ledger_advanced(ctx, advance).await;
    // Session-scoped effort must not leak into the fresh session (#1067).
    ctx.agent.reset_effort_to_default();
    set_workflow_run(ctx, None);
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
    // The /resume picker selects by full session key (e.g. a `chat-…` user
    // chat); a typed `/resume <name>` refers to a legacy `cli:<name>` session.
    // Don't re-prefix an already-qualified user-chat key.
    let new_key = if name.starts_with(crate::domain::session::USER_CHAT_PREFIX) {
        name.to_string()
    } else {
        Session::build_key("cli", name)
    };
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
    ctx.agent.set_session_key(new_key.clone());
    // Session-scoped effort must not follow the client into the resumed
    // session (#1067).
    ctx.agent.reset_effort_to_default();
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    let workflow_run = loaded.workflow_run;
    *ctx.messages = loaded.messages;
    ctx.last_persisted_message_index = ctx.messages.len();
    set_workflow_run(ctx, workflow_run);
    inject_system_prompt(ctx.messages, ctx.system_prompt);
    // Atomically reset history AND spill namespace to the resumed session so
    // refs from the previous session cannot resolve and collapsed refs from the
    // resumed session never query the previous session key.
    let advance = ctx
        .conversation_snapshot
        .write()
        .await
        .reset_to_with_spill_store(
            ctx.messages,
            ctx.agent.spill_store().cloned(),
            new_key.clone(),
        );
    emit_ledger_advanced(ctx, advance).await;
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
    // `"type":"steer"` substring match in the reader, so the auto-continue nudge
    // is not permanently suppressed (#896 AC3). See `handle_prompt`.
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
    let advance = ctx.conversation_snapshot.write().await.clear();
    emit_ledger_advanced(ctx, advance).await;
    ctx.last_persisted_message_index = 0;
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    // Also clear spill store so stale context isn't re-injected (#412).
    if let Some(spill) = ctx.agent.spill_store() {
        if let Err(e) = spill.clear(ctx.session_key).await {
            tracing::warn!("clear_history: failed to clear spill store: {e}");
        }
    }
    if let Err(err) = persist_current_session(ctx).await {
        let ev = AgentEvent::err(id, tn, format!("failed to save cleared session: {err}"));
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let ev = AgentEvent::ok(id, tn, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

pub(super) async fn handle_rewind_to(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
    message_index: Option<usize>,
    message_id: Option<String>,
) -> bool {
    if ctx.session.is_streaming() {
        let ev = AgentEvent::err(id, tn, "cannot rewind while agent is running");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }

    // Prefer the stable `messageId`, resolved against the full conversation (#1061).
    let message_index =
        match resolve_rewind_target(ctx.messages, message_id.as_deref(), message_index) {
            Ok(idx) => idx,
            Err(msg) => {
                let ev = AgentEvent::err(id, tn, msg);
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                return false;
            }
        };

    if !rewind_to_message_index(ctx.messages, message_index) {
        let ev = AgentEvent::err(id, tn, "invalid rewind target");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }

    ctx.last_persisted_message_index = 0;
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    // Reset the stable-ref ledger to the truncated conversation so a rewound-away
    // message is no longer recoverable via get_message (same intent as the spill
    // clear below — truncated content must not be recallable) (#1060 review r4).
    let advance = ctx
        .conversation_snapshot
        .write()
        .await
        .reset_to(ctx.messages);
    emit_ledger_advanced(ctx, advance).await;
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
#[path = "uds_dispatch_1093_tests.rs"]
mod tests_1093;

#[cfg(test)]
#[path = "uds_dispatch_843_tests.rs"]
mod tests_843;
