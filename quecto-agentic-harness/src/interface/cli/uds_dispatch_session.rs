use super::super::uds_session::{
    clear_conversation, resolve_rewind_target, rewind_to_message_index,
};
use super::AgentEvent;
use super::{
    DispatchCtx, emit_event_to_broadcast_or_writer, emit_ledger_advanced, inject_system_prompt,
    remove_injected_system_prompt,
};
use crate::domain::session::{PersistedSubagentRosterEntry, Session, SubagentRestoreReason};

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
    snapshot_subagent_roster_with_restore_reason(registry, SubagentRestoreReason::LegacyUnspecified)
}

pub(crate) fn snapshot_subagent_roster_with_restore_reason(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    restore_reason: SubagentRestoreReason,
) -> Vec<PersistedSubagentRosterEntry> {
    // Killing exit preserves conversation history, not an operational child roster.
    if restore_reason == SubagentRestoreReason::OrdinaryTuiExitStopped {
        return Vec::new();
    }
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
            liveness: entry.persisted_liveness,
            restore_reason,
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

/// Reset the operational child roster for a restored session (#1937).
///
/// A launcher-created child is lifetime-scoped to the harness that launched
/// it, so no persisted record can describe a live child of *this* harness.
/// Restore therefore creates **no** operational row from the persisted
/// roster: no socket probe, no pid compare, no monitor, no readoption. Legacy
/// rows — live, detached, dead or malformed alike — are read only to be
/// ignored; the transcript, workflow and past child messages are restored by
/// the caller, and the master explicitly re-spawns the workers it needs with
/// a fresh identity and launch generation.
pub(crate) fn reset_subagent_roster_on_restore(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    persisted: &[PersistedSubagentRosterEntry],
) {
    let Some(registry) = registry else { return };
    if !persisted.is_empty() {
        tracing::info!(
            ignored_rows = persisted.len(),
            "session restore: persisted subagent rows are history only; no child readopted"
        );
    }
    registry.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Bound on waiting for the current session's owned children to exit after
/// the session-transition teardown: the protocol ACK (5 s) plus the
/// acknowledged-exit wait (10 s) plus the TERM and KILL graces (2 s each).
const SESSION_TRANSITION_EXIT_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// Tear down the current session's children before the harness switches
/// away from it (`resume_session` into another session, `new_session`).
///
/// Interim for #1937 until #1938 migrates the session-transition exit into
/// the application-owned teardown: a launcher-created child cannot be
/// readopted by any later session (restore creates no operational row), so
/// a child left running past this point would be unreachable from every
/// teardown path until the master exits. The existing registry-driven
/// teardown runs — supervisor termination protocol-first for owned children,
/// the environment kill plans for container members — on a blocking thread,
/// then the owned children's exit is awaited within a bounded budget.
pub(crate) async fn tear_down_children_before_session_switch(
    ctx: &mut DispatchCtx<'_>,
    transition: &str,
) {
    let Some(registry) = ctx.subagent_registry.clone() else {
        return;
    };
    let owned: Vec<_> = {
        let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        if entries.is_empty() {
            return;
        }
        tracing::warn!(
            transition,
            live_rows = entries.len(),
            "session switch: tearing down the current session's children; they cannot be readopted"
        );
        entries
            .values()
            .filter_map(|entry| Some((entry.owned_child?, entry.owned_child_supervisor.clone()?)))
            .collect()
    };
    let broadcast_tx = ctx.broadcast_tx.clone();
    let removed = tokio::task::spawn_blocking(move || {
        super::super::uds_delete_all_subagents::delete_all_subagents_from_registry(
            &registry,
            broadcast_tx.as_ref(),
        )
    })
    .await
    .unwrap_or_else(|join_error| {
        tracing::error!(%join_error, "session switch: teardown thread failed");
        0
    });
    let deadline = tokio::time::Instant::now() + SESSION_TRANSITION_EXIT_BUDGET;
    let mut exited = 0usize;
    for (handle, supervisor) in &owned {
        match tokio::time::timeout_at(deadline, supervisor.wait_exit(*handle)).await {
            Ok(_) => exited += 1,
            Err(_elapsed) => {
                tracing::warn!(
                    ?handle,
                    "session switch: owned child did not exit within budget"
                );
            }
        }
    }
    tracing::warn!(
        transition,
        removed,
        owned = owned.len(),
        exited,
        "session switch: current session's children torn down"
    );
}

pub(super) async fn persist_current_session(
    ctx: &mut DispatchCtx<'_>,
) -> Result<(), crate::domain::error::DomainError> {
    persist_current_session_with_options(ctx, SubagentRestoreReason::LegacyUnspecified, false).await
}

pub(super) async fn persist_current_session_with_restore_reason(
    ctx: &mut DispatchCtx<'_>,
    restore_reason: SubagentRestoreReason,
) -> Result<(), crate::domain::error::DomainError> {
    // Explicit detach persistence can cancel a prior killing request; routine
    // saves cannot. Keep this intent in memory, not obsolete historical rows.
    ctx.session.killing_exit = restore_reason == SubagentRestoreReason::OrdinaryTuiExitStopped;
    persist_current_session_with_options(ctx, restore_reason, true).await
}

async fn persist_current_session_with_options(
    ctx: &mut DispatchCtx<'_>,
    restore_reason: SubagentRestoreReason,
    force_full_save: bool,
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
    let restore_reason = match restore_reason {
        SubagentRestoreReason::Unknown => SubagentRestoreReason::LegacyUnspecified,
        reason => reason,
    };
    let restore_reason = if ctx.session.killing_exit {
        SubagentRestoreReason::OrdinaryTuiExitStopped
    } else {
        restore_reason
    };
    let roster =
        snapshot_subagent_roster_with_restore_reason(&ctx.subagent_registry, restore_reason);
    let result = if force_full_save || ctx.durable_prefix_dirty || ctx.subagent_registry.is_some() {
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
    ctx.session.discard_pending();
    tear_down_children_before_session_switch(ctx, "new_session").await;
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
    // The session we are leaving cannot readopt its children later, and no
    // other session can: end them now, before the roster is reset.
    tear_down_children_before_session_switch(ctx, "resume_session").await;
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
    ctx.session.discard_pending();
    let workflow_run = loaded.workflow_run;
    reset_subagent_roster_on_restore(&ctx.subagent_registry, &loaded.subagent_roster);
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
    ctx.session.discard_pending();
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
    ctx.session.discard_pending();
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
