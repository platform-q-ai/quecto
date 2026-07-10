use super::protocol::AgentCommand;
use super::uds::DispatchCtx;
use super::uds_session::{compute_session_stats_with_usage, message_to_json, messages_tail_json};

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
            let state = ctx.session.state_snapshot(
                ctx.messages.len(),
                workflow,
                ctx.agent.max_context_tokens(),
                ctx.agent.effort().map(|l| l.as_str().to_string()),
            );
            Some(serde_json::to_value(&state).unwrap_or_default())
        }
        AgentCommand::GetMessages { count, .. } => match count {
            Some(count) => Some(messages_tail_json(ctx.messages, *count)),
            None => {
                let msgs: Vec<serde_json::Value> =
                    ctx.messages.iter().map(message_to_json).collect();
                Some(serde_json::json!({ "messages": msgs }))
            }
        },
        AgentCommand::GetMessagesTail { count, .. } => {
            Some(messages_tail_json(ctx.messages, *count))
        }
        AgentCommand::GetSessionStats { .. } => {
            let stats = compute_session_stats_with_usage(
                ctx.session_key,
                ctx.messages,
                ctx.session.usage_snapshot(),
                ctx.session.context_tokens(),
                ctx.agent.max_context_tokens(),
            );
            Some(serde_json::to_value(&stats).unwrap_or_default())
        }
        AgentCommand::GetExtensions { .. } => Some(
            serde_json::json!({ "extensions": super::uds_extensions::build_extension_list(ctx) }),
        ),
        AgentCommand::ListModels { .. } => Some(super::uds_models::list_models_response(ctx)),
        AgentCommand::GetSubagents { .. } => {
            let list = super::protocol::build_subagent_info_list(&ctx.subagent_registry);
            Some(serde_json::json!({ "subagents": list }))
        }
        AgentCommand::ReloadExtensions { .. } => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use crate::domain::message::Message;
    use crate::infrastructure::persistence::session_store::FileSessionStore;
    use crate::interface::cli::protocol::AgentCommand;
    use crate::interface::cli::uds_cancel::CancelSlot;
    use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;
    use crate::interface::cli::uds_session::AgentSession;

    struct Fx {
        agent: AgentLoopImpl,
        messages: Vec<Message>,
        session: AgentSession,
        session_key: String,
        store: FileSessionStore,
        _tmp: tempfile::TempDir,
        writer: tokio::io::Sink,
    }

    impl Fx {
        fn new() -> Self {
            let tmp = tempfile::TempDir::new().unwrap();
            Self {
                agent: AgentLoopImpl::new(AgentLoopConfig {
                    provider: crate::interface::test_support::make_stub_provider(),
                    tool_registry: Box::new(
                        crate::infrastructure::tools::registry::ToolRegistryImpl::new(),
                    ),
                    model: "stub".into(),
                    max_tokens: 100,
                    temperature: 0.0,
                    spill_store: None,
                    session_key: "cli:test".into(),
                    context_collapse_after_tool_calls: u32::MAX,
                    max_context_tokens: 190_000,
                    progress_callback: None,
                    streaming: false,
                    effort: None,
                    system_prompt_provider: None,
                    audit_log: None,
                    pin_recent_turns: 2,
                    context_collapse_after_messages: u32::MAX,
                    model_context_window: None,
                }),
                messages: vec![Message::user("one"), Message::assistant("two", vec![])],
                session: AgentSession::new("stub".into(), "cli:test".into()),
                session_key: "cli:test".into(),
                store: FileSessionStore::new(tmp.path()),
                _tmp: tmp,
                writer: tokio::io::sink(),
            }
        }

        fn ctx(&mut self) -> crate::interface::cli::uds::DispatchCtx<'_> {
            let initial_stats = compute_session_stats_with_usage(
                &self.session_key,
                &self.messages,
                self.session.usage_snapshot(),
                self.session.context_tokens(),
                self.agent.max_context_tokens(),
            );
            crate::interface::cli::uds::DispatchCtx {
                base_dir: self._tmp.path(),
                agent: &mut self.agent,
                messages: &mut self.messages,
                conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
                state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                    self.session.state_snapshot(0, None, 0, None),
                )),
                session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                    initial_stats,
                )),
                extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
                busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                session: &mut self.session,
                stdout: Some(&mut self.writer),
                session_key: &mut self.session_key,
                session_store: &self.store,
                ephemeral: false,
                system_prompt: "",
                cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
                turn_control: std::sync::Arc::default(),
                broadcast_tx: None,
                ext_registry: None,
                client_tool_registry: new_client_tool_registry(),
                current_client_id: 0,
                subagent_registry: None,
                notification_rx: None,
                workflow_state: None,
                workflow_config: None,
                provider_reload: None,
                provider_reload_inputs: None,
                last_persisted_message_index: 0,
            }
        }
    }

    #[test]
    fn query_get_state_messages_and_stats_are_shaped() {
        let mut fx = Fx::new();
        let ctx = fx.ctx();

        let state = query_response_data(&AgentCommand::GetState { id: None }, &ctx).unwrap();
        assert_eq!(state["model"], "stub");
        assert_eq!(state["messageCount"], 2);

        let all = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: None,
                agent_id: None,
            },
            &ctx,
        )
        .unwrap();
        assert_eq!(all["messages"].as_array().unwrap().len(), 2);

        let tail = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: Some(1),
                agent_id: None,
            },
            &ctx,
        )
        .unwrap();
        assert_eq!(tail["messages"].as_array().unwrap().len(), 1);

        let stats = query_response_data(&AgentCommand::GetSessionStats { id: None }, &ctx).unwrap();
        assert_eq!(stats["sessionKey"], "cli:test");
    }

    #[test]
    fn query_metadata_commands_are_shaped_or_deferred() {
        let mut fx = Fx::new();
        let ctx = fx.ctx();
        assert!(query_response_data(&AgentCommand::GetExtensions { id: None }, &ctx).unwrap()["extensions"].is_array());
        assert!(
            query_response_data(&AgentCommand::ListModels { id: None }, &ctx).unwrap()["models"]
                .is_array()
        );
        assert!(query_response_data(&AgentCommand::GetSubagents { id: None }, &ctx).unwrap()["subagents"].is_array());
        assert!(query_response_data(&AgentCommand::ReloadExtensions { id: None }, &ctx).is_none());
    }
}
