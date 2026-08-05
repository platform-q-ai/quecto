use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::{Message, ToolCall};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::test_support::message_contents;
use crate::infrastructure::tools::registration::ToolRegistration;
use std::{future::Future, pin::Pin, sync::Arc};
struct CatalogueFixtureTool {
    def: ToolDefinition,
}
impl CatalogueFixtureTool {
    fn new(name: &str) -> Self {
        Self {
            def: ToolDefinition {
                name: name.to_string().into(),
                description: format!("Fixture {name} tool").into(),
                parameters_schema: r#"{"type":"object"}"#.into(),
            },
        }
    }
}
impl std::fmt::Debug for CatalogueFixtureTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogueFixtureTool")
            .field("name", &self.def.name)
            .finish()
    }
}
impl Tool for CatalogueFixtureTool {
    fn definition(&self) -> ToolDefinition {
        self.def.clone()
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<
        Box<dyn Future<Output = Result<ToolResult, crate::domain::error::DomainError>> + Send + '_>,
    > {
        Box::pin(async move {
            Ok(ToolResult {
                content: "ok".into(),
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}
use crate::interface::cli::protocol::AgentCommand;
use crate::interface::cli::uds_cancel::CancelSlot;
use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;
use crate::interface::cli::uds_session::AgentSession;
pub(crate) struct Fx {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    session: AgentSession,
    session_key: String,
    store: FileSessionStore,
    _tmp: tempfile::TempDir,
    writer: tokio::io::Sink,
    subagent_registry: Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub(crate) container_registry:
        Option<crate::infrastructure::tools::container_registry::ContainerRegistry>,
}
impl Fx {
    pub(crate) fn new() -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut tool_registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
        tool_registry.register_with_metadata(
            Arc::new(CatalogueFixtureTool::new("bash")),
            ToolRegistration::official_native(),
        );
        Self {
            agent: AgentLoopImpl::new(AgentLoopConfig {
                provider: crate::interface::test_support::make_stub_provider(),
                tool_registry: Box::new(tool_registry),
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
                tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
            }),
            messages: vec![Message::user("one"), Message::assistant("two", vec![])],
            session: AgentSession::new("stub".into(), "cli:test".into()),
            session_key: "cli:test".into(),
            store: FileSessionStore::new(tmp.path()),
            _tmp: tmp,
            writer: tokio::io::sink(),
            subagent_registry: None,
            container_registry: None,
        }
    }
    pub(crate) fn ctx(&mut self) -> crate::interface::cli::uds::DispatchCtx<'_> {
        let initial_stats = compute_session_stats_with_usage(
            &self.session_key,
            &self.messages,
            self.session.usage_snapshot(),
            self.session.context_tokens(),
            self.agent.max_context_tokens(),
        );
        crate::interface::cli::uds::DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
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
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
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
            _ext_registry: None,
            client_tool_registry: new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: self.subagent_registry.clone(),
            container_registry: self.container_registry.clone(),
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
    assert_eq!(state["execution"]["phase"], "idle");
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
    for message_id in ["00000000-0000-0000-0000-000000000000", "not-a-uuid"] {
        let mut fx = Fx::new();
        let ctx = fx.ctx();
        // An unknown id must return None so dispatch emits a structured
        // "message not found" error rather than a stale/empty hit (#1060).
        let miss = query_response_data(
            &AgentCommand::GetMessage {
                id: Some("r1".into()),
                message_id: message_id.into(),
                agent_id: None,
                tool_call_id: None,
                offset: None,
                limit: None,
            },
            &ctx,
        );
        assert!(
            miss.is_none(),
            "unknown message id {message_id:?} must miss (None), got {miss:?}"
        );
    }
}
#[test]
fn query_get_message_tool_call_uses_tool_id_not_request_or_message_id() {
    let mut fx = Fx::new();
    fx.messages.push(Message::assistant(
        "call requested",
        vec![ToolCall {
            id: "call-target".into(),
            name: "bash".into(),
            arguments: "{\"command\":\"echo typed\"}".into(),
        }],
    ));
    let message_id = fx.messages.last().unwrap().id().to_string();
    let ctx = fx.ctx();
    let hit = query_response_data(
        &AgentCommand::GetMessage {
            id: Some("response-correlation".into()),
            message_id: message_id.clone(),
            agent_id: None,
            tool_call_id: Some("call-target".into()),
            offset: None,
            limit: None,
        },
        &ctx,
    )
    .expect("tool-call argument lookup must resolve by the requested toolCallId");
    assert_eq!(hit["id"], message_id);
    assert_eq!(hit["toolCallId"], "call-target");
    assert_eq!(hit["toolName"], "bash");
    assert_eq!(hit["arguments"], "{\"command\":\"echo typed\"}");
    assert!(
        hit.get("response-correlation").is_none(),
        "request/correlation id must not be confused with message or tool-call ids"
    );
}
#[test]
fn query_metadata_commands_are_shaped_or_deferred() {
    let mut fx = Fx::new();
    let ctx = fx.ctx();
    let catalogue =
        query_response_data(&AgentCommand::GetToolCatalogue { id: None }, &ctx).unwrap();
    let tools = catalogue["tools"]
        .as_array()
        .expect("tools must be an array");
    let bash = tools
        .iter()
        .find(|entry| entry["name"].as_str() == Some("bash"))
        .expect("core bash tool should be present in the catalogue");
    assert_eq!(bash["source"].as_str(), Some("bundled-native"));
    assert_eq!(bash["runtimeAvailability"].as_str(), Some("enabled"));
    assert_eq!(bash["effectiveEnabled"].as_bool(), Some(true));
    assert_eq!(bash["inputSchema"].as_str(), Some(r#"{"type":"object"}"#));
    assert!(
        query_response_data(&AgentCommand::ListModels { id: None }, &ctx).unwrap()["models"]
            .is_array()
    );
    assert!(
        query_response_data(&AgentCommand::GetSubagents { id: None }, &ctx).unwrap()["subagents"]
            .is_array()
    );
}
#[test]
fn kill_container_invokes_script_marks_members_stopped_and_signals_exit() {
    use crate::domain::ids::AgentUuid;
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_exit_signal_channel, new_registry,
    };
    let mut fx = Fx::new();
    let marker = fx._tmp.path().join("killed.txt");
    let registry = new_registry();
    let (tx, rx) = new_exit_signal_channel();
    let uuid = AgentUuid::mint();
    let mut entry = SubagentEntry::with_identity(
        uuid.clone(),
        "agent-c1".into(),
        fx._tmp.path().join("agent.sock"),
        0,
    );
    entry.status = SubagentStatus::Running;
    entry.runtime_backend = "container-script".into();
    entry.container_ref = Some("C1".into());
    entry.container_uuid = Some("env-1".into());
    entry.environment_id = Some("env-1".into());
    entry.workspace_path = Some(fx._tmp.path().to_string_lossy().to_string());
    let kill = fx._tmp.path().join("kill.py");
    std::fs::write(&kill, format!(
        "#!/usr/bin/env python3\nimport json, os\nopen({:?}, 'w').write(os.environ.get('QUECTO_CONTAINER_REF','') + ':' + os.environ.get('QUECTO_ENVIRONMENT_UUID',''))\nprint(json.dumps({{'environment_id': os.environ.get('QUECTO_ENVIRONMENT_UUID',''), 'status': 'stopped', 'workspace_path': os.environ.get('QUECTO_WORKSPACE_PATH',''), 'container_ref': os.environ.get('QUECTO_CONTAINER_REF',''), 'metadata': {{}}, 'cleanup': {{'removed': True}}}}))\n",
        marker
    )).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&kill, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    entry.container_kill_command = Some(kill.to_string_lossy().into_owned());
    entry.exit_signal_tx = Some(tx);
    registry.lock().unwrap().insert(uuid.to_string(), entry);
    fx.subagent_registry = Some(registry.clone());
    let creg = crate::infrastructure::tools::container_registry::new_container_registry();
    crate::infrastructure::tools::container_registry::register_container(
        &creg,
        crate::infrastructure::tools::container_registry::ContainerEntry {
            container_uuid: "env-1".into(),
            container_ref: String::new(),
            container_name: None,
            environment_id: "env-1".into(),
            repo_url: None,
            workspace_path: fx._tmp.path().to_string_lossy().to_string(),
            status: crate::infrastructure::tools::container_registry::ContainerStatus::Running,
            agents: vec![uuid.clone()],
            script_name: String::new(),
            exec_command: String::new(),
            inspect_command: String::new(),
            kill_command: String::new(),
            socket_path: None,
            socket_proxy: None,
            metadata: serde_json::json!({}),
        },
    );
    fx.container_registry = Some(creg);
    let response = query_response_data(
        &AgentCommand::KillContainer {
            id: None,
            container_ref: "C1".into(),
        },
        &fx.ctx(),
    )
    .expect("kill_container response");
    assert_eq!(response["status"], "stopped");
    assert_eq!(response["agents"].as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "C1:env-1");
    let entries = registry.lock().unwrap();
    let stopped = entries.get(uuid.as_str()).unwrap();
    assert_eq!(stopped.status, SubagentStatus::Exited);
    assert_eq!(stopped.environment_health.as_deref(), Some("stopped"));
    drop(entries);
    assert!(rx.borrow().is_some());
}
#[test]
fn kill_container_reports_script_failure_without_pretending_success() {
    use crate::domain::ids::AgentUuid;
    use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
    let mut fx = Fx::new();
    let registry = new_registry();
    let uuid = AgentUuid::mint();
    let mut entry = SubagentEntry::with_identity(
        uuid.clone(),
        "agent-c1".into(),
        fx._tmp.path().join("agent.sock"),
        0,
    );
    entry.container_ref = Some("C1".into());
    let kill = fx._tmp.path().join("fail.py");
    std::fs::write(
        &kill,
        "#!/usr/bin/env python3\nimport sys\nprint('boom', file=sys.stderr)\nsys.exit(7)\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&kill, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    entry.container_kill_command = Some(kill.to_string_lossy().into_owned());
    registry.lock().unwrap().insert(uuid.to_string(), entry);
    fx.subagent_registry = Some(registry);
    let creg = crate::infrastructure::tools::container_registry::new_container_registry();
    crate::infrastructure::tools::container_registry::register_container(
        &creg,
        crate::infrastructure::tools::container_registry::ContainerEntry {
            container_uuid: "env-1".into(),
            container_ref: String::new(),
            container_name: None,
            environment_id: "env-1".into(),
            repo_url: None,
            workspace_path: fx._tmp.path().to_string_lossy().to_string(),
            status: crate::infrastructure::tools::container_registry::ContainerStatus::Running,
            agents: vec![uuid.clone()],
            script_name: String::new(),
            exec_command: String::new(),
            inspect_command: String::new(),
            kill_command: String::new(),
            socket_path: None,
            socket_proxy: None,
            metadata: serde_json::json!({}),
        },
    );
    fx.container_registry = Some(creg);
    let response = query_response_data(
        &AgentCommand::KillContainer {
            id: None,
            container_ref: "C1".into(),
        },
        &fx.ctx(),
    )
    .expect("kill_container response");
    assert_eq!(response["status"], "error");
    assert!(response["error"].as_str().unwrap().contains("boom"));
    let entries = fx.subagent_registry.as_ref().unwrap().lock().unwrap();
    let failed = entries.get(uuid.as_str()).unwrap();
    assert_ne!(
        failed.status,
        crate::infrastructure::tools::subagent_registry::SubagentStatus::Exited
    );
    assert_eq!(failed.environment_health.as_deref(), Some("cleanup_failed"));
}
#[test]
fn kill_container_uses_canonical_cleanup_env() {
    use crate::domain::ids::AgentUuid;
    use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
    let mut fx = Fx::new();
    let marker = fx._tmp.path().join("env.txt");
    let registry = new_registry();
    let uuid = AgentUuid::mint();
    let mut entry = SubagentEntry::with_identity(
        uuid.clone(),
        "agent-c1".into(),
        fx._tmp.path().join("agent.sock"),
        0,
    );
    entry.container_ref = Some("C-ref".into());
    entry.container_uuid = Some("uuid-1".into());
    entry.container_name = Some("name-1".into());
    entry.environment_id = Some("env-1".into());
    entry.workspace_path = Some(fx._tmp.path().to_string_lossy().to_string());
    entry.container_script_name = Some("dev".into());
    let kill = fx._tmp.path().join("envkill.py");
    std::fs::write(&kill, format!(
        "#!/usr/bin/env python3\nimport json, os\nvals=[os.environ.get(k,'') for k in ['QUECTO_CONTAINER_UUID','QUECTO_CONTAINER_REF','QUECTO_CONTAINER_NAME','QUECTO_ENVIRONMENT_UUID','QUECTO_WORKSPACE_PATH','QUECTO_CONTAINER_SCRIPT']]\nopen({:?}, 'w').write('|'.join(vals))\nprint(json.dumps({{'environment_id': os.environ.get('QUECTO_ENVIRONMENT_UUID',''), 'status': 'stopped', 'workspace_path': os.environ.get('QUECTO_WORKSPACE_PATH',''), 'container_ref': os.environ.get('QUECTO_CONTAINER_REF',''), 'metadata': {{}}, 'cleanup': {{'removed': True}}}}))\n",
        marker
    )).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&kill, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    entry.container_kill_command = Some(kill.to_string_lossy().into_owned());
    registry.lock().unwrap().insert(uuid.to_string(), entry);
    fx.subagent_registry = Some(registry);
    let creg = crate::infrastructure::tools::container_registry::new_container_registry();
    crate::infrastructure::tools::container_registry::register_container(
        &creg,
        crate::infrastructure::tools::container_registry::ContainerEntry {
            container_uuid: "uuid-1".into(),
            container_ref: String::new(),
            container_name: Some("name-1".into()),
            environment_id: "env-1".into(),
            repo_url: None,
            workspace_path: fx._tmp.path().to_string_lossy().to_string(),
            status: crate::infrastructure::tools::container_registry::ContainerStatus::Running,
            agents: vec![uuid.clone()],
            script_name: "dev".into(),
            exec_command: String::new(),
            inspect_command: String::new(),
            kill_command: String::new(),
            socket_path: None,
            socket_proxy: None,
            metadata: serde_json::json!({}),
        },
    );
    fx.container_registry = Some(creg);
    let response = query_response_data(
        &AgentCommand::KillContainer {
            id: None,
            container_ref: "uuid-1".into(),
        },
        &fx.ctx(),
    )
    .unwrap();
    assert_eq!(response["status"], "stopped");
    assert_eq!(
        std::fs::read_to_string(marker).unwrap(),
        format!("uuid-1|C-ref|name-1|env-1|{}|dev", fx._tmp.path().display())
    );
}
