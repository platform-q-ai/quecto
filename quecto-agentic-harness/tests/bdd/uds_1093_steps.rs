//! #1093 — BDD steps for `get_message` recall of collapsed spill refs.

use super::*;
use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use quecto::domain::message::Message;
use quecto::domain::session::{ContextSpillStore, Session, SessionStore, SpillEntry};
use quecto::infrastructure::config::Config;
use quecto::infrastructure::persistence::context_spill::FileContextSpillStore;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::interface::cli::build_agent_provider;
use quecto::interface::cli::provider_reload::{ProviderReloadInputs, seeded_provider_reload};
use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ISSUE_1093_SESSION: &str = "issue-1093";
const ISSUE_1093_SPILL_ID: &str = "turn1:msg:assistant";
const ISSUE_1093_FULL: &str = "full spilled content for issue 1093";

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
    let cmd = serde_json::json!({"type": "prompt", "message": "later slow turn"});
    write_command(world, 1, &cmd);
    std::thread::sleep(Duration::from_millis(200));
}

#[when("client 2 requests the collapsed message by its stable ref while the agent is busy")]
fn when_client_2_requests_collapsed_message_while_busy(world: &mut QuectoWorld) {
    connect_issue_1093_client(world, 2);
    drain_issue_1093_client(world, 2, Duration::from_millis(400));
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
    let spill_id = ISSUE_1093_SPILL_ID;
    let stub =
        format!("[assistant: \"full spilled content...\" (5 tokens) — recall(\"{spill_id}\")]");
    let mut msg = Message::assistant(stub, vec![]);
    msg.turn = Some(1);
    msg.is_collapsed = true;
    msg.spill_id = Some(spill_id.into());

    let store = FileSessionStore::new(&base);
    let session = Session {
        key: session_key.clone(),
        messages: vec![msg],
        workflow_run: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        store.save(&session).await.expect("save seeded session");
        let spill_store = FileContextSpillStore::new(base.clone());
        spill_store
            .clear(&session_key)
            .await
            .expect("clear prior spill");
        if include_spill {
            spill_store
                .append(
                    &session_key,
                    &SpillEntry {
                        id: spill_id.into(),
                        tool: "message".into(),
                        input_preview: String::new(),
                        tokens: 5,
                        content: ISSUE_1093_FULL.into(),
                    },
                )
                .await
                .expect("append spill entry");
        }
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
    let mut registry =
        ToolRegistryImpl::with_core_tools_and_exec_settings(workspace, sandbox, exec_settings);
    let ext_registry = quecto::infrastructure::extensions::registry::ExtensionRegistry::new();
    quecto::interface::shared::register_extension_tools(&mut registry, &ext_registry);
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
        system_prompt_provider: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
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
    drain_issue_1093_client(world, client_id, Duration::from_secs(2));
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let response = events
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| {
            v.get("type").and_then(|t| t.as_str()) == Some("response")
                && v.get("command").and_then(|c| c.as_str()) == Some("get_messages")
                && v.get("id").and_then(|id| id.as_str()) == Some("gm-1093-list")
        })
        .unwrap_or_else(|| panic!("no get_messages response; events: {events:#?}"));
    let messages = response
        .get("data")
        .and_then(|d| d.get("messages"))
        .and_then(|m| m.as_array())
        .expect("get_messages.data.messages array");
    let msg = messages
        .iter()
        .find(|m| {
            m.get("content")
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
    drain_issue_1093_client(world, client_id, Duration::from_secs(3));
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let response = events
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| {
            v.get("type").and_then(|t| t.as_str()) == Some("response")
                && v.get("command").and_then(|c| c.as_str()) == Some("get_message")
                && v.get("id").and_then(|id| id.as_str()) == Some(request_id)
        })
        .unwrap_or_else(|| panic!("no get_message response {request_id}; events: {events:#?}"));
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

fn drain_issue_1093_client(world: &mut QuectoWorld, client_id: u32, budget: Duration) {
    let Some(stream) = world._mc_live_streams.get(&client_id) else {
        return;
    };
    let reader_stream = stream.try_clone().expect("clone client stream");
    reader_stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .ok();
    let mut reader = BufReader::new(reader_stream);
    let deadline = Instant::now() + budget;
    let events = world.mc_client_events.entry(client_id).or_default();
    while Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let line = line.trim_end().to_string();
                if !line.is_empty() {
                    events.push(line);
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
}
