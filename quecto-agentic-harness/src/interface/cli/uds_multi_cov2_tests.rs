use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::session::SessionStore;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[derive(Debug)]
struct NeverUsedProvider;

impl LlmProvider for NeverUsedProvider {
    fn name(&self) -> &str {
        "never-used"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<crate::domain::message::LlmResponse, DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Ok(crate::domain::message::LlmResponse {
                content: Some("never-used-ok".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

struct TinyExtensionTool;

impl Tool for TinyExtensionTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: Cow::Borrowed("owned_tool"),
            description: Cow::Borrowed("owned by this UDS client"),
            parameters_schema: Cow::Borrowed(r#"{"type":"object"}"#),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(ToolResult {
                content: "ok".into(),
                is_error: false,
                image_blocks: Vec::new(),
            })
        })
    }
}

fn make_agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(NeverUsedProvider),
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 32,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:cov".into(),
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
    })
}

fn multi_args<'a>(base: &'a std::path::Path) -> MultiClientArgs<'a> {
    MultiClientArgs {
        agent: make_agent(),
        base_dir: base,
        workspace: base,
        messages: vec![Message::user("seed")],
        model: "stub".into(),
        session_key: "cli:cov".into(),
        ephemeral: true,
        system_prompt: "system from test".into(),
        ext_registry: None,
        persist: false,
        notification_rx: None,
        subagent_registry: None,
        workflow_state: None,
        workflow_config: None,
        broadcast_tx: None,
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
    }
}

async fn next_json_line(
    lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>>,
) -> serde_json::Value {
    let line = tokio::time::timeout(std::time::Duration::from_secs(2), lines.next_line())
        .await
        .expect("timed out waiting for UDS response")
        .expect("socket read should succeed")
        .expect("server should keep socket open for response");
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("invalid json line {line:?}: {e}"))
}

#[tokio::test]
async fn real_multi_client_loop_answers_read_command_then_exits_on_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("multi.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let task = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let store = FileSessionStore::new(dir.path());
            multi_client_loop(multi_args(dir.path()), listener, &store).await
        })
    });

    let client = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let (read_half, mut write_half) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(read_half).lines();

    write_half
        .write_all(
            br#"{"type":"get_state","id":"s1"}
"#,
        )
        .await
        .unwrap();
    write_half.flush().await.unwrap();

    let event = next_json_line(&mut lines).await;
    assert_eq!(event["type"], "workspace");
    let event = next_json_line(&mut lines).await;
    assert_eq!(event["type"], "response");
    assert_eq!(event["id"], "s1");
    assert_eq!(event["command"], "get_state");
    assert_eq!(event["success"], true);
    assert_eq!(event["data"]["model"], "stub");
    assert_eq!(event["data"]["sessionKey"], "cli:cov");

    drop(write_half);
    drop(lines);
    let code = tokio::time::timeout(std::time::Duration::from_secs(2), async move {
        task.join().expect("join multi-client loop")
    })
    .await
    .expect("multi-client loop should stop after last disconnect");
    assert_eq!(code, 0);
}

#[tokio::test]
async fn cov2_test_helpers_execute_their_trait_surfaces() {
    use crate::domain::provider::StreamEvent;

    let provider = NeverUsedProvider;
    assert_eq!(provider.name(), "never-used");
    assert!(provider.as_any().downcast_ref::<()>().is_some());
    assert_eq!(
        provider
            .chat(ChatRequest {
                messages: &[],
                tools: &[],
                model: "stub",
                max_tokens: 8,
                temperature: 0.0,
                session_id: None,
                tool_choice: None,
                metadata: None,
                thinking_level: None,
                cancel_flag: None,
                effort: None,
            })
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("never-used-ok")
    );
    let mut rx = provider
        .chat_stream_incremental(ChatRequest {
            messages: &[],
            tools: &[],
            model: "stub",
            max_tokens: 8,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        })
        .await;
    assert!(matches!(
        rx.recv().await,
        Some(StreamEvent::Done(done)) if done.content.as_deref() == Some("never-used-ok")
    ));
    assert!(rx.recv().await.is_none());

    let tool = TinyExtensionTool;
    assert_eq!(tool.definition().name, "owned_tool");
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert_eq!(result.content, "ok");
    assert!(!result.is_error);
}

#[tokio::test]
async fn real_multi_client_loop_unregisters_client_extension_on_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileSessionStore::new(dir.path());
    let (broadcast_tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(8);
    let live = Arc::new(std::sync::atomic::AtomicU32::new(1));
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let mut agent = make_agent();
    agent.register_uds_tool_for_owner(Arc::new(TinyExtensionTool), "uds:client:55".into());
    registry
        .lock()
        .unwrap()
        .entry(55)
        .or_default()
        .tool_names
        .insert("owned_tool".into());
    cmd_tx
        .send(ClientMessage::Disconnected(ClientDisconnected {
            client_id: 55,
        }))
        .await
        .unwrap();
    drop(cmd_tx);

    let mut session = super::super::uds_session::AgentSession::new("stub".into(), "cli:cov".into());
    let mut messages = vec![Message::user("seed")];
    let mut session_key = "cli:cov".to_string();
    let mut writer = tokio::io::sink();
    let mut ctx = super::super::uds::DispatchCtx {
        execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        wire_mode: super::super::uds_wire::ConnectionWireMode::legacy(),
        base_dir: dir.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: Arc::new(tokio::sync::RwLock::new(Default::default())),
        state_snapshot: Arc::new(tokio::sync::RwLock::new(
            session.state_snapshot(0, None, 0, None),
        )),
        session_stats_snapshot: Arc::new(tokio::sync::RwLock::new(
            super::super::uds_session::compute_session_stats("cli:cov", &[]),
        )),
        tool_catalogue_snapshot: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: Some(&mut writer),
        session_key: &mut session_key,
        session_store: &store as &dyn SessionStore,
        ephemeral: true,
        system_prompt: "",
        cancel_handle: Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Idle,
        )),
        turn_control: Arc::default(),
        broadcast_tx: Some(broadcast_tx.clone()),
        _ext_registry: None,
        client_tool_registry: registry,
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: None,
        workflow_config: None,
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
        durable_prefix_dirty: false,
    };

    run_dispatch_loop(
        &mut ctx,
        DispatchLoopArgs {
            cmd_rx,
            persist: false,
        },
        &live,
    )
    .await;

    assert!(ctx.agent.runtime_tool_names().is_empty());
    let changed = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("tool_catalogue_changed should be broadcast")
        .expect("broadcast recv");
    assert!(changed.contains("tool_catalogue_changed"), "{changed}");
}

#[test]
fn tiny_extension_tool_default_session_key_is_noop() {
    let tool = TinyExtensionTool;
    Tool::set_session_key(&tool, "client-session".into());
    assert_eq!(Tool::definition(&tool).name, "owned_tool");
}
