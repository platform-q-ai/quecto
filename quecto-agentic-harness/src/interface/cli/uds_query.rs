use super::protocol::AgentCommand;
use super::uds::DispatchCtx;
use super::uds_session::{HISTORY_PAGE_SIZE, compute_session_stats_with_usage, history_page_json};
use crate::domain::conversation_view::user_visible_messages;

#[cfg(test)]
pub(super) fn query_response_data(
    cmd: &AgentCommand,
    ctx: &DispatchCtx<'_>,
) -> Option<serde_json::Value> {
    let session_key = ctx
        .sessions
        .active_session
        .try_read()
        .expect("the test rig holds no other lock")
        .identity()
        .runtime_key()
        .to_string();
    query_response_data_result(cmd, ctx, &session_key)
        .ok()
        .flatten()
}

/// The idle query projections; `session_key` is the active session's key
/// (D10 #1979), read once by the async dispatcher and presented here.
pub(super) fn query_response_data_result(
    cmd: &AgentCommand,
    ctx: &DispatchCtx<'_>,
    session_key: &str,
) -> Result<Option<serde_json::Value>, String> {
    let data = match cmd {
        AgentCommand::GetState { since, .. } => {
            let (workflow, workflow_revision) =
                ctx.workflow_state.as_ref().map_or((None, 0), |ws| {
                    let engine = ws.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    let revision = engine.revision();
                    let mut value = serde_json::to_value(engine.snapshot(true)).unwrap_or_default();
                    if let Some(config) = &ctx.workflow_config {
                        value["automation"] = serde_json::json!({
                            "autoContinue": config.auto_continue,
                            "completionNudge": config.completion_nudge,
                        });
                    }
                    (Some(value), revision)
                });
            // #1067: `SessionState` itself carries the session's effective
            // effort (the level string when set, an explicit null when unset)
            // plus the provider's valid vocabulary, so the live-query and
            // busy-connect snapshot paths serve the same `get_state` shape.
            let mut state = ctx.session.state_snapshot(
                session_key,
                user_visible_messages(ctx.messages, ctx.system_prompt).len(),
                workflow,
                ctx.agent.max_context_tokens(),
                super::catalogue_handles::effort_view(ctx),
            );
            {
                // ExecutionState is the sole owner of the public cursor. Recover
                // poisoned mutex data rather than publishing the raw session
                // generation, which may be behind a cursor already observed.
                let mut execution = ctx
                    .execution_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.generation =
                    execution.observe_visible_revisions(state.generation, workflow_revision);
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
            Some(super::uds_state_projection::slim_state_response_data(
                &state, *since,
            ))
        }
        // Read conversation history (#1856, #1971): the composed history
        // owner selects the window (and refuses a stale/unknown cursor as a
        // structured error, never a silent restart at the newest page); this
        // edge presents it under the frame budget.
        AgentCommand::GetMessages { count, before, .. } => Some(history_page_json(
            ctx.sessions
                .read_history
                .page(
                    ctx.messages,
                    ctx.system_prompt,
                    count.unwrap_or(HISTORY_PAGE_SIZE),
                    before.as_deref(),
                )
                .map_err(|err| err.to_string())?,
        )),
        AgentCommand::GetMessagesTail { count, .. } => Some(history_page_json(
            ctx.sessions
                .read_history
                .tail(ctx.messages, ctx.system_prompt, *count)
                .map_err(|err| err.to_string())?,
        )),
        AgentCommand::PersistSession { .. } => None,
        AgentCommand::GetSessionStats { .. } => {
            let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
            let stats = compute_session_stats_with_usage(
                session_key,
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
        // RefreshModels is handled earlier in `dispatch_fieldless_command` on
        // a blocking worker thread: its sequential HTTP must never run inline
        // on the async dispatch loop (slice-4 review).
        AgentCommand::GetSubagents { since, .. } => Some(
            serde_json::to_value(super::protocol::build_compact_subagent_roster(
                &ctx.subagent_registry,
                *since,
            )?)
            .map_err(|e| e.to_string())?,
        ),
        // `get_message` (#1060) and `get_report` (#1859) are answered by
        // `dispatch_fieldless_command` through their composed owners before
        // this projection runs.
        _ => None,
    };
    Ok(data)
}

#[cfg(test)]
#[path = "uds_query_tests.rs"]
mod tests;
#[cfg(test)]
mod cov2_tests {
    use super::{query_response_data, tests::Fx};
    use crate::interface::cli::protocol::AgentCommand;

    #[test]
    fn get_state_query_recovers_poisoned_workflow_lock() {
        let mut fx = Fx::new();
        let state: std::sync::Arc<std::sync::Mutex<crate::domain::workflow::WorkflowEngine>> =
            std::sync::Arc::new(std::sync::Mutex::new(
                crate::domain::workflow::WorkflowEngine::new(
                    crate::domain::workflow::WorkflowConfig {
                        auto_continue: true,
                        completion_nudge: true,
                        selector_prompt: None,
                        dir: None,
                        templates: vec![crate::domain::workflow::WorkflowTemplate {
                            id: "bugfix".into(),
                            label: "Bugfix".into(),
                            description: "test bugfix workflow".into(),
                            when_to_use: Some("tests".into()),
                            steps: vec![crate::domain::workflow::WorkflowTemplateStep {
                                key: "a".into(),
                                label: "A".into(),
                                phase: "test".into(),
                                guidance: None,
                            }],
                            guards: vec![],
                        }],
                    },
                    false,
                )
                .unwrap(),
            ));
        state
            .lock()
            .unwrap()
            .select_template("bugfix", None)
            .unwrap();
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
                since: None,
                agent_id: None,
            },
            &ctx,
        )
        .unwrap();
        assert_eq!(value["workflow"]["activeTemplate"]["id"], "bugfix");
        assert_eq!(value["model"], "stub");
    }

    #[test]
    fn get_state_query_recovers_poisoned_execution_cursor() {
        let mut fx = Fx::new();
        let execution = fx.execution_state.clone();
        let expected_generation = {
            let mut state = execution.lock().unwrap();
            state.observe_visible_revisions(50, 0)
        };
        let poisoned = execution.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison execution lock");
        })
        .join();

        let ctx = fx.ctx();
        let value = query_response_data(
            &AgentCommand::GetState {
                id: None,
                since: None,
                agent_id: None,
            },
            &ctx,
        )
        .unwrap();
        assert_eq!(
            value["generation"].as_u64().unwrap(),
            expected_generation,
            "must retain the execution-owned public cursor exactly after poison: {value}"
        );
        assert_eq!(value["state"], "idle");
    }
}
