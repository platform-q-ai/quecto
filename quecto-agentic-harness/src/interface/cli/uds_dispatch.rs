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
};
#[cfg(not(test))]
use super::uds_dispatch_session::{handle_new_session, handle_resume_session, handle_rewind_to};
use super::{AgentCommand, AgentEvent};
use super::{DispatchCtx, emit_event_to_broadcast_or_writer};
use crate::application::sessions::dto::SaveTrigger;
use crate::domain::session::SubagentRestoreReason;
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyOperation, ToolPolicyRequest,
};
use crate::interface::cli::protocol::{
    ToolPolicyApplyModeCommand, ToolPolicyMutationCommand, ToolPolicyOperationCommand,
};
use crate::interface::cli::uds_ext_protocol;
use crate::interface::uds::sessions::resume_session_controller::ResumeFields;

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
            persist,
            ..
        } => {
            handle_set_model(
                SetModelArgs {
                    id,
                    type_name,
                    model,
                    provider,
                    model_id,
                    persist,
                },
                ctx,
            )
            .await
        }
        AgentCommand::SetEffort {
            effort, persist, ..
        } => handle_set_effort(ctx, id.as_deref(), &type_name, &effort, persist.as_deref()).await,
        AgentCommand::SetToolPolicy {
            mutations,
            mode,
            operation,
            unlisted_scope,
            persist,
            ..
        } => {
            handle_set_tool_policy(
                ctx,
                id.as_deref(),
                &type_name,
                SetToolPolicyCommandParts {
                    mutations,
                    mode,
                    operation,
                    unlisted_scope,
                    persist,
                },
            )
            .await
        }
        AgentCommand::Reload { .. } => {
            // Reload runtime configuration (#1849): forced, rebuilt off the
            // runtime; the reply is the presenter's rendering of the outcome.
            let outcome = super::super::uds_dispatch_reload::force_reload(ctx).await;
            let ev = match crate::interface::uds::catalogue::reload_presenter::render(&outcome) {
                Ok(()) => AgentEvent::ok(id.as_deref(), &type_name, None),
                Err(error) => AgentEvent::err(id.as_deref(), &type_name, error),
            };
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            false
        }
        AgentCommand::NewSession { .. } => handle_new_session(ctx, id.as_deref(), &type_name).await,
        AgentCommand::SearchSessionMetadata {
            query,
            scope,
            generation,
            limit,
            ..
        } => {
            let fields = uds_dispatch_search::SearchFields {
                query,
                scope,
                generation,
                limit,
            };
            uds_dispatch_search::handle(ctx, id.as_deref(), &type_name, fields).await
        }
        AgentCommand::ResumeSession {
            session,
            action,
            expected_home_version,
            ..
        } => {
            let fields = ResumeFields {
                session,
                action: action.map(|action| action.0),
                expected_home_version,
            };
            handle_resume_session(ctx, id.as_deref(), &type_name, fields).await
        }
        AgentCommand::PersistSession { restore_reason, .. } => {
            let reason = match restore_reason.as_deref() {
                Some(SubagentRestoreReason::ORDINARY_TUI_EXIT_STOPPED_WIRE) => {
                    SubagentRestoreReason::OrdinaryTuiExitStopped
                }
                _ => SubagentRestoreReason::LegacyUnspecified,
            };
            let saved = ctx
                .save_session
                .save(
                    ctx.messages,
                    SaveTrigger::Explicit {
                        restore_reason: reason,
                    },
                )
                .await;
            let ev = match saved {
                Ok(_) => AgentEvent::ok(id.as_deref(), &type_name, None),
                Err(err) => AgentEvent::err(id.as_deref(), &type_name, err.to_string()),
            };
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            false
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
        | AgentCommand::RefreshModels { .. }
        | AgentCommand::GetToolCatalogue { .. }
        | AgentCommand::GetSubagents { .. }
        | AgentCommand::GetMessage { .. }
        | AgentCommand::GetState { .. }
        | AgentCommand::GetReport { .. }
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

pub(super) struct SetToolPolicyCommandParts {
    pub(super) mutations: Vec<ToolPolicyMutationCommand>,
    pub(super) mode: ToolPolicyApplyModeCommand,
    pub(super) operation: ToolPolicyOperationCommand,
    pub(super) unlisted_scope: Option<crate::domain::tool_descriptor::ProfileAvailabilityScope>,
    pub(super) persist: bool,
}

pub(super) async fn handle_set_tool_policy(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    command: SetToolPolicyCommandParts,
) -> bool {
    let SetToolPolicyCommandParts {
        mutations,
        mode,
        operation,
        unlisted_scope,
        persist,
    } = command;
    let mut domain_mutations = Vec::with_capacity(mutations.len());
    for mutation in mutations {
        let Some(name) = mutation.tool_id.or(mutation.name) else {
            let ev = AgentEvent::err(
                id,
                type_name,
                "tool policy mutation requires name or toolId",
            );
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        };
        let reason = mutation
            .reason
            .unwrap_or_else(|| "set_tool_policy".to_string());
        domain_mutations.push(ToolPolicyMutation::set_scope(name, mutation.scope, reason));
    }

    let apply_mode = match mode {
        ToolPolicyApplyModeCommand::ImmediateIfIdle => ToolPolicyApplyMode::ImmediateIfIdle,
        ToolPolicyApplyModeCommand::AtNextTurnBoundary => ToolPolicyApplyMode::AtNextTurnBoundary,
    };
    let request = match operation {
        ToolPolicyOperationCommand::Patch => ToolPolicyRequest {
            operation: ToolPolicyOperation::Patch,
            mutations: domain_mutations,
            unlisted_scope: None,
            correlation_id: id.map(str::to_string),
            persist,
        },
        ToolPolicyOperationCommand::Replace => {
            let Some(scope) = unlisted_scope else {
                let ev =
                    AgentEvent::err(id, type_name, "replace tool policy requires unlistedScope");
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                return false;
            };
            let mut request = ToolPolicyRequest::replace(domain_mutations, scope);
            request.correlation_id = id.map(str::to_string);
            request.persist = persist;
            request
        }
    };
    let reconciliation = ctx.agent.request_tool_policy(request, apply_mode);
    let data = match reconciliation {
        Some(reconciliation) => {
            if persist
                && reconciliation.results.iter().any(|result| {
                    result.status
                        == crate::domain::tool::ToolPolicyMutationStatus::PersistenceFailed
                })
            {
                let ev = AgentEvent::err(
                    id,
                    type_name,
                    reconciliation
                        .results
                        .iter()
                        .find(|result| {
                            result.status
                                == crate::domain::tool::ToolPolicyMutationStatus::PersistenceFailed
                        })
                        .map(|result| result.reason.as_str())
                        .unwrap_or("persisted tool policy update failed"),
                );
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                return false;
            }
            serde_json::to_value(&reconciliation).unwrap_or_default()
        }
        None => serde_json::json!({
            "mode": "atNextTurnBoundary",
            "queued": true,
            "results": [],
        }),
    };
    let ev = AgentEvent::ok(id, type_name, Some(data));
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

pub(super) async fn handle_steer(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
    // Acceptance is independent of the provider outcome. Keep the idle
    // control receipt even if the ensuing turn fails before completion.
    if !ctx.session.is_streaming() {
        let event = AgentEvent::ok(
            id,
            type_name,
            Some(serde_json::json!({"status":"accepted"})),
        );
        emit_event_to_broadcast_or_writer(ctx, &event).await;
    }
    super::handle_prompt(
        ctx,
        super::PromptCommand {
            id: id.map(str::to_owned),
            type_name: type_name.into(),
            message,
            streaming_behavior: Some(super::StreamingBehavior::Steer),
        },
    )
    .await
}

pub(super) async fn handle_follow_up(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
    // A retained follow-up cannot revoke a later admitted steer. Only that
    // steer's handler (or explicit abort) releases the priority gate.
    let retained = super::pending::queue_prompt(ctx, id, type_name, message, false).await;
    if retained && !ctx.session.is_streaming() {
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
    let before = (config.auto_continue, config.completion_nudge);
    if let Some(value) = auto_continue {
        config.auto_continue = value;
    }
    if let Some(value) = completion_nudge {
        config.completion_nudge = value;
    }
    if before != (config.auto_continue, config.completion_nudge) {
        ctx.session.bump_visible_generation();
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
    ctx.session.discard_pending();
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

#[path = "uds_dispatch_search.rs"]
mod uds_dispatch_search;

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
#[path = "uds_dispatch_fixture_tests.rs"]
pub(super) mod fixture_tests;
#[cfg(test)]
#[path = "uds_dispatch_1060_lifecycle_tests.rs"]
mod lifecycle_1060_tests;
#[cfg(test)]
#[path = "uds_dispatch_masked_pruning_tests.rs"]
mod masked_pruning_tests;
#[cfg(test)]
#[path = "uds_dispatch_ordinal_tests.rs"]
mod ordinal_tests;
#[cfg(test)]
#[path = "uds_dispatch_policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "uds_dispatch_resume_e2e_tests.rs"]
mod resume_e2e_tests;
#[cfg(test)]
#[path = "uds_dispatch_resume_persist_tests.rs"]
mod resume_persist_tests;
#[cfg(test)]
#[path = "uds_dispatch_resume_picker_tests.rs"]
mod resume_picker_tests;
#[cfg(test)]
#[path = "uds_dispatch_1093_tests.rs"]
mod tests_1093;
#[cfg(test)]
#[path = "uds_dispatch_1586_tests.rs"]
mod tests_1586;
#[cfg(test)]
#[path = "uds_dispatch_843_tests.rs"]
mod tests_843;
#[cfg(test)]
#[path = "uds_dispatch_843_targeted_tests.rs"]
mod tests_843_targeted;
#[cfg(test)]
#[path = "uds_dispatch_tool_catalogue_tests.rs"]
mod tool_catalogue_tests;
