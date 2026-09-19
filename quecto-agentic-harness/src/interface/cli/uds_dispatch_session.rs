//! The session-transition and history-rewrite handlers of the idle
//! dispatch loop: `new_session` (#1862, D7 #1976), `resume_session`
//! (#1863, D8 #1977), `clear_history` (#1864) and `rewind_to` (#1865) —
//! each admitted here and transacted by the application. The handlers
//! admit, request and present; no session-key generation or admission,
//! claim sequencing, settlement ordering or field-by-field switch lives
//! here.
use super::super::uds_session_switch_runtime::LoopSessionSwitchRuntime;
use super::super::uds_turn_accounting::LoopTurnAccounting;
use super::AgentEvent;
use super::{DispatchCtx, emit_event_to_broadcast_or_writer, emit_ledger_advanced};
use crate::application::sessions::dto::ResumeOutcome;
use crate::application::sessions::ports::FleetSettlement;
use crate::interface::cli::protocol::HISTORY_PAGE_SIZE;
use crate::interface::uds::sessions::resume_session_controller::ResumeFields;
use crate::interface::uds::sessions::rewind_conversation_controller::RewindFields;
#[path = "uds_dispatch_resume.rs"]
mod uds_dispatch_resume;

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
            ctx.catalogue.effort.clone(),
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

/// `resume_session` (#1863, D8 #1977, #2011): the controller maps the wire
/// fields and the transaction is the application's — admission, the typed
/// decision or refusal, settlement, save, claim, load, restore and switch.
/// The `busy` guard is defensive and unreachable today: turns and dispatch
/// share one task, so a mid-turn request is queued and RUNS after the turn.
/// On a restore the roster reset is broadcast, then the ledger position, then
/// the response. A cancel, a decision and a refusal announce only their answer.
pub(super) async fn handle_resume_session(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    fields: ResumeFields,
) -> bool {
    if ctx.session.is_streaming() {
        let busy = crate::application::sessions::dto::ResumeSavedSessionError::Busy;
        let ev = uds_dispatch_resume::refusal_event(id, type_name, &busy);
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let resume = ctx.switch.resume.clone();
    let result = match fields.into_request() {
        Ok(request) => {
            let mut runtime = LoopSessionSwitchRuntime::new(
                ctx.agent,
                ctx.session,
                &ctx.execution_state,
                ctx.workflow_state.as_ref(),
                ctx.catalogue.effort.clone(),
            );
            let fleet = fleet_settlement_of(&ctx.fleet_teardown);
            resume
                .execute(&request, ctx.messages, fleet, &mut runtime)
                .await
        }
        Err(refused) => Err(refused),
    };
    let ev = match &result {
        Ok(outcome) => uds_dispatch_resume::outcome_event(id, type_name, outcome),
        Err(refusal) => uds_dispatch_resume::refusal_event(id, type_name, refusal),
    };
    if let Ok(ResumeOutcome::Resumed(resumed)) = result {
        broadcast_roster_reset(ctx);
        emit_ledger_advanced(ctx, resumed.ledger).await;
    }
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

/// Discovery through the injected handle, never a store; wire scope mapped here.
pub(super) async fn handle_list_sessions(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    scope: crate::interface::cli::protocol::SessionListScopeCommand,
) {
    let requested = crate::application::sessions::dto::SessionListScope::from(scope);
    let event = match ctx.discovery.list(requested).await {
        Ok(result) => {
            let data = super::uds_dispatch_query::discovery_json(&result, scope);
            AgentEvent::ok(id, type_name, Some(data))
        }
        Err(error) => AgentEvent::err(id, type_name, error.to_string()),
    };
    super::emit_response_or_frame_limit_error(ctx, id, type_name, event).await;
}
