use super::protocol::AgentCommand;
use super::uds::DispatchCtx;
use super::uds_session::{
    HISTORY_PAGE_SIZE, compute_session_stats_with_usage, messages_page_json, messages_tail_json,
    position_by_wire_id,
};

pub(super) fn get_message_response_data(
    message_id: &str,
    tool_call_id: Option<&str>,
    offset: Option<usize>,
    limit: Option<usize>,
    request_id: Option<&str>,
    ctx: &DispatchCtx<'_>,
) -> Option<serde_json::Value> {
    let message = &ctx.messages[position_by_wire_id(ctx.messages, message_id)?];
    match tool_call_id {
        Some(tool_call_id) => super::uds_session::tool_call_arguments_to_json_range_for_response(
            message,
            tool_call_id,
            offset,
            limit,
            request_id,
        ),
        None => Some(super::uds_session::message_to_json_range_for_response(
            message, offset, limit, request_id,
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
            let state = ctx.session.state_snapshot(
                ctx.messages.len(),
                workflow,
                ctx.agent.max_context_tokens(),
                ctx.agent.effort().map(|l| l.as_str().to_string()),
            );
            Some(serde_json::to_value(&state).unwrap_or_default())
        }
        AgentCommand::GetMessages { count, before, .. } => Some(messages_page_json(
            ctx.messages,
            count.unwrap_or(HISTORY_PAGE_SIZE),
            before.as_deref(),
        )),
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
        // #1060: on-demand single-message lookup by stable id (busy-path safe).
        // Miss returns None so dispatch_fieldless_command emits a structured error.
        AgentCommand::GetMessage {
            id,
            message_id,
            tool_call_id,
            offset,
            limit,
            ..
        } => get_message_response_data(
            message_id,
            tool_call_id.as_deref(),
            *offset,
            *limit,
            id.as_deref(),
            ctx,
        ),
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
    use crate::infrastructure::test_support::message_contents;
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
                wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
                base_dir: self._tmp.path(),
                agent: &mut self.agent,
                messages: &mut self.messages,
                conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                    crate::interface::cli::uds_snapshots::ConversationSnapshotData::default(),
                )),
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
                durable_prefix_dirty: false,
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
                before: None,
                agent_id: None,
            },
            &ctx,
        )
        .unwrap();
        let all_messages = all["messages"].as_array().unwrap();
        assert_eq!(all_messages.len(), 2);
        assert_eq!(all["hasMoreBefore"], false);
        assert_eq!(all["before"], serde_json::Value::Null);

        let tail = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: Some(1),
                before: None,
                agent_id: None,
            },
            &ctx,
        )
        .unwrap();
        assert_eq!(tail["messages"].as_array().unwrap().len(), 1);

        let stats = query_response_data(&AgentCommand::GetSessionStats { id: None }, &ctx).unwrap();
        assert_eq!(stats["sessionKey"], "cli:test");
    }

    fn assert_page_metadata(data: &serde_json::Value, has_more: bool) {
        assert_eq!(data["hasMoreBefore"], has_more);
        if has_more {
            assert!(
                data["before"].as_str().is_some(),
                "older cursor should be present"
            );
        } else {
            assert_eq!(data["before"], serde_json::Value::Null);
        }
        assert_ne!(data["trimmed"], true);
    }

    #[test]
    fn query_get_messages_without_count_returns_newest_bounded_page_with_cursor() {
        let mut fx = Fx::new();
        fx.messages = (0..(HISTORY_PAGE_SIZE * 3))
            .map(|i| Message::user(format!("msg-{i:03}")))
            .collect();
        let ctx = fx.ctx();

        let page = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: None,
                before: None,
                agent_id: None,
            },
            &ctx,
        )
        .expect("get_messages returns a page");
        let contents = message_contents(&page);

        assert_eq!(
            contents.len(),
            HISTORY_PAGE_SIZE,
            "attach/resume must be bounded by the protocol history page size"
        );
        assert_eq!(contents.last().map(String::as_str), Some("msg-191"));
        assert_eq!(contents.first().map(String::as_str), Some("msg-128"));
        assert_page_metadata(&page, true);
    }

    #[test]
    fn query_get_messages_exact_page_has_no_older_cursor() {
        let mut fx = Fx::new();
        fx.messages = (0..HISTORY_PAGE_SIZE)
            .map(|i| Message::user(format!("msg-{i:03}")))
            .collect();
        let ctx = fx.ctx();

        let page = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: None,
                before: None,
                agent_id: None,
            },
            &ctx,
        )
        .expect("exact page returns data");

        assert_eq!(message_contents(&page).len(), HISTORY_PAGE_SIZE);
        assert_page_metadata(&page, false);
    }

    #[test]
    fn query_get_messages_just_over_page_keeps_oldest_reachable() {
        let mut fx = Fx::new();
        fx.messages = (0..(HISTORY_PAGE_SIZE + 1))
            .map(|i| Message::user(format!("msg-{i:03}")))
            .collect();
        let ctx = fx.ctx();

        let newest = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: None,
                before: None,
                agent_id: None,
            },
            &ctx,
        )
        .expect("newest page returns data");
        assert_eq!(message_contents(&newest).len(), HISTORY_PAGE_SIZE);
        assert_eq!(
            message_contents(&newest).first().map(String::as_str),
            Some("msg-001")
        );
        assert_page_metadata(&newest, true);

        let older = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: None,
                before: newest["before"].as_str().map(str::to_owned),
                agent_id: None,
            },
            &ctx,
        )
        .expect("oldest omitted message is reachable");
        assert_eq!(message_contents(&older), ["msg-000"]);
        assert_page_metadata(&older, false);
    }

    #[test]
    fn query_get_messages_before_cursor_returns_adjacent_older_page() {
        let mut fx = Fx::new();
        fx.messages = (0..(HISTORY_PAGE_SIZE * 2))
            .map(|i| Message::user(format!("msg-{i:03}")))
            .collect();
        let cursor = fx.messages[80].id().to_string();
        let cmd: AgentCommand = serde_json::from_value(serde_json::json!({
            "type": "get_messages",
            "before": cursor,
        }))
        .expect("paged get_messages command parses");
        let ctx = fx.ctx();

        let page = query_response_data(&cmd, &ctx).expect("older page returns data");
        let contents = message_contents(&page);

        assert_eq!(contents.len(), HISTORY_PAGE_SIZE);
        assert_eq!(contents.last().map(String::as_str), Some("msg-079"));
        assert_eq!(contents.first().map(String::as_str), Some("msg-016"));
        assert!(contents.iter().all(|content| content.as_str() < "msg-080"));
    }

    #[test]
    fn query_get_messages_pages_to_start_without_gap_or_duplicate() {
        let mut fx = Fx::new();
        fx.messages = (0..(HISTORY_PAGE_SIZE * 2 + 1))
            .map(|i| Message::user(format!("msg-{i:03}")))
            .collect();
        let ctx = fx.ctx();
        let mut before = None;
        let mut collected = Vec::new();

        loop {
            let page = query_response_data(
                &AgentCommand::GetMessages {
                    id: None,
                    count: None,
                    before: before.clone(),
                    agent_id: None,
                },
                &ctx,
            )
            .expect("page returns data");
            let mut contents = message_contents(&page);
            contents.extend(collected);
            collected = contents;
            if page["hasMoreBefore"] == false {
                break;
            }
            before = page["before"].as_str().map(str::to_owned);
        }

        assert_eq!(collected.len(), HISTORY_PAGE_SIZE * 2 + 1);
        assert_eq!(collected.first().map(String::as_str), Some("msg-000"));
        assert_eq!(collected.last().map(String::as_str), Some("msg-128"));
        let unique = collected.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            unique.len(),
            collected.len(),
            "collected messages must be exact-once"
        );
    }

    #[test]
    fn query_get_messages_count_is_older_client_newest_slice_with_page_metadata() {
        let mut fx = Fx::new();
        fx.messages = (0..10).map(|i| Message::user(format!("msg-{i}"))).collect();
        let ctx = fx.ctx();

        let page = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: Some(3),
                before: None,
                agent_id: None,
            },
            &ctx,
        )
        .expect("compat get_messages count returns data");
        let contents = message_contents(&page);

        assert_eq!(contents, ["msg-7", "msg-8", "msg-9"]);
        assert_page_metadata(&page, true);
    }

    #[test]
    fn query_get_messages_explicit_count_above_page_size_is_preserved() {
        let mut fx = Fx::new();
        fx.messages = (0..100)
            .map(|i| Message::user(format!("msg-{i:03}")))
            .collect();
        let ctx = fx.ctx();

        let page = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: Some(80),
                before: None,
                agent_id: None,
            },
            &ctx,
        )
        .expect("explicit count returns data");

        assert_eq!(message_contents(&page).len(), 80);
        assert_eq!(
            message_contents(&page).first().map(String::as_str),
            Some("msg-020")
        );
    }

    #[test]
    fn query_get_messages_count_at_or_beyond_total_has_no_older_cursor() {
        let mut fx = Fx::new();
        fx.messages = (0..3).map(|i| Message::user(format!("msg-{i}"))).collect();
        let ctx = fx.ctx();

        for count in [3, 10] {
            let page = query_response_data(
                &AgentCommand::GetMessages {
                    id: None,
                    count: Some(count),
                    before: None,
                    agent_id: None,
                },
                &ctx,
            )
            .expect("compat page returns data");
            assert_eq!(message_contents(&page), ["msg-0", "msg-1", "msg-2"]);
            assert_page_metadata(&page, false);
        }
    }

    #[test]
    fn query_get_messages_count_with_before_returns_bounded_slice_before_cursor() {
        let mut fx = Fx::new();
        fx.messages = (0..20)
            .map(|i| Message::user(format!("msg-{i:02}")))
            .collect();
        let cursor = fx.messages[10].id().to_string();
        let ctx = fx.ctx();

        let page = query_response_data(
            &AgentCommand::GetMessages {
                id: None,
                count: Some(4),
                before: Some(cursor),
                agent_id: None,
            },
            &ctx,
        )
        .expect("count combined with before returns data");

        assert_eq!(
            message_contents(&page),
            ["msg-06", "msg-07", "msg-08", "msg-09"],
            "an explicit count pages that many messages strictly before the cursor"
        );
        assert_page_metadata(&page, true);
    }

    #[test]
    fn query_get_message_hit_returns_message_by_stable_id() {
        let mut fx = Fx::new();
        // Resolve against the stable id of the second (assistant) message.
        let target_id = fx.messages[1].id().to_string();
        let ctx = fx.ctx();
        let hit = query_response_data(
            &AgentCommand::GetMessage {
                id: Some("r1".into()),
                message_id: target_id.clone(),
                agent_id: None,
                tool_call_id: None,
                offset: None,
                limit: None,
            },
            &ctx,
        )
        .expect("get_message must resolve a present stable id");
        assert_eq!(
            hit["id"], target_id,
            "resolved message carries its stable id"
        );
        assert_eq!(hit["role"], "assistant");
        assert_eq!(
            hit["content"], "two",
            "the exact referenced body is returned"
        );
    }

    #[test]
    fn query_get_message_miss_returns_none_for_structured_error() {
        let mut fx = Fx::new();
        let ctx = fx.ctx();
        // An unknown id must return None so dispatch emits a structured
        // "message not found" error rather than a stale/empty hit (#1060).
        let miss = query_response_data(
            &AgentCommand::GetMessage {
                id: Some("r1".into()),
                message_id: "00000000-0000-0000-0000-000000000000".into(),
                agent_id: None,
                tool_call_id: None,
                offset: None,
                limit: None,
            },
            &ctx,
        );
        assert!(
            miss.is_none(),
            "unknown message id must miss (None), got {miss:?}"
        );
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
