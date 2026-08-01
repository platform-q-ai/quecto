//! #1093 — BDD steps for `get_message` recall of collapsed spill refs.

use super::*;
use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use quecto::domain::agent::AgentLoop;
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message};
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::domain::session::{ContextSpillStore, Session, SessionStore};
use quecto::infrastructure::config::Config;
use quecto::infrastructure::persistence::context_spill::FileContextSpillStore;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::interface::cli::build_agent_provider;
use quecto::interface::cli::provider_reload::{ProviderReloadInputs, seeded_provider_reload};
use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};
use std::collections::HashMap;
use std::future::Future;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ISSUE_1093_SESSION: &str = "issue-1093";
const ISSUE_1093_SPILL_ID: &str = "turn1:msg:assistant";
const ISSUE_1093_FULL: &str = "full spilled content for issue 1093";

#[derive(Debug)]
struct Issue1093SeedProvider;

impl LlmProvider for Issue1093SeedProvider {
    fn name(&self) -> &str {
        "issue-1093-seed"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some(ISSUE_1093_FULL.into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

#[given("a completed turn with a collapsed message whose full content was spilled")]
fn given_collapsed_message_with_spill(world: &mut QuectoWorld) {
    seed_collapsed_session(world, true);
    start_issue_1093_agent(world);
    record_collapsed_ref_from_get_messages(world, 1);
}

#[given("a completed turn with a collapsed message whose spilled content is unavailable")]
fn given_collapsed_message_without_spill(world: &mut QuectoWorld) {
    seed_collapsed_session(world, false);
    start_issue_1093_agent(world);
    record_collapsed_ref_from_get_messages(world, 1);
}

#[when("I request the collapsed message by its stable ref via get_message")]
fn when_request_collapsed_message(world: &mut QuectoWorld) {
    send_get_message_for_recorded_ref(world, 1, "gm-1093-idle");
}

#[when("client 1 starts a later turn")]
fn when_client_1_starts_later_turn(world: &mut QuectoWorld) {
    let start = issue_1093_event_count(world, 1);
    let cmd = serde_json::json!({"type": "prompt", "message": "later slow turn"});
    write_command(world, 1, &cmd);
    wait_for_issue_1093_event_since(
        world,
        1,
        start,
        Duration::from_secs(5),
        "the later turn_start event",
        |event| event.get("type").and_then(|v| v.as_str()) == Some("turn_start"),
    );
}

#[when("client 2 requests the collapsed message by its stable ref while the agent is busy")]
fn when_client_2_requests_collapsed_message_while_busy(world: &mut QuectoWorld) {
    connect_issue_1093_client(world, 2);
    // The accept loop emits this unsolicited snapshot only after observing the
    // shared BusyGuard flag, so this is a deterministic busy-path barrier (not
    // a scheduler-dependent sleep).
    wait_for_issue_1093_event(
        world,
        2,
        Duration::from_secs(5),
        "the busy-connect get_messages snapshot",
        |event| {
            event.get("type").and_then(|v| v.as_str()) == Some("response")
                && event.get("command").and_then(|v| v.as_str()) == Some("get_messages")
                && event
                    .get("data")
                    .and_then(|d| d.get("snapshot"))
                    .and_then(|v| v.as_bool())
                    == Some(true)
        },
    );
    send_get_message_for_recorded_ref(world, 2, "gm-1093-busy");
}

#[then("the get_message response should carry the full spilled content for the requested ref")]
fn then_get_message_carries_spilled_content(world: &mut QuectoWorld) {
    let resp = world
        ._bounded_get_message_responses
        .first()
        .expect("no get_message response recorded");
    let content = resp
        .get("data")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    assert_eq!(
        content, ISSUE_1093_FULL,
        "unexpected get_message content: {resp}"
    );
}

#[then("the get_message response should not carry a recall stub for the requested ref")]
fn then_get_message_does_not_carry_recall_stub(world: &mut QuectoWorld) {
    let resp = world
        ._bounded_get_message_responses
        .first()
        .expect("no get_message response recorded");
    let content = resp
        .get("data")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    assert!(
        !content.contains("recall("),
        "stub leaked into response: {resp}"
    );
}

#[then("the get_message response should succeed for the requested ref")]
fn then_get_message_succeeds(world: &mut QuectoWorld) {
    let resp = world
        ._bounded_get_message_responses
        .first()
        .expect("no get_message response recorded");
    assert_eq!(
        resp.get("success").and_then(|v| v.as_bool()),
        Some(true),
        "{resp}"
    );
}

#[then("the get_message response should carry a recall stub for the requested ref")]
fn then_get_message_carries_recall_stub(world: &mut QuectoWorld) {
    let resp = world
        ._bounded_get_message_responses
        .first()
        .expect("no get_message response recorded");
    let content = resp
        .get("data")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    assert!(
        content.contains("recall("),
        "expected fallback stub: {resp}"
    );
}

fn seed_collapsed_session(world: &mut QuectoWorld, include_spill: bool) {
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    let session_key = Session::build_key("cli", ISSUE_1093_SESSION);
    let store = FileSessionStore::new(&base);
    let spill_store = Arc::new(FileContextSpillStore::new(base.clone()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let messages = rt.block_on(async {
        spill_store
            .clear(&session_key)
            .await
            .expect("clear prior spill");

        // Run the real agent-loop spill + count-collapse path. No test-created
        // spill id, entry, or collapsed shape: production assigns the id,
        // persists the full content under the active key, and emits the stub.
        let mut agent = AgentLoopImpl::new(AgentLoopConfig {
            provider: Arc::new(Issue1093SeedProvider),
            tool_registry: Box::new(ToolRegistryImpl::new()),
            model: "issue-1093-seed".into(),
            max_tokens: 100,
            temperature: 0.0,
            spill_store: Some(spill_store.clone()),
            session_key: session_key.clone(),
            context_collapse_after_tool_calls: u32::MAX,
            max_context_tokens: 190_000,
            progress_callback: None,
            streaming: false,
            effort: None,
            audit_log: None,
            pin_recent_turns: 0,
            context_collapse_after_messages: 0,
            model_context_window: None,
            tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
        });
        let mut messages = vec![Message::user("seed issue 1093")];
        agent
            .process(&mut messages)
            .await
            .expect("production seed turn");
        // Collapse is applied before a provider request, so run a second turn
        // to collapse the first turn's creation-time-spilled assistant reply.
        messages.push(Message::user("collapse the prior reply"));
        agent
            .process(&mut messages)
            .await
            .expect("production collapse turn");

        let collapsed = messages
            .iter()
            .find(|m| m.is_collapsed && m.spill_id.as_deref() == Some(ISSUE_1093_SPILL_ID))
            .expect("production must collapse the first spilled assistant reply");
        assert!(collapsed.content.contains("recall("));
        if !include_spill {
            spill_store
                .clear(&session_key)
                .await
                .expect("remove seeded spills for fallback scenario");
        }
        messages
    });
    let session = Session {
        key: session_key.clone(),
        messages,
        workflow_run: None,
    };
    rt.block_on(async {
        store.save(&session).await.expect("save seeded session");
    });
    world.no_session = false;
    world.session_name = Some(ISSUE_1093_SESSION.into());
    world._mc_persist = true;
    world.mc_mode = true;
    world.mc_connected_clients = vec![1];
    world._bounded_expected_body = Some(ISSUE_1093_FULL.into());
}

fn start_issue_1093_agent(world: &mut QuectoWorld) {
    if world._mc_live_socket.is_some() {
        return;
    }
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    spawn_issue_1093_agent(world, &base);
    connect_issue_1093_client(world, 1);
}

fn spawn_issue_1093_agent(world: &mut QuectoWorld, base: &std::path::Path) {
    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();
    let config_path = base.join("config.json");
    let config = Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides)
        .expect("load config");
    let http_client = reqwest::Client::new();
    let provider = build_agent_provider(&config, base, &http_client).expect("provider");
    let mut provider_reload = seeded_provider_reload(&config_path, provider.clone());
    let provider_reload_inputs =
        ProviderReloadInputs::new(config_path, base.to_path_buf(), env_overrides, http_client);
    let workspace = std::path::PathBuf::from(config.workspace_path());
    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let mut registry = quecto::infrastructure::extensions::native::build_official_tool_registry(
        workspace,
        sandbox,
        quecto::infrastructure::tools::bash::ExecOptions {
            max_capture_bytes: exec_settings,
            ..Default::default()
        },
    );
    let ext_registry = quecto::infrastructure::extensions::registry::ExtensionRegistry::new();
    quecto::interface::shared::register_bundled_native_extension_tools(
        &mut registry,
        &ext_registry,
    );
    let spill_store = Arc::new(FileContextSpillStore::new(base.to_path_buf()));
    let session_key = Session::build_key("cli", ISSUE_1093_SESSION);
    let model = config.agents.defaults.model.clone();
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: Some(spill_store),
        session_key: session_key.clone(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: config.agents.defaults.max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    let socket_path = base.join("issue-1093.sock");
    let _ = std::fs::remove_file(&socket_path);
    let socket_for_thread = socket_path.clone();
    let base_for_thread = base.to_path_buf();
    let ext_reg = Arc::new(std::sync::Mutex::new(ext_registry));
    let handle = std::thread::spawn(move || {
        run_uds_loop(UdsLoopArgs {
            agent,
            base_dir: &base_for_thread,
            session_key,
            model,
            ephemeral: false,
            system_prompt: String::new(),
            socket_path: socket_for_thread,
            socket_override: None,
            session_store_override: None,
            ext_registry: Some(ext_reg),
            persist: true,
            notification_rx: None,
            subagent_registry: None,
            workflow_state: None,
            workflow_config: None,
            broadcast_tx: None,
            provider_reload: Some(&mut provider_reload),
            provider_reload_inputs: Some(&provider_reload_inputs),
        })
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket_path.exists() {
        assert!(
            Instant::now() <= deadline,
            "timeout waiting for issue #1093 socket"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    world._mc_live_socket = Some(socket_path);
    world._mc_live_handle = Some(handle);
}

fn connect_issue_1093_client(world: &mut QuectoWorld, client_id: u32) {
    if world._mc_live_streams.contains_key(&client_id) {
        return;
    }
    let socket_path = world._mc_live_socket.clone().expect("no live socket");
    let stream = UnixStream::connect(socket_path).expect("connect issue #1093 client");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok();
    world._mc_live_streams.insert(client_id, stream);
    world.mc_client_events.entry(client_id).or_default();
    if !world.mc_connected_clients.contains(&client_id) {
        world.mc_connected_clients.push(client_id);
    }
}

fn record_collapsed_ref_from_get_messages(world: &mut QuectoWorld, client_id: u32) {
    let cmd = serde_json::json!({"type": "get_messages", "id": "gm-1093-list"});
    write_command(world, client_id, &cmd);
    let response = wait_for_issue_1093_event(
        world,
        client_id,
        Duration::from_secs(5),
        "the seeded get_messages response",
        |event| {
            event.get("type").and_then(|t| t.as_str()) == Some("response")
                && event.get("command").and_then(|c| c.as_str()) == Some("get_messages")
                && event.get("id").and_then(|id| id.as_str()) == Some("gm-1093-list")
        },
    );
    let messages = response
        .get("data")
        .and_then(|d| d.get("messages"))
        .and_then(|m| m.as_array())
        .expect("get_messages.data.messages array");
    let msg = messages
        .iter()
        .find(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("assistant")
                && m.get("content")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains("recall("))
        })
        .unwrap_or_else(|| panic!("no collapsed message in get_messages response: {response}"));
    world._bounded_recorded_ref = Some(
        msg.get("id")
            .and_then(|id| id.as_str())
            .expect("message id")
            .to_string(),
    );
}

fn send_get_message_for_recorded_ref(world: &mut QuectoWorld, client_id: u32, request_id: &str) {
    let message_id = world
        ._bounded_recorded_ref
        .clone()
        .expect("no recorded collapsed ref");
    let cmd = serde_json::json!({"type": "get_message", "id": request_id, "messageId": message_id});
    write_command(world, client_id, &cmd);
    let response = wait_for_issue_1093_event(
        world,
        client_id,
        Duration::from_secs(5),
        "the requested get_message response",
        |event| {
            event.get("type").and_then(|t| t.as_str()) == Some("response")
                && event.get("command").and_then(|c| c.as_str()) == Some("get_message")
                && event.get("id").and_then(|id| id.as_str()) == Some(request_id)
        },
    );
    world._bounded_get_message_responses = vec![response];
}

fn write_command(world: &mut QuectoWorld, client_id: u32, cmd: &serde_json::Value) {
    let stream = world
        ._mc_live_streams
        .get_mut(&client_id)
        .unwrap_or_else(|| panic!("client {client_id} is not connected"));
    writeln!(stream, "{cmd}").expect("write UDS command");
    stream.flush().expect("flush UDS command");
}

fn issue_1093_event_count(world: &QuectoWorld, client_id: u32) -> usize {
    world.mc_client_events.get(&client_id).map_or(0, Vec::len)
}

fn wait_for_issue_1093_event<F>(
    world: &mut QuectoWorld,
    client_id: u32,
    timeout: Duration,
    description: &str,
    predicate: F,
) -> serde_json::Value
where
    F: FnMut(&serde_json::Value) -> bool,
{
    let start = issue_1093_event_count(world, client_id);
    wait_for_issue_1093_event_since(world, client_id, start, timeout, description, predicate)
}

fn wait_for_issue_1093_event_since<F>(
    world: &mut QuectoWorld,
    client_id: u32,
    start: usize,
    timeout: Duration,
    description: &str,
    mut predicate: F,
) -> serde_json::Value
where
    F: FnMut(&serde_json::Value) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut cursor = start;
    loop {
        if let Some(events) = world.mc_client_events.get(&client_id) {
            while cursor < events.len() {
                let parsed = serde_json::from_str::<serde_json::Value>(&events[cursor]).ok();
                cursor += 1;
                if let Some(event) = parsed
                    && predicate(&event)
                {
                    return event;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "timeout waiting for {description} on client {client_id}; events: {:#?}",
            world.mc_client_events.get(&client_id)
        );
        read_one_issue_1093_line(world, client_id, Duration::from_millis(100));
    }
}

/// Read exactly one line without a temporary `BufReader`: a dropped buffered
/// reader can consume subsequent event bytes as read-ahead and make barriers
/// flaky. Reading a byte at a time is acceptable for this focused BDD harness.
fn read_one_issue_1093_line(
    world: &mut QuectoWorld,
    client_id: u32,
    timeout: Duration,
) -> Option<String> {
    let stream = world
        ._mc_live_streams
        .get_mut(&client_id)
        .unwrap_or_else(|| panic!("client {client_id} is not connected"));
    stream.set_read_timeout(Some(timeout)).ok();
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => bytes.push(byte[0]),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return None;
            }
            Err(_) => return None,
        }
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let line = String::from_utf8(bytes).expect("UDS output is UTF-8 JSON");
    if line.is_empty() {
        return None;
    }
    world
        .mc_client_events
        .entry(client_id)
        .or_default()
        .push(line.clone());
    Some(line)
}
