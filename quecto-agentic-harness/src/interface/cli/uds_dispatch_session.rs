//! The session-transition and history-rewrite handlers of the idle
//! dispatch loop: `new_session` (#1862, D7 #1976 — admitted here,
//! transacted by the application), `resume_session` (#1863, D8 #1977 —
//! still interface-orchestrated, over the application's departing-
//! children collaborator), `clear_history` (#1864) and `rewind_to`
//! (#1865). The handlers admit, request and present; no session-key
//! generation, settlement ordering or field-by-field switch lives here.
use super::super::uds_session_switch_runtime::{LoopSessionSwitchRuntime, apply_workflow_run};
use super::super::uds_turn_accounting::LoopTurnAccounting;
use super::AgentEvent;
use super::{
    DispatchCtx, emit_event_to_broadcast_or_writer, emit_ledger_advanced, inject_system_prompt,
};
use crate::application::sessions::dto::{SaveTrigger, SessionTransition};
use crate::application::sessions::ports::FleetSettlement;
use crate::domain::session::{PersistedSubagentRosterEntry, Session};
use crate::domain::session_identity::SessionIdentity;
use crate::interface::cli::protocol::HISTORY_PAGE_SIZE;
use crate::interface::uds::sessions::rewind_conversation_controller::RewindFields;

fn sync_message_count(ctx: &DispatchCtx<'_>) {
    if let Ok(mut state) = ctx.execution_state.lock() {
        state.set_message_count(
            crate::domain::conversation_view::user_visible_messages(
                ctx.messages,
                ctx.system_prompt,
            )
            .len(),
        );
    }
}

pub(super) fn set_workflow_run(
    ctx: &mut DispatchCtx<'_>,
    workflow_run: Option<crate::domain::workflow::WorkflowRunPersisted>,
) {
    apply_workflow_run(ctx.workflow_state.as_ref(), ctx.session, workflow_run);
}

/// Log what a restored session's persisted rows are: history only (#1937).
///
/// A launcher-created child is lifetime-scoped to the harness that launched
/// it, so no persisted record can describe a live child of *this* harness.
/// Restore creates **no** operational row from the persisted roster: no
/// socket probe, no pid compare, no monitor, no readoption. The master
/// explicitly re-spawns the workers it needs with a fresh identity and
/// launch generation.
pub(crate) fn note_persisted_roster_is_history(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    persisted: &[PersistedSubagentRosterEntry],
) {
    if registry.is_some() && !persisted.is_empty() {
        tracing::info!(
            ignored_rows = persisted.len(),
            "session restore: persisted subagent rows are history only; no child readopted"
        );
    }
}

/// The loop's fleet teardown as the settlement port the transitions
/// order (#1938), if the loop has one.
fn fleet_settlement_of(
    fleet: &Option<
        std::sync::Arc<crate::application::subagents::use_cases::TerminateAllDelegatedAgents>,
    >,
) -> Option<&dyn FleetSettlement> {
    fleet.as_deref().map(|fleet| fleet as &dyn FleetSettlement)
}

/// Tell every connected client the survivor set is empty after a roster
/// reset, so no panel keeps a departed child.
fn broadcast_roster_reset(ctx: &DispatchCtx<'_>) {
    if let (Some(registry), Some(tx)) = (&ctx.subagent_registry, &ctx.broadcast_tx) {
        let _ = tx.send(
            crate::infrastructure::tools::subagent_cascade::build_state_changed_event(registry),
        );
    }
}

/// `new_session` (#1862): admitted only while the agent is idle; the
/// transaction itself is the application's. On success the roster reset
/// is broadcast, then the ledger position, then the response — the same
/// on-socket order as before the migration (master broadcast the roster
/// reset before the key was generated and the ledger position before the
/// effort/workflow reset and the retention clear; no event is emitted in
/// between, so the requester and the other clients see the same sequence).
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
    let fresh = ctx.switch.fresh.clone();
    let result = {
        let mut runtime = LoopSessionSwitchRuntime::new(
            ctx.agent,
            ctx.session,
            &ctx.execution_state,
            ctx.workflow_state.as_ref(),
        );
        fresh
            .execute(
                ctx.messages,
                fleet_settlement_of(&ctx.fleet_teardown),
                &mut runtime,
            )
            .await
    };
    let started = match result {
        Ok(started) => started,
        Err(err) => {
            let ev = AgentEvent::err(id, type_name, err.to_string());
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    broadcast_roster_reset(ctx);
    emit_ledger_advanced(ctx, started.ledger).await;
    let ev = AgentEvent::ok(
        id,
        type_name,
        Some(serde_json::json!({ "sessionKey": started.identity.runtime_key() })),
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
    // The departing session's children are settled BEFORE its final save
    // and before the target session is opened (#1938); a child that does
    // not settle keeps the current session.
    let departing = ctx.switch.departing_children.clone();
    if let Err(refused) = departing
        .settle(
            fleet_settlement_of(&ctx.fleet_teardown),
            SessionTransition::Resume,
        )
        .await
    {
        let ev = AgentEvent::err(id, type_name, refused.to_string());
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    if let Err(err) = ctx
        .save_session
        .save(ctx.messages, SaveTrigger::Routine)
        .await
    {
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
    let target = SessionIdentity::from_persisted_key(new_key.as_str());
    if let Err(err) = ctx.session_store.claim(&target) {
        let ev = AgentEvent::err(id, type_name, err.to_string());
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let loaded = match ctx.session_store.load(&target).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            ctx.session_store.release(&target);
            let ev = AgentEvent::err(id, type_name, format!("session not found: {name}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
        Err(err) => {
            ctx.session_store.release(&target);
            let ev = AgentEvent::err(id, type_name, format!("failed to load session: {err}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    // The departing children settled above; what remains are records. The
    // persisted rows of the resumed session are history, never readopted.
    // Replaced BEFORE the session key moves: a refusal here keeps the
    // current session whole and only releases the claim just taken.
    note_persisted_roster_is_history(&ctx.subagent_registry, &loaded.subagent_roster);
    if let Err(refused) = departing.reset_roster(SessionTransition::Resume) {
        ctx.session_store.release(&target);
        let ev = AgentEvent::err(id, type_name, refused.to_string());
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let old_identity = ctx.sessions.active_session.read().await.identity().clone();
    if old_identity != target {
        ctx.session_store.release(&old_identity);
    }
    ctx.session.set_session_key(new_key.clone());
    ctx.agent.set_session_key(target.clone());
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
    broadcast_roster_reset(ctx);
    *ctx.messages = loaded.messages;
    let loaded_len = ctx.messages.len();
    ctx.sessions
        .active_session
        .write()
        .await
        .set_persisted_watermark(loaded_len);
    set_workflow_run(ctx, workflow_run);
    inject_system_prompt(ctx.messages, ctx.system_prompt);
    sync_message_count(ctx);
    // Atomically reset history AND spill namespace to the resumed session so
    // refs from the previous session cannot resolve and collapsed refs from the
    // resumed session never query the previous session key.
    let advance = ctx.sessions.active_session.write().await.switch_to(
        target,
        ctx.agent.spill_store().cloned(),
        ctx.messages,
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

/// `clear_history` (#1864): admitted only while the agent is idle; the
/// transaction itself is the application's. Its ledger position is
/// announced whether the save succeeded or not — the history was replaced
/// either way. The announcement follows the whole transaction (master
/// broadcast it right after the ledger write, before the retention clear
/// and the save): the requester sees the same on-socket order; other
/// clients see `ledger_advanced` once the save has settled.
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
    let clear = ctx.rewrite.clear.clone();
    let mut accounting = LoopTurnAccounting::new(ctx.session, &ctx.execution_state);
    let result = clear.execute(ctx.messages, &mut accounting).await;
    let (advance, ev) = match result {
        Ok(cleared) => (cleared.ledger, AgentEvent::ok(id, tn, None)),
        Err(err) => (err.ledger(), AgentEvent::err(id, tn, err.to_string())),
    };
    emit_ledger_advanced(ctx, advance).await;
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

/// `rewind_to` (#1865): admitted only while the agent is idle; the target
/// mapping is the controller's and the transaction the application's. A
/// refused target announces nothing; a rewound history announces its
/// ledger position whether the save succeeded or not, after the whole
/// transaction (see `handle_clear_history` on the broadcast timing).
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
    let request = RewindFields {
        message_id,
        message_index,
    }
    .into_request(HISTORY_PAGE_SIZE);
    let result = match request {
        Ok(request) => {
            let rewind = ctx.rewrite.rewind.clone();
            let mut accounting = LoopTurnAccounting::new(ctx.session, &ctx.execution_state);
            rewind
                .execute(ctx.messages, &mut accounting, &request)
                .await
        }
        Err(refused) => Err(refused),
    };
    let (advance, ev) = match result {
        Ok(rewound) => (
            Some(rewound.ledger),
            AgentEvent::ok(
                id,
                tn,
                Some(serde_json::json!({
                    "rewound": true,
                    "messageIndex": rewound.message_index,
                })),
            ),
        ),
        Err(err) => (err.ledger(), AgentEvent::err(id, tn, err.to_string())),
    };
    if let Some(advance) = advance {
        emit_ledger_advanced(ctx, advance).await;
    }
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}
