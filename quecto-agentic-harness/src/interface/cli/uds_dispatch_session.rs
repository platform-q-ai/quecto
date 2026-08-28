use super::super::uds_session::{
    clear_conversation, resolve_rewind_target, rewind_to_message_index,
};
use super::AgentEvent;
use super::{
    DispatchCtx, emit_event_to_broadcast_or_writer, emit_ledger_advanced, inject_system_prompt,
    remove_injected_system_prompt,
};
use crate::domain::session::{PersistedSubagentRosterEntry, Session, SubagentLiveness};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentStatus};

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
        let before = serde_json::to_value(engine.snapshot(true)).ok();
        if let Some(run) = workflow_run {
            engine.restore_run(run);
        } else {
            engine.reset();
        }
        let after = serde_json::to_value(engine.snapshot(true)).ok();
        if before != after {
            ctx.session.bump_visible_generation();
        }
    }
}

pub(crate) fn snapshot_subagent_roster(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) -> Vec<PersistedSubagentRosterEntry> {
    let Some(registry) = registry else {
        return Vec::new();
    };
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let mut roster: Vec<_> = entries
        .iter()
        .map(|(key, entry)| PersistedSubagentRosterEntry {
            agent_uuid: entry.agent_uuid.as_str().to_string(),
            display_name: entry.effective_display_name(key).to_string(),
            session_key: entry.agent_uuid.as_str().to_string(),
            socket_path: entry.socket_path.clone(),
            pid: entry.pid,
            liveness: entry.persisted_liveness,
            parent_id: entry.parent_id.clone(),
            read_only: entry.read_only,
            status: Some(entry.status.to_wire_str().to_string()),
            delivered_message_ordinal: entry.delivered_message_ordinal,
            pending_message_reports: entry.pending_message_reports.clone(),
        })
        .collect();
    roster.sort_by(|a, b| a.agent_uuid.cmp(&b.agent_uuid));
    roster
}

pub(crate) fn verify_persisted_live_subagent(entry: &PersistedSubagentRosterEntry) -> bool {
    use std::io::{Read, Write};
    use std::time::Duration;

    if entry.socket_path.as_os_str().is_empty()
        || entry.agent_uuid.is_empty()
        || entry.session_key.is_empty()
    {
        return false;
    }
    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&entry.socket_path) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let request_id = format!("restore-verify-{}", entry.agent_uuid);
    let request = serde_json::json!({
        "type": "get_session_stats",
        "id": request_id,
    })
    .to_string();
    let Ok(len) = u32::try_from(request.len()) else {
        return false;
    };
    if stream.write_all(&len.to_be_bytes()).is_err()
        || stream.write_all(request.as_bytes()).is_err()
        || stream.flush().is_err()
    {
        return false;
    }
    let mut prefix = [0u8; 4];
    if stream.read_exact(&mut prefix).is_err() {
        return false;
    }
    let response_len = u32::from_be_bytes(prefix) as usize;
    if response_len > quecto_line_io::PROTOCOL_FRAME_CAP_BYTES {
        return false;
    }
    let mut payload = vec![0u8; response_len];
    if stream.read_exact(&mut payload).is_err() {
        return false;
    }
    let Ok(response) = serde_json::from_slice::<serde_json::Value>(&payload) else {
        return false;
    };
    response.get("type").and_then(|v| v.as_str()) == Some("response")
        && response.get("id").and_then(|v| v.as_str()) == Some(request_id.as_str())
        && response.get("success").and_then(|v| v.as_bool()) == Some(true)
        && response
            .get("data")
            .and_then(|data| data.get("sessionKey"))
            .and_then(|v| v.as_str())
            == Some(entry.session_key.as_str())
}

pub(crate) fn restore_persisted_subagent_roster(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    roster: Vec<PersistedSubagentRosterEntry>,
) {
    let Some(registry) = registry else { return };
    // Probe sockets BEFORE taking the registry mutex (N1): verify does blocking
    // UDS IO with up to 500ms timeouts, and holding the lock across that stalls
    // concurrent get_subagents / spawn registration during resume.
    let mut restored_entries = Vec::new();
    for persisted in roster {
        if persisted.agent_uuid.is_empty() {
            continue;
        }
        let verified_live = persisted.liveness != SubagentLiveness::Dead
            && verify_persisted_live_subagent(&persisted);
        let restored_pid = if verified_live { persisted.pid } else { 0 };
        let mut entry = SubagentEntry::with_identity(
            crate::domain::ids::AgentUuid::from(persisted.agent_uuid.clone()),
            persisted.display_name.clone(),
            persisted.socket_path.clone(),
            restored_pid,
        );
        entry.status = if verified_live {
            SubagentStatus::Idle
        } else {
            SubagentStatus::Exited
        };
        entry.persisted_liveness = if verified_live {
            SubagentLiveness::Live
        } else {
            SubagentLiveness::Dead
        };
        entry.parent_id = persisted.parent_id;
        entry.read_only = persisted.read_only;
        entry.delivered_message_ordinal = persisted.delivered_message_ordinal;
        entry.pending_message_ordinal = persisted.pending_message_reports.back().map(|p| p.ordinal);
        entry.pending_message_reports = persisted.pending_message_reports;
        restored_entries.push((persisted.agent_uuid, entry));
    }
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries.clear();
    for (agent_uuid, entry) in restored_entries {
        entries.insert(agent_uuid, entry);
    }
}

pub(super) async fn persist_current_session(
    ctx: &mut DispatchCtx<'_>,
) -> Result<(), crate::domain::error::DomainError> {
    if ctx.ephemeral || ctx.session_key.is_empty() {
        return Ok(());
    }
    remove_injected_system_prompt(ctx.messages, ctx.system_prompt);
    crate::infrastructure::persistence::session_store::session_store_ordinals::assign_missing_ordinals_in_place(
        ctx.messages,
    );
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
    let roster = snapshot_subagent_roster(&ctx.subagent_registry);
    let result = if ctx.durable_prefix_dirty || ctx.subagent_registry.is_some() {
        ctx.session_store
            .save(&Session {
                key: ctx.session_key.to_string(),
                messages: ctx.messages.to_vec(),
                workflow_run,
                subagent_roster: roster,
            })
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
    let old_key = ctx.session_key.to_string();
    clear_conversation(ctx.messages);
    sync_message_count(ctx);
    ctx.last_persisted_message_index = 0;
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    if let Some(registry) = &ctx.subagent_registry {
        registry.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    let key = crate::interface::shared::generate_chat_key();
    ctx.session_key.clear();
    ctx.session_key.push_str(&key);
    if old_key != key {
        ctx.session_store.release(&old_key);
    }
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
    let before_effort = ctx.agent.effort();
    ctx.agent.reset_effort_to_default();
    if ctx.agent.effort() != before_effort {
        ctx.session.bump_visible_generation();
    }
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
    let legacy_cli_name = name
        .strip_prefix("cli:")
        .filter(|suffix| crate::interface::cli::is_valid_session_name(suffix));
    if legacy_cli_name.is_none() && !crate::interface::cli::is_valid_session_name(name) {
        let ev = AgentEvent::err(
            id,
            type_name,
            "session name must contain only alphanumeric, '-', or '_'",
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    // The /resume picker selects by full session key (e.g. a `chat-…` user
    // chat or legacy `cli:<name>` row); a typed `/resume <name>` refers to a
    // legacy `cli:<name>` session. Don't re-prefix an already-qualified picker
    // key.
    let new_key = if name.starts_with(crate::domain::session::USER_CHAT_PREFIX)
        || legacy_cli_name.is_some()
    {
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
    // Refuse at open (#1460): resuming a key owned by another live process
    // must fail before any turn runs against it.
    if let Err(err) = ctx.session_store.claim(&new_key) {
        let ev = AgentEvent::err(id, type_name, err.to_string());
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let loaded = match ctx.session_store.load(&new_key).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            ctx.session_store.release(&new_key);
            let ev = AgentEvent::err(id, type_name, format!("session not found: {name}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
        Err(err) => {
            ctx.session_store.release(&new_key);
            let ev = AgentEvent::err(id, type_name, format!("failed to load session: {err}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    let old_key = std::mem::replace(ctx.session_key, new_key.clone());
    if old_key != new_key {
        ctx.session_store.release(&old_key);
    }
    ctx.session.set_session_key(new_key.clone());
    ctx.agent.set_session_key(new_key.clone());
    // Session-scoped effort must not follow the client into the resumed
    // session (#1067).
    let before_effort = ctx.agent.effort();
    ctx.agent.reset_effort_to_default();
    if ctx.agent.effort() != before_effort {
        ctx.session.bump_visible_generation();
    }
    ctx.session.clear_usage();
    ctx.session.drain_pending();
    let workflow_run = loaded.workflow_run;
    restore_persisted_subagent_roster(&ctx.subagent_registry, loaded.subagent_roster);
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
