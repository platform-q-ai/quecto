use super::protocol::AgentCommand;
use super::uds::DispatchCtx;
use super::uds_session::{
    HISTORY_PAGE_SIZE, compute_session_stats_with_usage, messages_page_json_for_id,
    messages_tail_json, position_by_message_id,
};
use crate::domain::ids::{CommandId, MessageId, ToolCallId};
use crate::domain::message::Message;

fn user_visible_messages(messages: &[Message], system_prompt: &str) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| !super::uds_snapshots::is_injected_system_prompt(m, system_prompt))
        .cloned()
        .collect()
}

fn user_visible_message_count(messages: &[Message], system_prompt: &str) -> usize {
    messages
        .iter()
        .filter(|m| !super::uds_snapshots::is_injected_system_prompt(m, system_prompt))
        .count()
}

pub(super) struct GetMessageLookup<'ctx, 'data> {
    pub(super) message_id: MessageId,
    pub(super) tool_call_id: Option<ToolCallId>,
    pub(super) offset: Option<usize>,
    pub(super) limit: Option<usize>,
    pub(super) request_id: Option<CommandId>,
    pub(super) ctx: &'ctx DispatchCtx<'data>,
}

pub(super) fn get_message_response_data(
    req: GetMessageLookup<'_, '_>,
) -> Option<serde_json::Value> {
    let message = &req.ctx.messages[position_by_message_id(req.ctx.messages, &req.message_id)?];
    match req.tool_call_id.as_ref() {
        Some(tool_call_id) => super::uds_session::tool_call_arguments_to_json_range_for_response(
            message,
            tool_call_id.as_str(),
            req.offset,
            req.limit,
            req.request_id.as_ref().map(CommandId::as_str),
        ),
        None => Some(super::uds_session::message_to_json_range_for_response(
            message,
            req.offset,
            req.limit,
            req.request_id.as_ref().map(CommandId::as_str),
        )),
    }
}

pub(super) fn query_response_data(
    cmd: &AgentCommand,
    ctx: &DispatchCtx<'_>,
) -> Option<serde_json::Value> {
    match cmd {
        AgentCommand::GetState { .. } => {
            let workflow = ctx.workflow_state.as_ref().and_then(|ws| {
                ws.lock().ok().map(|engine| {
                    let mut value = serde_json::to_value(engine.snapshot(true)).unwrap_or_default();
                    if let Some(config) = &ctx.workflow_config {
                        value["automation"] = serde_json::json!({
                            "autoContinue": config.auto_continue,
                            "completionNudge": config.completion_nudge,
                        });
                    }
                    value
                })
            });
            // #1067: `SessionState` itself carries the session's effective
            // effort (the level string when set, an explicit null when unset)
            // plus the provider's valid vocabulary, so the live-query and
            // busy-connect snapshot paths serve the same `get_state` shape.
            let mut state = ctx.session.state_snapshot(
                user_visible_message_count(ctx.messages, ctx.system_prompt),
                workflow,
                ctx.agent.max_context_tokens(),
                ctx.agent.effort().map(|l| l.as_str().to_string()),
            );
            if let Ok(mut execution) = ctx.execution_state.lock() {
                if ctx.session.is_streaming() {
                    state.message_count = execution.message_count();
                } else {
                    execution.set_hidden_message_count(
                        ctx.messages.len().saturating_sub(state.message_count),
                    );
                    execution.set_message_count(state.message_count);
                }
                state.execution = Some(execution.snapshot());
            }
            Some(serde_json::to_value(&state).unwrap_or_default())
        }
        AgentCommand::GetMessages { count, before, .. } => {
            let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
            let before = before.as_deref().map(MessageId::from);
            Some(messages_page_json_for_id(
                &visible_messages,
                count.unwrap_or(HISTORY_PAGE_SIZE),
                before.as_ref(),
            ))
        }
        AgentCommand::GetMessagesTail { count, .. } => {
            let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
            Some(messages_tail_json(&visible_messages, *count))
        }
        AgentCommand::GetSessionStats { .. } => {
            let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
            let stats = compute_session_stats_with_usage(
                ctx.session_key,
                &visible_messages,
                ctx.session.usage_snapshot(),
                ctx.session.context_tokens(),
                ctx.agent.max_context_tokens(),
            );
            Some(serde_json::to_value(&stats).unwrap_or_default())
        }
        AgentCommand::GetToolCatalogue { .. } => Some(serde_json::json!({
            "tools": ctx.agent.tool_catalogue_entries(),
        })),
        AgentCommand::ListModels { .. } => Some(super::uds_models::list_models_response(ctx)),
        AgentCommand::GetSubagents { .. } => {
            let list = super::protocol::build_subagent_info_list(&ctx.subagent_registry);
            Some(serde_json::json!({ "subagents": list }))
        }
        AgentCommand::DeleteAllSubagents { .. } => {
            Some(super::uds_delete_all_subagents::response_data(ctx))
        }
        // #1060: on-demand single-message lookup by stable id (busy-path safe).
        // Miss returns None so dispatch_fieldless_command emits a structured error.
        AgentCommand::GetMessage {
            id,
            message_id,
            tool_call_id,
            offset,
            limit,
            ..
        } => get_message_response_data(GetMessageLookup {
            message_id: MessageId::from(message_id.as_str()),
            tool_call_id: tool_call_id.as_deref().map(ToolCallId::from),
            offset: *offset,
            limit: *limit,
            request_id: id.as_deref().map(CommandId::from),
            ctx,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "uds_query_tests.rs"]
mod tests;
#[cfg(test)]
mod cov2_tests {
    use super::{query_response_data, tests::Fx};
    use crate::interface::cli::protocol::AgentCommand;

    #[test]
    fn get_state_query_ignores_poisoned_workflow_lock() {
        let mut fx = Fx::new();
        let state: std::sync::Arc<std::sync::Mutex<crate::domain::workflow::WorkflowEngine>> =
            std::sync::Arc::new(std::sync::Mutex::new(
                crate::domain::workflow::WorkflowEngine::new(
                    crate::domain::workflow::WorkflowConfig {
                        auto_continue: true,
                        completion_nudge: true,
                        selector_prompt: None,
                        dir: None,
                        templates: vec![],
                    },
                    false,
                )
                .unwrap(),
            ));
        let poisoned = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison workflow lock");
        })
        .join();

        let mut ctx = fx.ctx();
        ctx.workflow_state = Some(state);
        let value = query_response_data(
            &AgentCommand::GetState {
                id: None,
                agent_id: None,
            },
            &ctx,
        )
        .unwrap();
        assert!(value.get("workflow").is_none());
        assert_eq!(value["model"], "stub");
    }
}
