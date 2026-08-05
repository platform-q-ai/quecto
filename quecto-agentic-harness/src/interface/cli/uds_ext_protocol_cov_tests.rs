use super::*;
use crate::domain::provider::{ChatRequest, LlmProvider};

#[derive(Debug)]
struct CovProvider;

impl LlmProvider for CovProvider {
    fn name(&self) -> &str {
        "cov"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::domain::message::LlmResponse,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err(crate::domain::error::DomainError::Provider("unused".into())) })
    }
}

#[tokio::test]
async fn cov_provider_trait_methods_are_invoked() {
    let provider = CovProvider;
    assert_eq!(provider.name(), "cov");
    assert!(provider.as_any().downcast_ref::<CovProvider>().is_some());
    let messages = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "stub",
        max_tokens: 100,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let err = provider.chat(request).await.unwrap_err();
    assert!(err.to_string().contains("unused"));

    let messages = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "stub",
        max_tokens: 100,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let err = provider.chat_stream(request).await.unwrap_err();
    assert!(err.to_string().contains("unused"));

    let messages = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "stub",
        max_tokens: 100,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    assert!(
        matches!(rx.recv().await, Some(crate::domain::provider::StreamEvent::Error(message)) if message.contains("unused"))
    );
}

pub(super) fn cov_agent() -> crate::application::agent_loop::AgentLoopImpl {
    cov_agent_with_registry(crate::infrastructure::tools::registry::ToolRegistryImpl::new())
}

pub(super) fn cov_agent_with_registry(
    registry: crate::infrastructure::tools::registry::ToolRegistryImpl,
) -> crate::application::agent_loop::AgentLoopImpl {
    crate::application::agent_loop::AgentLoopImpl::new(
        crate::application::agent_loop::AgentLoopConfig {
            provider: std::sync::Arc::new(CovProvider),
            tool_registry: Box::new(registry),
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
        },
    )
}

pub(super) fn tool_reg(name: &str) -> ToolRegistration {
    ToolRegistration {
        name: name.to_string(),
        description: format!("{name} description"),
        parameters_schema: r#"{"type":"object"}"#.to_string(),
        stable_id: None,
    }
}

#[test]
fn unregister_response_lists_only_removed_registered_tools() {
    let registry = new_client_tool_registry();
    let core = HashSet::new();
    let (ok, _, _) = handle_register_tools(RegisterToolsArgs {
        client_id: 9,
        id: None,
        tools: &[tool_reg("alpha"), tool_reg("beta")],
        registry: &registry,
        core_tool_names: &core,
    });
    assert!(ok);

    let (event, removed) = handle_unregister_tools(
        9,
        Some("unreg-1"),
        &["alpha".to_string(), "missing".to_string()],
        &registry,
    );

    assert_eq!(removed, vec!["alpha"]);
    let json = event.to_json_line();
    assert!(json.contains("\"id\":\"unreg-1\""), "json: {json}");
    assert!(
        json.contains("\"unregistered\":[\"alpha\"]"),
        "json: {json}"
    );
    let locked = registry.lock().unwrap();
    assert!(!locked[&9].tool_names.contains("alpha"));
    assert!(locked[&9].tool_names.contains("beta"));
}

#[test]
fn register_tools_rejects_core_shadow_duplicate_and_other_client_collision() {
    let registry = new_client_tool_registry();
    let core = HashSet::from(["bash".to_string()]);

    let (ok, ev, tools) = handle_register_tools(RegisterToolsArgs {
        client_id: 1,
        id: Some("shadow"),
        tools: &[tool_reg("bash")],
        registry: &registry,
        core_tool_names: &core,
    });
    assert!(!ok);
    assert!(tools.is_empty());
    assert!(ev.to_json_line().contains("shadows a core tool"));

    let (ok, ev, _) = handle_register_tools(RegisterToolsArgs {
        client_id: 1,
        id: Some("dup"),
        tools: &[tool_reg("ext"), tool_reg("ext")],
        registry: &registry,
        core_tool_names: &HashSet::new(),
    });
    assert!(!ok);
    assert!(ev.to_json_line().contains("registered more than once"));

    assert!(
        handle_register_tools(RegisterToolsArgs {
            client_id: 1,
            id: None,
            tools: &[tool_reg("owned")],
            registry: &registry,
            core_tool_names: &HashSet::new(),
        })
        .0
    );
    let (ok, ev, _) = handle_register_tools(RegisterToolsArgs {
        client_id: 2,
        id: Some("collision"),
        tools: &[tool_reg("owned")],
        registry: &registry,
        core_tool_names: &HashSet::new(),
    });
    assert!(!ok);
    assert!(ev.to_json_line().contains("already registered by client 1"));
}

#[tokio::test]
async fn tool_result_for_pending_sender_delivers_and_sweeps_expired_entries() {
    let registry = new_client_tool_registry();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (expired_tx, expired_rx) = tokio::sync::oneshot::channel();
    {
        let mut reg = registry.lock().unwrap();
        let state = reg.entry(5).or_default();
        state.insert_pending(
            "live".to_string(),
            "tool".to_string(),
            tx,
            std::time::Duration::from_secs(30),
        );
        state.pending_results.insert(
            "expired".to_string(),
            PendingResult {
                reply: expired_tx,
                tool_name: "tool".to_string(),
                deadline: std::time::Instant::now() - std::time::Duration::from_millis(1),
            },
        );
    }

    handle_tool_result(ToolResultArgs {
        client_id: 5,
        tool_call_id: "live",
        content: "done",
        is_error: false,
        registry: &registry,
    });

    let result = rx.await.expect("live result delivered");
    assert_eq!(result.content, "done");
    assert!(!result.is_error);
    assert!(expired_rx.await.is_err(), "expired sender was dropped");
    assert!(registry.lock().unwrap()[&5].pending_results.is_empty());
}

#[test]
fn tool_result_for_unknown_client_does_not_panic_or_create_state() {
    let registry = new_client_tool_registry();

    handle_tool_result(ToolResultArgs {
        client_id: 123,
        tool_call_id: "missing-call",
        content: "late result",
        is_error: false,
        registry: &registry,
    });

    assert!(registry.lock().unwrap().is_empty());
}

#[tokio::test]
async fn client_writer_tx_returns_clone_and_ack_accepted_control_forwards_optional_work() {
    let registry = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(2);
    register_client_writer(&registry, 77, tx);

    let cloned = client_writer_tx(&registry, 77).expect("registered writer");
    cloned.send("hello extension".to_string()).await.unwrap();
    assert_eq!(rx.recv().await.as_deref(), Some("hello extension"));
    assert!(client_writer_tx(&registry, 88).is_none());

    let forward = ack_accepted_control(
        &registry,
        77,
        super::super::uds_control_forward::AcceptedControl {
            ack_line: "ack\n".to_string(),
            forward_line: Some("work".to_string()),
        },
    )
    .await;
    assert_eq!(forward.as_deref(), Some("work"));
    assert_eq!(rx.recv().await.as_deref(), Some("ack\n"));

    let forward = ack_accepted_control(
        &registry,
        88,
        super::super::uds_control_forward::AcceptedControl {
            ack_line: "lost\n".to_string(),
            forward_line: None,
        },
    )
    .await;
    assert!(forward.is_none());
}

#[test]
fn handle_client_disconnect_removes_state_and_errors_pending_execution() {
    let registry = new_client_tool_registry();
    let (tx, rx) = tokio::sync::oneshot::channel();
    assert!(
        handle_register_tools(RegisterToolsArgs {
            client_id: 44,
            id: None,
            tools: &[tool_reg("bye")],
            registry: &registry,
            core_tool_names: &HashSet::new(),
        })
        .0
    );
    registry
        .lock()
        .unwrap()
        .get_mut(&44)
        .unwrap()
        .insert_pending(
            "call".to_string(),
            "tool".to_string(),
            tx,
            std::time::Duration::from_secs(30),
        );

    let removed = handle_client_disconnect(44, &registry);

    assert_eq!(removed, vec!["bye".to_string()]);
    assert!(!registry.lock().unwrap().contains_key(&44));
    let result = rx.blocking_recv().expect("disconnect result");
    assert_eq!(result.content, "Extension disconnected");
    assert!(result.is_error);
    assert!(handle_client_disconnect(44, &registry).is_empty());
}

#[tokio::test]
async fn handle_one_request_sends_execute_tool_and_tool_result_resolves_reply() {
    let registry = new_client_tool_registry();
    let (writer_tx, mut writer_rx) = tokio::sync::mpsc::channel::<String>(1);
    register_client_writer(&registry, 91, writer_tx.clone());
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

    handle_one_request(
        "weather",
        crate::domain::extension_tool::ToolInvocation {
            tool_call_id: "call-1".to_string(),
            tool_name: "weather".to_string(),
            arguments: r#"{"city":"Oslo"}"#.to_string(),
            reply: reply_tx,
        },
        91,
        &registry,
        &Some(writer_tx),
    )
    .await;

    let line = writer_rx.recv().await.expect("execute_tool event");
    assert!(line.contains(r#""type":"execute_tool""#), "{line}");
    assert!(line.contains(r#""toolName":"weather""#), "{line}");
    assert!(
        registry.lock().unwrap()[&91]
            .pending_results
            .contains_key("call-1")
    );

    handle_tool_result(ToolResultArgs {
        client_id: 91,
        tool_call_id: "call-1",
        content: "snow",
        is_error: false,
        registry: &registry,
    });
    let result = reply_rx.await.expect("tool result delivered");
    assert_eq!(result.content, "snow");
    assert!(!result.is_error);
    assert!(registry.lock().unwrap()[&91].pending_results.is_empty());
}

#[tokio::test]
async fn handle_one_request_without_writer_drops_pending_so_caller_fails_fast() {
    let registry = new_client_tool_registry();
    registry.lock().unwrap().entry(92).or_default();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

    handle_one_request(
        "offline",
        crate::domain::extension_tool::ToolInvocation {
            tool_call_id: "call-2".to_string(),
            tool_name: "offline".to_string(),
            arguments: "{}".to_string(),
            reply: reply_tx,
        },
        92,
        &registry,
        &None,
    )
    .await;

    assert!(registry.lock().unwrap()[&92].pending_results.is_empty());
    assert!(
        reply_rx.await.is_err(),
        "reply sender was dropped immediately"
    );
}

#[tokio::test]
async fn dispatch_register_tools_adds_extension_and_forwards_real_tool_execute() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut agent = cov_agent();
    let mut messages = Vec::new();
    let mut session =
        super::super::uds_session::AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let registry = new_client_tool_registry();
    let (writer_tx, mut writer_rx) = tokio::sync::mpsc::channel::<String>(4);
    register_client_writer(&registry, 123, writer_tx);
    let tools = [tool_reg("cov_ext")];
    let state = session.state_snapshot(0, None, 0, None);
    let initial_stats = super::super::uds_session::compute_session_stats(&session_key, &messages);
    let mut ctx = super::super::uds::DispatchCtx {
        execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        wire_mode: super::super::uds_wire::ConnectionWireMode::legacy(),
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
            super::super::uds_snapshots::ConversationSnapshotData::default(),
        )),
        state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(state)),
        session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
        tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: Some(&mut writer),
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Idle,
        )),
        turn_control: std::sync::Arc::default(),
        broadcast_tx: None,
        _ext_registry: None,
        client_tool_registry: registry.clone(),
        current_client_id: 123,
        subagent_registry: None,
        container_registry: None,
        notification_rx: None,
        workflow_state: None,
        workflow_config: None,
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
        durable_prefix_dirty: false,
    };

    dispatch_register_tools(&mut ctx, Some("reg-1"), &tools).await;

    assert!(
        ctx.agent
            .runtime_tool_names()
            .contains(&"cov_ext".to_string())
    );
    let descriptor = ctx
        .agent
        .tool_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.name() == "cov_ext")
        .expect("cov_ext descriptor");
    assert!(matches!(
        descriptor.source,
        crate::domain::tool_descriptor::ToolSource::Uds
    ));
    assert_eq!(descriptor.owner.as_ref(), "uds:client:123");
    let state = registry.lock().unwrap();
    let state = state.get(&123).expect("client state retained");
    assert!(
        state.tool_request_rxs.is_empty(),
        "dispatch moved staged rx into task"
    );
    assert!(state.tool_request_tasks.contains_key("cov_ext"));
    assert!(
        writer_rx.try_recv().is_err(),
        "registration alone should not execute the tool"
    );
}

#[tokio::test]
async fn dispatch_register_tools_rejects_later_denied_tool_without_unloading_existing_replacement()
{
    let tmp = tempfile::TempDir::new().unwrap();
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.remove("blocked_ext");
    let mut agent = cov_agent_with_registry(registry);
    let (existing_tool, _) = create_uds_tool(
        crate::domain::tool::ToolDefinition {
            name: "weather".into(),
            description: "Existing weather".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        },
        std::time::Duration::from_secs(1),
    );
    agent.register_uds_tool_for_owner(existing_tool, "uds:client:123".into());
    let mut messages = Vec::new();
    let mut session =
        super::super::uds_session::AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let client_registry = new_client_tool_registry();
    let state = session.state_snapshot(0, None, 0, None);
    let initial_stats = super::super::uds_session::compute_session_stats(&session_key, &messages);
    let tools = [tool_reg("weather"), tool_reg("blocked_ext")];
    let mut ctx = super::super::uds::DispatchCtx {
        execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        wire_mode: super::super::uds_wire::ConnectionWireMode::legacy(),
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
            super::super::uds_snapshots::ConversationSnapshotData::default(),
        )),
        state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(state)),
        session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
        tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: Some(&mut writer),
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Idle,
        )),
        turn_control: std::sync::Arc::default(),
        broadcast_tx: None,
        _ext_registry: None,
        client_tool_registry: client_registry.clone(),
        current_client_id: 123,
        subagent_registry: None,
        container_registry: None,
        notification_rx: None,
        workflow_state: None,
        workflow_config: None,
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
        durable_prefix_dirty: false,
    };

    dispatch_register_tools(&mut ctx, Some("mixed-reject"), &tools).await;

    let descriptors = ctx.agent.tool_descriptors();
    assert!(
        descriptors
            .iter()
            .any(|descriptor| descriptor.name() == "weather")
    );
    assert!(
        !descriptors
            .iter()
            .any(|descriptor| descriptor.name() == "blocked_ext")
    );
    assert!(
        client_registry.lock().unwrap().get(&123).is_none(),
        "failed batch must not mutate UDS client state"
    );
}

#[tokio::test]
async fn forward_tool_requests_shutdown_drains_buffered_invocations_with_reason() {
    let registry = new_client_tool_registry();
    registry.lock().unwrap().entry(93).or_default();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(forward_tool_requests(
        93,
        "drainme".to_string(),
        rx,
        shutdown_rx,
        registry,
        None,
    ));

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send(crate::domain::extension_tool::ToolInvocation {
        tool_call_id: "call-3".to_string(),
        tool_name: "drainme".to_string(),
        arguments: "{}".to_string(),
        reply: reply_tx,
    })
    .await
    .unwrap();
    shutdown_tx.send("Tool unregistered").unwrap();

    let result = reply_rx.await.expect("drained reply");
    assert!(result.is_error);
    assert_eq!(result.content, "Tool unregistered");
    drop(tx);
    task.await.unwrap();
}

#[test]
fn poisoned_client_tool_registry_locks_recover_across_public_handlers() {
    let registry = new_client_tool_registry();
    let poisoned = registry.clone();
    std::thread::spawn(move || {
        let _guard = poisoned.lock().unwrap();
        panic!("poison client tool registry for coverage");
    })
    .join()
    .expect_err("thread panicked while holding lock");

    let (writer_tx, _writer_rx) = tokio::sync::mpsc::channel::<String>(1);
    register_client_writer(&registry, 501, writer_tx);
    assert!(client_writer_tx(&registry, 501).is_some());

    let (ok, ev, tools) = handle_register_tools(RegisterToolsArgs {
        client_id: 501,
        id: Some("poison-reg"),
        tools: &[tool_reg("poison_tool")],
        registry: &registry,
        core_tool_names: &HashSet::new(),
    });
    assert!(ok, "event: {}", ev.to_json_line());
    assert_eq!(tools.len(), 1);

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    {
        let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
        reg.get_mut(&501).unwrap().insert_pending(
            "poison-call".to_string(),
            "tool".to_string(),
            reply_tx,
            std::time::Duration::from_secs(30),
        );
    }
    handle_tool_result(ToolResultArgs {
        client_id: 501,
        tool_call_id: "poison-call",
        content: "ok after poison",
        is_error: false,
        registry: &registry,
    });
    let result = reply_rx.blocking_recv().expect("reply survives poison");
    assert_eq!(result.content, "ok after poison");

    let (event, removed) = handle_unregister_tools(
        501,
        Some("poison-unreg"),
        &["poison_tool".to_string()],
        &registry,
    );
    assert_eq!(removed, vec!["poison_tool".to_string()]);
    assert!(event.to_json_line().contains("poison-unreg"));

    assert!(handle_client_disconnect(501, &registry).is_empty());
}

#[tokio::test]
async fn poisoned_registry_lock_recovered_by_forwarder_paths() {
    let registry = new_client_tool_registry();
    registry.lock().unwrap().entry(777).or_default();
    let poisoned = registry.clone();
    std::thread::spawn(move || {
        let _guard = poisoned.lock().unwrap();
        panic!("poison before handle_one_request");
    })
    .join()
    .expect_err("poison panic");

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    handle_one_request(
        "poisoned_forward",
        crate::domain::extension_tool::ToolInvocation {
            tool_call_id: "forward-call".to_string(),
            tool_name: "poisoned_forward".to_string(),
            arguments: "{}".to_string(),
            reply: reply_tx,
        },
        777,
        &registry,
        &None,
    )
    .await;

    assert!(
        reply_rx.await.is_err(),
        "missing writer drops pending sender"
    );
    assert!(
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&777)
            .unwrap()
            .pending_results
            .is_empty()
    );
}
