use super::super::uds_session::{
    clear_conversation, resolve_rewind_target, rewind_to_message_index,
};
use super::AgentEvent;
use super::{
    DispatchCtx, emit_event_to_broadcast_or_writer, emit_ledger_advanced, inject_system_prompt,
    remove_injected_system_prompt,
};
use crate::domain::session::Session;

fn sync_message_count(ctx: &DispatchCtx<'_>) {
    if let Ok(mut state) = ctx.execution_state.lock() {
        state.set_message_count(
            super::super::uds_snapshots::user_visible_messages(ctx.messages, ctx.system_prompt)
                .len(),
        );
    }
}

pub(super) fn set_workflow_run(
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
    sync_message_count(ctx);
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
    if let Some(spill) = ctx.agent.spill_store()
        && let Err(e) = spill.clear(ctx.session_key).await
    {
        tracing::warn!("new_session: failed to clear spill store: {e}");
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
    sync_message_count(ctx);
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
    sync_message_count(ctx);
    let advance = ctx.conversation_snapshot.write().await.clear();
    emit_ledger_advanced(ctx, advance).await;
    ctx.last_persisted_message_index = 0;
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    // Also clear spill store so stale context isn't re-injected (#412).
    if let Some(spill) = ctx.agent.spill_store()
        && let Err(e) = spill.clear(ctx.session_key).await
    {
        tracing::warn!("clear_history: failed to clear spill store: {e}");
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
    sync_message_count(ctx);

    ctx.last_persisted_message_index = 0;
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    // Reset the stable-ref ledger to the truncated conversation so a
    // rewound-away message is no longer recoverable via get_message (same
    // intent as the spill clear below — truncated content must not be
    // recallable) (#1060 review r4).
    let advance = ctx
        .conversation_snapshot
        .write()
        .await
        .reset_to(ctx.messages);
    emit_ledger_advanced(ctx, advance).await;
    // Clear spill store and remove retained spill references so stale truncated
    // tool output is not recallable or re-injected.
    if let Some(spill) = ctx.agent.spill_store()
        && let Err(e) = spill.clear(ctx.session_key).await
    {
        tracing::warn!("rewind_to: failed to clear spill store: {e}");
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
