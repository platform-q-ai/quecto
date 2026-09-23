use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::interface::cli::uds_single_client::{SingleClientArgs, single_client_loop};
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
        retention: None,
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
        retention: None,
        base_dir: base,
        workspace: base,
        identity: crate::domain::session_identity::SessionIdentity::from_persisted_key("cli:life"),
        model: "stub".into(),
        ephemeral: true,
        system_prompt: "system".into(),
        socket_path,
        socket_override: None,
        sessions: crate::composition::sessions::build_session_handles,
        catalogue: crate::composition::catalogue::build_catalogue_handles(base, None),
        ext_registry: None,
        lifetime: crate::domain::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects,
        notification_rx: None,
        subagent_registry: None,
        harness_lifecycle: None,
        workflow_state: None,
        workflow_config: None,
        broadcast_tx: None,
        parent_control: None,
        teardown_graph: None,
        environment_control: None,
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
                trace: None,
                admission: None,
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
            trace: None,
            admission: None,
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
            trace: None,
            admission: None,
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
    assert_eq!(event["type"], "workspace");
    let line = tokio::time::timeout(std::time::Duration::from_secs(2), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(snapshot["type"], "response");
    assert!(snapshot.get("id").is_none());
    assert_eq!(snapshot["command"], "get_state");
    assert_eq!(snapshot["success"], true);

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
            let store = crate::composition::sessions::build_session_handles(
                crate::interface::cli::uds::dispatch_session_roster_tests::loop_inputs(
                    dir.path(),
                    "cli:cov",
                ),
            );
            single_client_loop(
                SingleClientArgs {
                    agent: make_agent(),
                    workspace: dir.path(),
                    messages: Vec::new(),
                    model: "stub".into(),
                    admission_slots: Vec::new(),
                    session_key: "cli:single".into(),
                    system_prompt: "system".into(),
                    ext_registry: None,
                    subagent_registry: None,
                    workflow_state: None,
                },
                server_std,
                &store,
                &crate::composition::catalogue::build_catalogue_handles(dir.path(), None),
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
    assert_eq!(event["type"], "workspace");
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

// A published runtime, not a hand-wired AgentSession or accept-loop rig, is
// the source of these warnings. The composed catalogue handles must carry its
// store through lifecycle, dispatch and (for multi-client) the accept push.
fn publish_admission_slots(base: &std::path::Path, slots: &[&str]) {
    use crate::application::catalogue::{CatalogueSource, CredentialStatusPort, SourceEntries};
    use crate::application::provider_runtime::{
        AdmissionBindingDiagnostic, ComposeProviderRuntimeUseCase, CompositionPorts,
        ProviderRuntimeFactory, ProviderRuntimeOutcome,
    };
    use crate::domain::catalogue::{CatalogueEntry, SourceLayer};
    struct Source;
    impl CatalogueSource for Source {
        fn id(&self) -> &str {
            "test"
        }
        fn layer(&self) -> SourceLayer {
            SourceLayer::BuiltIn
        }
        fn load(&self) -> Result<SourceEntries, String> {
            Ok(SourceEntries::from(Vec::<CatalogueEntry>::new()))
        }
    }
    struct Credentials;
    impl CredentialStatusPort for Credentials {
        fn credential_available(&self, _: &CatalogueEntry) -> bool {
            true
        }
    }
    #[derive(Debug)]
    struct UnboundProvider;
    impl LlmProvider for UnboundProvider {
        fn name(&self) -> &str {
            "distinct-unbound-slot"
        }
        fn chat(
            &self,
            _request: ChatRequest<'_>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
            Box::pin(async {
                Ok(LlmResponse {
                    content: Some("usable".into()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                    thinking_blocks: vec![],
                })
            })
        }
    }
    struct Factory(Vec<String>);
    impl ProviderRuntimeFactory<(), ()> for Factory {
        fn compose_runtime(&self, _: &(), _: &()) -> Result<Arc<dyn LlmProvider>, String> {
            Ok(Arc::new(UnboundProvider))
        }
        fn compose_runtime_outcome(
            &self,
            _: &(),
            _: &(),
        ) -> Result<ProviderRuntimeOutcome, String> {
            Ok(ProviderRuntimeOutcome {
                provider: Arc::new(UnboundProvider),
                admission_binding_diagnostic: AdmissionBindingDiagnostic {
                    unbound_slots: self.0.clone(),
                },
            })
        }
    }
    ComposeProviderRuntimeUseCase::new()
        .compose_and_publish(
            &Factory(slots.iter().map(|s| (*s).to_string()).collect()),
            &(),
            &(),
            &CompositionPorts {
                sources: &[&Source],
                credentials: &Credentials,
                catalogue_store: &crate::infrastructure::catalogue_registry::snapshot_store_for(
                    base,
                ),
                runtime_store: &crate::infrastructure::catalogue_registry::runtime_store_for(base),
            },
        )
        .expect("publish usable runtime");
}

async fn admission_state(
    lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>>,
    id: Option<&str>,
    expected_slot: Option<&str>,
) {
    let line = tokio::time::timeout(std::time::Duration::from_secs(2), lines.next_line())
        .await
        .expect("state response timeout")
        .expect("read state")
        .expect("socket closed");
    let event: serde_json::Value = serde_json::from_str(&line).expect("state JSON");
    assert_eq!(event["type"], "response", "{event}");
    assert_eq!(event["command"], "get_state", "{event}");
    assert_eq!(event["success"], true, "{event}");
    match id {
        Some(id) => assert_eq!(event["id"], id, "{event}"),
        None => assert!(event.get("id").is_none(), "unsolicited state: {event}"),
    }
    let warnings = event["data"]["admissionWarnings"]
        .as_array()
        .expect("warnings array");
    match expected_slot {
        Some(slot) => {
            assert_eq!(warnings.len(), 1, "{event}");
            assert_eq!(warnings[0]["slot"], slot, "{event}");
            assert_eq!(warnings[0]["code"], "admission_binding_missing", "{event}");
        }
        None => assert!(warnings.is_empty(), "stale warning: {event}"),
    }
}

#[tokio::test]
async fn composed_runtime_warning_reaches_single_client_requested_state_and_replacement() {
    let dir = tempfile::tempdir().unwrap();
    publish_admission_slots(dir.path(), &["distinct-unbound-slot"]);
    let (client_std, server_std) = std::os::unix::net::UnixStream::pair().unwrap();
    let task = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut args = loop_args(dir.path(), dir.path().join("unused.sock"));
        args.socket_override = Some(server_std);
        rt.block_on(uds_loop_async(args))
    });
    client_std.set_nonblocking(true).unwrap();
    let client = tokio::net::UnixStream::from_std(client_std).unwrap();
    let (read_half, mut write_half) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(read_half).lines();
    let workspace: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(workspace["type"], "workspace");
    write_half
        .write_all(b"{\"type\":\"get_state\",\"id\":\"single-before\"}\n")
        .await
        .unwrap();
    admission_state(
        &mut lines,
        Some("single-before"),
        Some("distinct-unbound-slot"),
    )
    .await;
    // A later successful composition replaces the live snapshot, not the session.
    let base = workspace["path"].as_str().unwrap();
    publish_admission_slots(std::path::Path::new(base), &[]);
    write_half
        .write_all(b"{\"type\":\"get_state\",\"id\":\"single-after\"}\n")
        .await
        .unwrap();
    admission_state(&mut lines, Some("single-after"), None).await;
    drop(write_half);
    drop(lines);
    assert_eq!(task.join().unwrap(), 0);
}

#[tokio::test]
async fn composed_runtime_warning_reaches_multi_client_push_and_requested_state() {
    let dir = tempfile::tempdir().unwrap();
    publish_admission_slots(dir.path(), &["distinct-unbound-slot"]);
    let socket_path = dir.path().join("admission.sock");
    let connect_path = socket_path.clone();
    let task = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(uds_loop_async(loop_args(dir.path(), socket_path)))
    });
    let client = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(s) = tokio::net::UnixStream::connect(&connect_path).await {
                break s;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("socket bind");
    let (read_half, mut write_half) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(read_half).lines();
    let workspace: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(workspace["type"], "workspace");
    admission_state(&mut lines, None, Some("distinct-unbound-slot")).await;
    write_half
        .write_all(b"{\"type\":\"get_state\",\"id\":\"multi-before\"}\n")
        .await
        .unwrap();
    admission_state(
        &mut lines,
        Some("multi-before"),
        Some("distinct-unbound-slot"),
    )
    .await;
    let base = workspace["path"].as_str().unwrap();
    publish_admission_slots(std::path::Path::new(base), &[]);
    // A second connection reads the latest runtime even when its shared
    // turn-boundary state still describes the earlier generation.
    let replacement_client = tokio::net::UnixStream::connect(&connect_path)
        .await
        .unwrap();
    let (replacement_read, replacement_write) = tokio::io::split(replacement_client);
    let mut replacement_lines = tokio::io::BufReader::new(replacement_read).lines();
    let replacement_workspace: serde_json::Value =
        serde_json::from_str(&replacement_lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(replacement_workspace["type"], "workspace");
    admission_state(&mut replacement_lines, None, None).await;
    drop(replacement_write);
    drop(replacement_lines);
    write_half
        .write_all(b"{\"type\":\"get_state\",\"id\":\"multi-after\"}\n")
        .await
        .unwrap();
    admission_state(&mut lines, Some("multi-after"), None).await;
    drop(write_half);
    drop(lines);
    assert_eq!(task.join().unwrap(), 0);
}
