//! Step definitions for `uds_paged_history.feature` (#1061 / ADR-0008 part 3).
//!
//! These drive the REAL UDS server: a `run_uds_loop` thread serves a seeded
//! persisted session over a Unix socket, and a client sends `get_messages` /
//! `get_message` exactly as the TUI would. Assertions read the wire responses,
//! so paging, `hasMoreBefore`/`before` cursors, the `collapsed` stub flag, and
//! spill recall are all exercised end-to-end.

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

/// Authoritative protocol page size, imported so this suite cannot drift from
/// the protocol constant. Asserted behaviourally below, never sent on the wire.
use quecto::interface::cli::protocol::HISTORY_PAGE_SIZE as PAGE;
const PAGED_SESSION: &str = "paged-history";
const STUB_SESSION: &str = "paged-stub";
const STUB_FULL: &str = "the full demoted body recalled for paged history";
const STUB_SPILL_ID: &str = "turn1:msg:assistant";

// ── Given: seed a persisted session ─────────────────────────────────────────

#[given("a persisted UDS session with enough history to require paging")]
fn given_enough_history(world: &mut QuectoWorld) {
    seed_plain_session(world, PAGE * 3);
}

#[given("a persisted UDS session whose history exactly fits in the first slice")]
fn given_exact_fit(world: &mut QuectoWorld) {
    seed_plain_session(world, PAGE);
}

#[given("a persisted UDS session whose history continues just beyond the first slice")]
fn given_just_beyond(world: &mut QuectoWorld) {
    seed_plain_session(world, PAGE + 1);
}

#[given("a persisted UDS session with enough history to require multiple older slices")]
fn given_multiple_slices(world: &mut QuectoWorld) {
    seed_plain_session(world, PAGE * 2 + 1);
}

#[given("a persisted UDS session whose newest history slice is near the wire limit")]
fn given_near_limit_live_page(world: &mut QuectoWorld) {
    // Two pages whose count-bounded slice remains just below the shared frame
    // cap. Paging preserves every message without a transport-tail fallback.
    seed_plain_session_with_body(world, PAGE * 2, 60 * 1024);
}

#[given("a persisted UDS session containing a stubbed long message")]
fn given_stubbed_session(world: &mut QuectoWorld) {
    seed_stub_session(world);
}

#[given("a client has received history containing a stubbed long message")]
fn given_client_received_stub(world: &mut QuectoWorld) {
    seed_stub_session(world);
    start_paged_agent(world, STUB_SESSION);
    connect_paged_client(world, 1);
    let response = attach_get_messages(world, 1);
    record_stub_ref(world, &response);
}

// ── When ────────────────────────────────────────────────────────────────────

#[when("a client attaches to the session")]
fn when_client_attaches(world: &mut QuectoWorld) {
    start_paged_agent(world, active_session_name(world));
    connect_paged_client(world, 1);
    let response = attach_get_messages(world, 1);
    world._paged_response = Some(response);
}

#[when("an older client requests conversation history without a paging cursor")]
fn when_older_client_no_cursor(world: &mut QuectoWorld) {
    // A pre-paging client sends `get_messages` with no `before`; the server must
    // still hand it a usable newest slice (#1059 compatibility).
    when_client_attaches(world);
}

#[when("a client pages backward to the beginning of the session")]
fn when_pages_backward(world: &mut QuectoWorld) {
    start_paged_agent(world, active_session_name(world));
    connect_paged_client(world, 1);
    let mut response = attach_get_messages(world, 1);
    let mut collected = page_contents(&response);
    while response
        .get("hasMoreBefore")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let before = response
            .get("before")
            .and_then(|v| v.as_str())
            .expect("hasMoreBefore implies a before cursor")
            .to_string();
        response = send_get_messages(world, 1, Some(&before));
        let mut older = page_contents(&response);
        older.extend(collected);
        collected = older;
    }
    world._paged_collected = collected;
    world._paged_response = Some(response);
}

#[when("the client requests the full message by its stable message reference")]
fn when_request_full_by_ref(world: &mut QuectoWorld) {
    let message_id = world
        ._bounded_recorded_ref
        .clone()
        .expect("no recorded stub ref");
    let cmd = serde_json::json!({
        "type": "get_message",
        "id": "paged-recall",
        "messageId": message_id,
    });
    write_command(world, 1, &cmd);
    let response = wait_for_paged_event(
        world,
        1,
        Duration::from_secs(5),
        "the requested get_message response",
        |event| {
            event.get("type").and_then(|t| t.as_str()) == Some("response")
                && event.get("command").and_then(|c| c.as_str()) == Some("get_message")
                && event.get("id").and_then(|i| i.as_str()) == Some("paged-recall")
        },
    );
    world._bounded_get_message_responses = vec![response];
}

// ── Then ────────────────────────────────────────────────────────────────────

fn last_response(world: &QuectoWorld) -> &serde_json::Value {
    world
        ._paged_response
        .as_ref()
        .expect("no captured get_messages response")
}

#[then("the client should receive only a bounded newest slice of history")]
#[then("the client should receive the newest bounded history slice")]
fn then_bounded_newest_slice(world: &mut QuectoWorld) {
    let response = last_response(world);
    let contents = page_contents(response);
    assert_eq!(
        contents.len(),
        PAGE,
        "attach must be bounded to the protocol page size"
    );
    let newest = world._paged_seeded.last().expect("seeded messages");
    assert_eq!(
        contents.last(),
        Some(newest),
        "the slice must be the NEWEST messages"
    );
}

#[then("the client should receive a usable newest history slice")]
fn then_usable_newest_slice(world: &mut QuectoWorld) {
    let response = last_response(world);
    let contents = page_contents(response);
    assert!(!contents.is_empty(), "a usable slice must not be empty");
    let newest = world._paged_seeded.last().expect("seeded messages");
    assert_eq!(contents.last(), Some(newest));
}

#[then("the client should know older history can be requested")]
#[then("the client should know older history can still be requested")]
fn then_older_can_be_requested(world: &mut QuectoWorld) {
    let data = last_response(world);
    assert_eq!(
        data.get("hasMoreBefore").and_then(|v| v.as_bool()),
        Some(true),
        "older history must be advertised: {data}"
    );
    assert!(
        data.get("before").and_then(|v| v.as_str()).is_some(),
        "hasMoreBefore=true must carry a before cursor: {data}"
    );
}

#[then("the client should know the beginning of history has been reached")]
fn then_beginning_reached(world: &mut QuectoWorld) {
    let data = last_response(world);
    assert_eq!(
        data.get("hasMoreBefore").and_then(|v| v.as_bool()),
        Some(false),
        "beginning of history must be signalled: {data}"
    );
    assert_eq!(
        data.get("before"),
        Some(&serde_json::Value::Null),
        "beginning-of-history cursor must be null: {data}"
    );
}

#[then("no reachable history should be reported as trimmed")]
fn then_not_trimmed(world: &mut QuectoWorld) {
    let response = last_response(world);
    assert_ne!(
        response.get("trimmed"),
        Some(&serde_json::Value::Bool(true)),
        "paged history must never mark reachable messages as trimmed: {response}"
    );
}

#[then("the client should receive every session message")]
fn then_receive_every_message(world: &mut QuectoWorld) {
    let response = last_response(world);
    let contents = page_contents(response);
    assert_eq!(
        contents, world._paged_seeded,
        "an exact-fit session must return every message"
    );
}

#[then("the omitted oldest message should be reachable by paging")]
fn then_oldest_reachable_by_paging(world: &mut QuectoWorld) {
    let before = last_response(world)
        .get("before")
        .and_then(|v| v.as_str())
        .expect("older cursor present")
        .to_string();
    let older = send_get_messages(world, 1, Some(&before));
    let contents = page_contents(&older);
    let oldest = world
        ._paged_seeded
        .first()
        .expect("seeded messages")
        .clone();
    assert!(
        contents.contains(&oldest),
        "the omitted oldest message must be reachable by paging: {contents:?}"
    );
}

#[then("every history slice should join to the next slice without an interior gap")]
fn then_no_interior_gap(world: &mut QuectoWorld) {
    assert_eq!(
        world._paged_collected, world._paged_seeded,
        "paged slices must join into the exact chronological history"
    );
}

#[then("the collected history should contain each session message exactly once")]
fn then_each_message_once(world: &mut QuectoWorld) {
    let unique: std::collections::BTreeSet<_> = world._paged_collected.iter().collect();
    assert_eq!(
        unique.len(),
        world._paged_collected.len(),
        "collected history must be exact-once"
    );
    assert_eq!(world._paged_collected.len(), world._paged_seeded.len());
}

#[then("the collected history should include the first session message")]
fn then_includes_first(world: &mut QuectoWorld) {
    let first = world._paged_seeded.first().expect("seeded").clone();
    assert!(world._paged_collected.contains(&first));
}

#[then("the collected history should include the newest session message")]
fn then_includes_newest(world: &mut QuectoWorld) {
    let newest = world._paged_seeded.last().expect("seeded").clone();
    assert!(world._paged_collected.contains(&newest));
}

#[then("the history should show the stubbed message in place")]
fn then_stub_in_place(world: &mut QuectoWorld) {
    let response = last_response(world);
    let messages = response
        .get("data")
        .and_then(|d| d.get("messages"))
        .or_else(|| response.get("messages"))
        .and_then(|m| m.as_array())
        .expect("messages array");
    assert!(
        messages
            .iter()
            .any(|m| m.get("collapsed").and_then(|v| v.as_bool()) == Some(true)),
        "a demoted message must arrive as a stub (collapsed=true): {response}"
    );
}

#[then("the stubbed message should include a stable message reference")]
fn then_stub_has_ref(world: &mut QuectoWorld) {
    let response = last_response(world);
    let messages = response
        .get("data")
        .and_then(|d| d.get("messages"))
        .or_else(|| response.get("messages"))
        .and_then(|m| m.as_array())
        .expect("messages array");
    let stub = messages
        .iter()
        .find(|m| m.get("collapsed").and_then(|v| v.as_bool()) == Some(true))
        .expect("a collapsed stub message");
    assert!(
        stub.get("id").and_then(|v| v.as_str()).is_some(),
        "the stub must carry a stable id for recall: {stub}"
    );
}

#[then("the client should receive the full message content")]
fn then_receive_full_content(world: &mut QuectoWorld) {
    let response = world
        ._bounded_get_message_responses
        .first()
        .expect("no get_message response recorded");
    let content = response
        .get("data")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    assert_eq!(
        content, STUB_FULL,
        "recall must return the full demoted body, not the stub: {response}"
    );
    assert!(
        !content.contains("recall("),
        "recall must not return a stub: {response}"
    );
}

// ── seeding helpers ─────────────────────────────────────────────────────────

/// Write a syntactically valid provider config so `build_agent_provider`
/// succeeds. These scenarios only issue history queries and never call the LLM,
/// so an intentionally unreachable loopback endpoint avoids leaking a mock
/// server and Tokio runtime per scenario.
fn ensure_query_only_provider_config(world: &mut QuectoWorld) {
    super::e2e_steps::rewrite_config_to_uri(world, "http://127.0.0.1:9");
}

fn active_session_name(world: &QuectoWorld) -> &'static str {
    // Only two sessions exist in this feature; pick by which was seeded.
    if world.session_name.as_deref() == Some(STUB_SESSION) {
        STUB_SESSION
    } else {
        PAGED_SESSION
    }
}

fn seed_plain_session(world: &mut QuectoWorld, n: usize) {
    seed_plain_session_with_body(world, n, 0);
}

fn seed_plain_session_with_body(world: &mut QuectoWorld, n: usize, body_len: usize) {
    ensure_temp_dir(world);
    ensure_query_only_provider_config(world);
    let base = base_path(world);
    let session_key = Session::build_key("cli", PAGED_SESSION);
    let store = FileSessionStore::new(&base);
    let body = "x".repeat(body_len);
    let messages: Vec<Message> = (0..n)
        .map(|i| Message::user(format!("paged-msg-{i:04}-{body}")))
        .collect();
    world._paged_seeded = messages.iter().map(|m| m.content.clone()).collect();
    let session = Session {
        key: session_key,
        messages,
        workflow_run: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async { store.save(&session).await.expect("save seeded session") });
    world.no_session = false;
    world.session_name = Some(PAGED_SESSION.into());
    world._mc_persist = true;
    world.mc_mode = true;
    world.mc_connected_clients = vec![1];
}

#[derive(Debug)]
struct StubSeedProvider;

impl LlmProvider for StubSeedProvider {
    fn name(&self) -> &str {
        "paged-stub-seed"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some(STUB_FULL.into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// Produce a genuine ladder-collapsed message (with its full body spilled to the
/// FILE store) via the real agent loop, then persist the session — mirrors the
/// proven #1093 seeding so the served loop resolves recall against the spill.
fn seed_stub_session(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    ensure_query_only_provider_config(world);
    let base = base_path(world);
    let session_key = Session::build_key("cli", STUB_SESSION);
    let store = FileSessionStore::new(&base);
    let spill_store = Arc::new(FileContextSpillStore::new(base.clone()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let messages = rt.block_on(async {
        spill_store.clear(&session_key).await.expect("clear spill");
        let agent = AgentLoopImpl::new(AgentLoopConfig {
            provider: Arc::new(StubSeedProvider),
            tool_registry: Box::new(ToolRegistryImpl::new()),
            model: "paged-stub-seed".into(),
            max_tokens: 100,
            temperature: 0.0,
            spill_store: Some(spill_store.clone()),
            session_key: session_key.clone(),
            context_collapse_after_tool_calls: u32::MAX,
            max_context_tokens: 190_000,
            progress_callback: None,
            streaming: false,
            effort: None,
            system_prompt_provider: None,
            audit_log: None,
            pin_recent_turns: 0,
            context_collapse_after_messages: 0,
            model_context_window: None,
        });
        let mut messages = vec![Message::user("seed stub")];
        agent.process(&mut messages).await.expect("seed turn");
        messages.push(Message::user("collapse the prior reply"));
        agent.process(&mut messages).await.expect("collapse turn");
        let collapsed = messages
            .iter()
            .find(|m| m.is_collapsed && m.spill_id.as_deref() == Some(STUB_SPILL_ID))
            .expect("production must collapse the first spilled reply");
        assert!(collapsed.content.contains("recall("));
        messages
    });
    let session = Session {
        key: session_key,
        messages,
        workflow_run: None,
    };
    rt.block_on(async { store.save(&session).await.expect("save stub session") });
    world.no_session = false;
    world.session_name = Some(STUB_SESSION.into());
    world._mc_persist = true;
    world.mc_mode = true;
    world.mc_connected_clients = vec![1];
}

fn record_stub_ref(world: &mut QuectoWorld, response: &serde_json::Value) {
    // `response` is the already-unwrapped `data` object from send_get_messages.
    let messages = response
        .get("messages")
        .and_then(|m| m.as_array())
        .expect("messages array");
    // Target the collapsed ASSISTANT stub whose full body was spilled (aggressive
    // collapse can also stub the seed user turn, which has no recallable spill).
    let stub = messages
        .iter()
        .find(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("assistant")
                && m.get("collapsed").and_then(|v| v.as_bool()) == Some(true)
                && m.get("content")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains("recall("))
        })
        .expect("a collapsed assistant stub message");
    world._paged_response = Some(response.clone());
    world._bounded_recorded_ref = Some(
        stub.get("id")
            .and_then(|v| v.as_str())
            .expect("stub id")
            .to_string(),
    );
}

// ── UDS server + client plumbing (self-contained, mirrors uds_1093) ─────────

fn attach_get_messages(world: &mut QuectoWorld, client: u32) -> serde_json::Value {
    send_get_messages(world, client, None)
}

fn send_get_messages(
    world: &mut QuectoWorld,
    client: u32,
    before: Option<&str>,
) -> serde_json::Value {
    // Unique per request: `before` cursors are distinct message ids, so this
    // never collides across the backward-paging loop.
    let request_id = format!("paged-get-{}", before.unwrap_or("newest"));
    let mut cmd = serde_json::json!({ "type": "get_messages", "id": request_id });
    if let Some(before) = before {
        cmd["before"] = serde_json::Value::String(before.to_string());
    }
    write_command(world, client, &cmd);
    let response = wait_for_paged_event(
        world,
        client,
        Duration::from_secs(5),
        "the get_messages response",
        |event| {
            event.get("type").and_then(|t| t.as_str()) == Some("response")
                && event.get("command").and_then(|c| c.as_str()) == Some("get_messages")
                && event.get("id").and_then(|i| i.as_str()) == Some(request_id.as_str())
        },
    );
    // Normalize to the `data` object clients read (attach returns {data:{...}}).
    response
        .get("data")
        .cloned()
        .unwrap_or_else(|| response.clone())
}

fn page_contents(data: &serde_json::Value) -> Vec<String> {
    data.get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("content")
                        .and_then(|c| c.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn start_paged_agent(world: &mut QuectoWorld, session_name: &str) {
    if world._mc_live_socket.is_some() {
        return;
    }
    let base = base_path(world);
    spawn_paged_agent(world, &base, session_name);
}

fn spawn_paged_agent(world: &mut QuectoWorld, base: &std::path::Path, session_name: &str) {
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
    let session_key = Session::build_key("cli", session_name);
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
    let socket_path = base.join(format!("{session_name}.sock"));
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
            "timeout waiting for paged socket"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    world._mc_live_socket = Some(socket_path);
    world._mc_live_handle = Some(handle);
}

fn connect_paged_client(world: &mut QuectoWorld, client_id: u32) {
    if world._mc_live_streams.contains_key(&client_id) {
        return;
    }
    let socket_path = world._mc_live_socket.clone().expect("no live socket");
    let stream = UnixStream::connect(socket_path).expect("connect paged client");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok();
    world._mc_live_streams.insert(client_id, stream);
    world.mc_client_events.entry(client_id).or_default();
    if !world.mc_connected_clients.contains(&client_id) {
        world.mc_connected_clients.push(client_id);
    }
}

fn write_command(world: &mut QuectoWorld, client_id: u32, cmd: &serde_json::Value) {
    let stream = world
        ._mc_live_streams
        .get_mut(&client_id)
        .unwrap_or_else(|| panic!("client {client_id} is not connected"));
    writeln!(stream, "{cmd}").expect("write UDS command");
    stream.flush().expect("flush UDS command");
}

fn paged_event_count(world: &QuectoWorld, client_id: u32) -> usize {
    world.mc_client_events.get(&client_id).map_or(0, Vec::len)
}

fn wait_for_paged_event<F>(
    world: &mut QuectoWorld,
    client_id: u32,
    timeout: Duration,
    description: &str,
    mut predicate: F,
) -> serde_json::Value
where
    F: FnMut(&serde_json::Value) -> bool,
{
    let start = paged_event_count(world, client_id);
    let deadline = Instant::now() + timeout;
    let mut cursor = start;
    loop {
        if let Some(events) = world.mc_client_events.get(&client_id) {
            while cursor < events.len() {
                let parsed = serde_json::from_str::<serde_json::Value>(&events[cursor]).ok();
                cursor += 1;
                if let Some(event) = parsed {
                    if predicate(&event) {
                        return event;
                    }
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "timeout waiting for {description} on client {client_id}; events: {:#?}",
            world.mc_client_events.get(&client_id)
        );
        read_one_paged_line(world, client_id, Duration::from_millis(100));
    }
}

fn read_one_paged_line(
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
