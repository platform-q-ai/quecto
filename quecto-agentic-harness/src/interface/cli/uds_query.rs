use super::protocol::AgentCommand;
use super::uds::DispatchCtx;
use super::uds_session::{
    HISTORY_PAGE_SIZE, compute_session_stats_with_usage, messages_page_json_for_id,
    messages_tail_json, position_by_message_id,
};
use crate::domain::ids::{CommandId, MessageId, ToolCallId};
use crate::domain::message::Message;
use crate::infrastructure::tools::subagent_cascade;

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
        AgentCommand::GetContainers { .. } => {
            let registry = ctx.container_registry.as_ref()?;
            let containers = crate::infrastructure::tools::container_registry::list_containers(registry)
                .into_iter()
                .map(|entry| serde_json::json!({
                    "container_uuid": entry.container_uuid,
                    "container_id": entry.container_uuid,
                    "container_ref": entry.container_ref,
                    "container_name": entry.container_name,
                    "environment_id": entry.environment_id,
                    "environment_health": format!("{:?}", entry.status).to_lowercase(),
                    "status": format!("{:?}", entry.status).to_lowercase(),
                    "repo_url": entry.repo_url,
                    "workspace_path": entry.workspace_path,
                    "agents": entry.agents.iter().map(|a| a.as_ref().to_string()).collect::<Vec<_>>(),
                    "members": entry.agents.iter().map(|a| serde_json::json!({"agent_uuid": a.as_ref()})).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>();
            Some(serde_json::json!({ "containers": containers }))
        }
        AgentCommand::KillContainer { container_ref, .. } => {
            let container_registry = ctx.container_registry.as_ref()?;
            let target =
                match crate::infrastructure::tools::container_registry::resolve_container_ref_any(
                    container_registry,
                    container_ref,
                ) {
                    Ok(entry) => entry,
                    Err(error) => {
                        return Some(
                            serde_json::json!({ "container_ref": container_ref, "status": "error", "error": error }),
                        );
                    }
                };
            let registry = ctx.subagent_registry.as_ref();
            let matched: Vec<_> = registry
                .map(|registry| {
                    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
                    entries
                        .iter()
                        .filter(|(_, entry)| {
                            entry.container_ref.as_deref() == Some(target.container_ref.as_str())
                                || entry.container_uuid.as_deref()
                                    == Some(target.container_uuid.as_str())
                                || target.agents.contains(&entry.agent_uuid)
                        })
                        .map(|(id, entry)| (id.clone(), entry.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let agents: Vec<_> = matched.iter().map(|(id, _)| id.clone()).collect();
            if let Some((_, owner)) = matched
                .iter()
                .find(|(_, entry)| entry.container_kill_command.is_some())
            {
                if let Err(error) = crate::infrastructure::tools::container_script_cleanup::run_container_kill_script(owner) {
                    if let Some(registry) = registry {
                        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
                        for (id, _) in &matched {
                            if let Some(entry) = entries.get_mut(id) { entry.environment_health = Some("cleanup_failed".into()); }
                        }
                    }
                    let _ = crate::infrastructure::tools::container_registry::set_container_status(
                        container_registry,
                        &target.container_uuid,
                        crate::infrastructure::tools::container_registry::ContainerStatus::CleanupFailed,
                    );
                    return Some(serde_json::json!({ "container_ref": container_ref, "container_uuid": target.container_uuid, "agents": agents, "status": "error", "error": error, "retry": "fix cleanup and call kill_container again" }));
                }
            }
            for (_, entry) in &matched {
                if let Some(ref tx) = entry.exit_signal_tx {
                    let _ = tx.send(Some(
                        crate::infrastructure::tools::subagent_registry::ExitSignal {
                            exit_code: None,
                            signal: Some(15),
                        },
                    ));
                }
                subagent_cascade::terminate_removed_entry(entry);
            }
            if let Some(registry) = registry {
                let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
                for (id, _) in &matched {
                    if let Some(entry) = entries.get_mut(id) {
                        entry.status =
                            crate::infrastructure::tools::subagent_registry::SubagentStatus::Exited;
                        entry.environment_health = Some("stopped".into());
                    }
                }
            }
            let stopped =
                match crate::infrastructure::tools::container_registry::set_container_status(
                    container_registry,
                    &target.container_uuid,
                    crate::infrastructure::tools::container_registry::ContainerStatus::Stopped,
                ) {
                    Ok(entry) => entry,
                    Err(error) => {
                        return Some(
                            serde_json::json!({ "container_ref": container_ref, "status": "error", "error": error }),
                        );
                    }
                };
            Some(serde_json::json!({
                "container_ref": stopped.container_ref,
                "container_uuid": stopped.container_uuid,
                "agents": agents,
                "status": "stopped"
            }))
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
pub(crate) mod tests;
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
        let value = query_response_data(&AgentCommand::GetState { id: None }, &ctx).unwrap();
        assert!(value.get("workflow").is_none());
        assert_eq!(value["model"], "stub");
    }
}
#[cfg(test)]
mod container_query_tests {
    use super::{query_response_data, tests::Fx};
    use crate::interface::cli::protocol::AgentCommand;
    #[test]
    fn get_containers_uses_authoritative_registry_including_empty_stopped() {
        use crate::infrastructure::tools::container_registry::{
            ContainerEntry, ContainerStatus, register_container,
        };
        let mut fx = Fx::new();
        let reg = crate::infrastructure::tools::container_registry::new_container_registry();
        register_container(
            &reg,
            ContainerEntry {
                container_uuid: "env-empty".into(),
                container_ref: String::new(),
                container_name: Some("empty".into()),
                environment_id: "env-empty".into(),
                repo_url: None,
                workspace_path: "/workspace".into(),
                status: ContainerStatus::Stopped,
                agents: vec![],
                script_name: "default".into(),
                exec_command: "exec".into(),
                inspect_command: "inspect".into(),
                kill_command: "kill".into(),
                socket_path: None,
                socket_proxy: None,
                metadata: serde_json::json!({}),
            },
        );
        fx.container_registry = Some(reg);
        let data =
            query_response_data(&AgentCommand::GetContainers { id: None }, &fx.ctx()).unwrap();
        assert_eq!(data["containers"][0]["container_ref"], "C1");
        assert_eq!(data["containers"][0]["status"], "stopped");
        assert_eq!(data["containers"][0]["agents"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn kill_container_uses_registry_ref_not_stale_subagent_only() {
        use crate::infrastructure::tools::container_registry::{
            ContainerEntry, ContainerStatus, register_container,
        };
        let mut fx = Fx::new();
        let reg = crate::infrastructure::tools::container_registry::new_container_registry();
        register_container(
            &reg,
            ContainerEntry {
                container_uuid: "env-live".into(),
                container_ref: String::new(),
                container_name: None,
                environment_id: "env-live".into(),
                repo_url: None,
                workspace_path: "/workspace".into(),
                status: ContainerStatus::Running,
                agents: vec![],
                script_name: "default".into(),
                exec_command: "exec".into(),
                inspect_command: "inspect".into(),
                kill_command: "kill".into(),
                socket_path: None,
                socket_proxy: None,
                metadata: serde_json::json!({}),
            },
        );
        fx.container_registry = Some(reg.clone());
        let data = query_response_data(
            &AgentCommand::KillContainer {
                id: None,
                container_ref: "C1".into(),
            },
            &fx.ctx(),
        )
        .unwrap();
        assert_eq!(data["status"], "stopped");
        assert_eq!(
            crate::infrastructure::tools::container_registry::list_containers(&reg)[0].status,
            ContainerStatus::Stopped
        );
    }
}
