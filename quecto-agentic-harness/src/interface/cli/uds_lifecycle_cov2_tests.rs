use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[derive(Debug)]
struct ReadOnlyProvider;

impl LlmProvider for ReadOnlyProvider {
    fn name(&self) -> &str {
        "read-only"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("read-only-ok".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

fn make_agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(ReadOnlyProvider),
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 32,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:life".into(),
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

fn loop_args<'a>(base: &'a std::path::Path, socket_path: std::path::PathBuf) -> UdsLoopArgs<'a> {
    UdsLoopArgs {
        agent: make_agent(),
        base_dir: base,
        session_key: "cli:life".into(),
        model: "stub".into(),
        ephemeral: true,
        system_prompt: "system".into(),
        socket_path,
        socket_override: None,
        session_store_override: None,
        ext_registry: None,
        persist: false,
        notification_rx: None,
        subagent_registry: None,
        workflow_state: None,
        workflow_config: None,
        broadcast_tx: None,
        provider_reload: None,
        provider_reload_inputs: None,
    }
}

#[tokio::test]
async fn read_only_provider_trait_defaults_are_exercised() {
    use crate::domain::provider::StreamEvent;

    let provider = ReadOnlyProvider;
    assert_eq!(provider.name(), "read-only");
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
        Some("read-only-ok")
    );
    let response = provider
        .chat_stream(ChatRequest {
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
        .unwrap();
    assert_eq!(response.content.as_deref(), Some("read-only-ok"));
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
        Some(StreamEvent::Done(done)) if done.content.as_deref() == Some("read-only-ok")
    ));
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn uds_loop_async_binds_multi_socket_serves_get_state_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("life.sock");
    let connect_path = socket_path.clone();
    let task = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move { uds_loop_async(loop_args(dir.path(), socket_path)).await })
    });

    let client = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match tokio::net::UnixStream::connect(&connect_path).await {
                Ok(s) => break s,
                Err(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("UDS loop should bind socket promptly");

    let (read_half, mut write_half) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(read_half).lines();
    write_half
        .write_all(
            br#"{"type":"get_state","id":"multi-state"}
"#,
        )
        .await
        .unwrap();
    write_half.flush().await.unwrap();
    let line = tokio::time::timeout(std::time::Duration::from_secs(2), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["type"], "response");
    assert_eq!(event["id"], "multi-state");
    assert_eq!(event["command"], "get_state");
    assert_eq!(event["success"], true);

    drop(write_half);
    drop(lines);
    let code = tokio::time::timeout(std::time::Duration::from_secs(2), async move {
        task.join().expect("join uds loop")
    })
    .await
    .expect("uds loop should exit after disconnect");
    assert_eq!(code, 0);
}

#[test]
fn run_uds_loop_returns_error_for_unbindable_socket_parent() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("not-a-dir");
    std::fs::write(&blocked, b"file").unwrap();
    let code = run_uds_loop(loop_args(dir.path(), blocked.join("child.sock")));
    assert_eq!(code, 1);
}

#[tokio::test]
async fn single_client_socket_override_serves_get_state() {
    let dir = tempfile::tempdir().unwrap();
    let (client_std, server_std) = std::os::unix::net::UnixStream::pair().unwrap();
    let task = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let store = FileSessionStore::new(dir.path());
            single_client_loop(
                SingleClientArgs {
                    agent: make_agent(),
                    base_dir: dir.path(),
                    messages: Vec::new(),
                    model: "stub".into(),
                    session_key: "cli:single".into(),
                    ephemeral: true,
                    system_prompt: "system".into(),
                    ext_registry: None,
                    workflow_state: None,
                    provider_reload: None,
                    provider_reload_inputs: None,
                    last_persisted_message_index: 0,
                },
                server_std,
                &store,
            )
            .await
        })
    });

    client_std.set_nonblocking(true).unwrap();
    let client = tokio::net::UnixStream::from_std(client_std).unwrap();
    let (read_half, mut write_half) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(read_half).lines();
    write_half
        .write_all(
            br#"{"type":"get_state","id":"single-state"}
"#,
        )
        .await
        .unwrap();
    write_half.flush().await.unwrap();
    let line = tokio::time::timeout(std::time::Duration::from_secs(2), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["type"], "response");
    assert_eq!(event["id"], "single-state");
    assert_eq!(event["command"], "get_state");
    assert_eq!(event["success"], true);

    drop(write_half);
    drop(lines);
    let code = tokio::time::timeout(std::time::Duration::from_secs(2), async move {
        task.join().expect("join single-client loop")
    })
    .await
    .expect("single-client loop should exit after disconnect");
    assert_eq!(code, 0);
}
