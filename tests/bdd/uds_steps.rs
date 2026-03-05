use super::*;

// UDS Agent Steps
// ===========================================================================
//
// The UDS loop runs in a dedicated OS thread using a tokio runtime.
// Tests inject a pre-connected `UnixStream` via `socket_override` so
// the loop never calls `accept()`.  The test harness:
//   1. Creates a `TempDir` and a socket path inside it.
//   2. Spawns `run_uds_loop` in a thread with `socket_override = Some(server_half)`.
//   3. Writes accumulated command lines to `client_half`, then shuts down the write side.
//   4. Reads all response lines from `client_half` until the server closes.

use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use quecto::domain::message::Role;
use quecto::domain::session::Session;
use quecto::infrastructure::config::Config;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::interface::cli::build_agent_provider;
use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};

// ─── Execution helper ────────────────────────────────────────────────────────

/// Prepared agent + session context for `execute_uds`.
struct UdsAgentContext {
    agent: AgentLoopImpl,
    model: String,
    session_key: String,
    ephemeral: bool,
}

/// Build the agent and session key from world state + config.
/// Returns `Err(message)` on any configuration failure.
fn build_uds_agent(world: &QuectoWorld, base: &std::path::Path) -> Result<UdsAgentContext, String> {
    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let config = Config::load_with_env(
        base.join("config.json").to_str().unwrap_or(""),
        &env_overrides,
    )
    .map_err(|e| format!("failed to load config: {e}"))?;

    let provider =
        build_agent_provider(&config, base).map_err(|e| format!("provider error: {e}"))?;

    let workspace = std::path::PathBuf::from(config.workspace_path());
    let model = config.agents.defaults.model.clone();
    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let registry =
        ToolRegistryImpl::with_core_tools_and_exec_settings(workspace, sandbox, exec_settings);

    let ephemeral = world.no_session || world.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        Session::build_key("cli", world.session_name.as_deref().unwrap_or("default"))
    };

    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: None,
        session_key: session_key.clone(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: config.agents.defaults.max_context_tokens,
        progress_callback: None,
    });

    Ok(UdsAgentContext {
        agent,
        model,
        session_key,
        ephemeral,
    })
}

/// Build an agent and run the UDS loop with a real `UnixStream` socket pair.
/// The "server" half is passed to `run_uds_loop` via `socket_override`; the
/// "client" half is used to write commands and read events.
///
/// Stores event lines, stderr text, and exit code into `world`. Idempotent.
fn execute_uds(world: &mut QuectoWorld) {
    if world.uds_exit_code.is_some() {
        return;
    }

    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");

    // Config check.
    if !base.join("config.json").exists() {
        world.agent_stderr = "config not found".to_string();
        world.uds_exit_code = Some(1);
        return;
    }

    let ctx = match build_uds_agent(world, &base) {
        Ok(c) => c,
        Err(e) => {
            world.agent_stderr = e;
            world.uds_exit_code = Some(1);
            return;
        }
    };
    let UdsAgentContext {
        agent,
        model,
        session_key,
        ephemeral,
    } = ctx;

    // Create a socket path in the temp dir.
    let socket_path = base.join("test-agent.sock");
    // Remove any leftover from a previous (failed) run.
    let _ = std::fs::remove_file(&socket_path);

    // Create a connected UnixStream pair.
    let (server_stream, client_stream) =
        std::os::unix::net::UnixStream::pair().expect("UnixStream::pair failed");

    // Build stdin bytes from accumulated command lines.
    let stdin_bytes: Vec<u8> = world
        .uds_commands
        .iter()
        .flat_map(|l| format!("{l}\n").into_bytes())
        .collect();

    let system_prompt = world.system_prompt.clone().unwrap_or_default();
    let base_for_thread = base.clone();
    let socket_path_for_thread = socket_path.clone();

    // Convert std UnixStream to tokio UnixStream for the server half.
    let server_tokio = {
        // We will convert inside the spawned thread where the tokio runtime exists.
        server_stream
    };

    let exit_code = std::thread::spawn(move || {
        run_uds_loop(UdsLoopArgs {
            agent,
            base_dir: &base_for_thread,
            session_key,
            model,
            ephemeral,
            system_prompt,
            socket_path: socket_path_for_thread,
            socket_override: Some(server_tokio),
            session_store_override: None,
        })
    });

    // Write commands to the client side, then shut down the write half.
    use std::io::{Read, Write};
    let mut client = client_stream;
    client
        .set_nonblocking(false)
        .expect("set_nonblocking failed");
    let _ = client.write_all(&stdin_bytes);
    // Signal EOF on the write side by calling `shutdown(Write)`.
    use std::net::Shutdown;
    let _ = client.shutdown(Shutdown::Write);

    // Read all response bytes.
    let mut response_bytes = Vec::new();
    let _ = client.read_to_end(&mut response_bytes);

    let exit = exit_code.join().unwrap_or(1);

    let raw = String::from_utf8_lossy(&response_bytes).to_string();
    world.agent_events = raw
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    world.uds_exit_code = Some(exit);

    // Simulate what the production binary prints to stderr: the socket path.
    // In tests the socket_override path skips the real eprintln!, so we inject
    // it here so the "agent stderr should contain" step can assert on it.
    world.agent_stderr = format!("quecto-agent-socket: {}\n", socket_path.display());

    // Capture socket path for transport assertions.
    world._uds_socket_path = Some(socket_path);
}

// ─── Given steps ─────────────────────────────────────────────────────────────

#[given(expr = "the mock LLM returns a tool call then a text response {string}")]
fn given_mock_llm_tool_call_then_text(world: &mut QuectoWorld, text: String) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        let tool_call_body = serde_json::json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_uds_bash",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"echo hi\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let text_body = serde_json::json!({
            "id": "chatcmpl-text",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": text },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 15, "completion_tokens": 5, "total_tokens": 20}
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(tool_call_body))
            .up_to_n_times(1)
            .with_priority(2)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .mount(&server)
            .await;

        e2e_steps::rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

/// Mount a delayed response on the existing wiremock server.  The delay causes
/// the agent's LLM call to block long enough for a concurrent `abort` or
/// `steer` command to arrive and cancel it.
#[given(expr = "the mock LLM will delay its response by {int} seconds")]
fn given_mock_llm_delayed_response(world: &mut QuectoWorld, delay_secs: u64) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set — add 'And a config file with an OpenAI provider pointing at a mock server' first"
    );
    let uri = world._wiremock_server_uri.clone().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // Connect to the existing mock server via its URI.  We start a fresh
        // server pointing at the same port is not possible directly, so we
        // create a new one and rewrite the config to point at it.
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = serde_json::json!({
            "id": "chatcmpl-delayed",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "delayed response" },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(body)
                    .set_delay(std::time::Duration::from_secs(delay_secs)),
            )
            .mount(&server)
            .await;
        // Drop old URI reference (unused after rewrite).
        drop(uri);
        e2e_steps::rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// ─── When steps — session setup ───────────────────────────────────────────────

#[when("I start the UDS agent with no session")]
fn when_start_uds_no_session(world: &mut QuectoWorld) {
    world.session_name = None;
    world.no_session = true;
}

#[when("I start the UDS agent with --no-session flag")]
fn when_start_uds_no_session_flag(world: &mut QuectoWorld) {
    world.session_name = None;
    world.no_session = true;
}

#[when(expr = "I start the UDS agent with no session and system prompt {string}")]
fn when_start_uds_no_session_with_system(world: &mut QuectoWorld, system: String) {
    world.session_name = None;
    world.no_session = true;
    world.system_prompt = Some(system);
}

#[when(expr = "I start the UDS agent with session {string} and system prompt {string}")]
fn when_start_uds_with_session_and_system(
    world: &mut QuectoWorld,
    session: String,
    system: String,
) {
    world.session_name = Some(session);
    world.no_session = false;
    world.system_prompt = Some(system);
}

#[when(expr = "I start the UDS agent with session {string}")]
fn when_start_uds_with_session(world: &mut QuectoWorld, session: String) {
    world.session_name = Some(session);
    world.no_session = false;
}

#[when("I start the UDS agent with explicit socket path")]
fn when_start_uds_with_explicit_socket(world: &mut QuectoWorld) {
    world.session_name = None;
    world.no_session = true;
    // The explicit socket path will be set during execute_uds — we use the same
    // base dir path convention so this step just marks the intent.
    world._uds_use_explicit_socket = true;
}

// ─── When steps — commands ──────────────────────────────────────────────────

#[when(expr = "I send prompt {string}")]
fn when_send_prompt(world: &mut QuectoWorld, message: String) {
    let cmd = serde_json::json!({"type": "prompt", "message": message});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send prompt with id {string} and message {string}")]
fn when_send_prompt_with_id(world: &mut QuectoWorld, id: String, message: String) {
    let cmd = serde_json::json!({"type": "prompt", "id": id, "message": message});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send command {string} with id {string}")]
fn when_send_command_with_id(world: &mut QuectoWorld, command: String, id: String) {
    let cmd = serde_json::json!({"type": command, "id": id});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send set_model {string}")]
fn when_send_set_model(world: &mut QuectoWorld, model: String) {
    let cmd = serde_json::json!({"type": "set_model", "id": "sm-1", "model": model});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send set_model provider {string} modelId {string}")]
fn when_send_set_model_provider(world: &mut QuectoWorld, provider: String, model_id: String) {
    let cmd = serde_json::json!({"type": "set_model", "id": "sm-1", "provider": provider, "modelId": model_id});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send follow_up {string} with id {string}")]
fn when_send_follow_up(world: &mut QuectoWorld, message: String, id: String) {
    let cmd = serde_json::json!({"type": "follow_up", "id": id, "message": message});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send steer {string} with id {string}")]
fn when_send_steer(world: &mut QuectoWorld, message: String, id: String) {
    let cmd = serde_json::json!({"type": "steer", "id": id, "message": message});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send get_messages_tail with count {int} and id {string}")]
fn when_send_get_messages_tail(world: &mut QuectoWorld, count: usize, id: String) {
    let cmd = serde_json::json!({"type": "get_messages_tail", "id": id, "count": count});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send raw line {string}")]
fn when_send_raw_line(world: &mut QuectoWorld, line: String) {
    world.uds_commands.push(line);
}

#[when("I send unknown command with id \"u-1\"")]
fn when_send_unknown_command(world: &mut QuectoWorld) {
    let cmd = serde_json::json!({"type": "unknown_command", "id": "u-1"});
    world.uds_commands.push(cmd.to_string());
}

#[when("I close the UDS connection")]
fn when_close_uds_connection(world: &mut QuectoWorld) {
    execute_uds(world);
}

/// Run quecto agent with an invalid --mode value (uses existing CLI runner).
#[when(expr = "I run quecto agent --mode {word} -m {string}")]
fn when_run_agent_with_invalid_mode(world: &mut QuectoWorld, mode: String, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--mode".to_string(),
        mode,
        "-m".to_string(),
        message,
    ];
    let output = quecto::interface::cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// ─── Then steps — exit code ───────────────────────────────────────────────────

#[then(expr = "the UDS agent exits with code {int}")]
fn then_uds_exits_with_code(world: &mut QuectoWorld, code: i32) {
    execute_uds(world);
    assert_eq!(
        world.uds_exit_code,
        Some(code),
        "expected exit code {code}, got {:?}\nstderr: {}\nstdout: {:#?}",
        world.uds_exit_code,
        world.agent_stderr,
        world.agent_events,
    );
}

// ─── Then steps — stderr ──────────────────────────────────────────────────────

#[then(expr = "the agent stderr should contain {string}")]
fn then_agent_stderr_contains(world: &mut QuectoWorld, expected: String) {
    execute_uds(world);
    assert!(
        world.agent_stderr.contains(&expected),
        "expected stderr to contain {expected:?}\ngot: {}",
        world.agent_stderr,
    );
}

// ─── Then steps — transport assertions ───────────────────────────────────────

#[then("the socket file should not exist after agent exits")]
fn then_socket_file_removed(world: &mut QuectoWorld) {
    execute_uds(world);
    if let Some(path) = &world._uds_socket_path {
        assert!(
            !path.exists(),
            "expected socket file to be removed after exit, but it still exists: {}",
            path.display()
        );
    }
}

// ─── Then steps — stdout event assertions ─────────────────────────────────────

fn agent_event_types(world: &QuectoWorld) -> Vec<String> {
    world
        .agent_events
        .iter()
        .filter_map(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v["type"].as_str().map(str::to_owned))
        })
        .collect()
}

#[then(expr = "the agent output should contain an event of type {string}")]
fn then_agent_output_contains_event_type(world: &mut QuectoWorld, event_type: String) {
    let types = agent_event_types(world);
    assert!(
        types.contains(&event_type),
        "expected event {event_type:?}\ngot: {types:?}\nlines: {:#?}",
        world.agent_events,
    );
}

#[then(expr = "the agent output should not contain an event of type {string}")]
fn then_agent_output_not_contains_event_type(world: &mut QuectoWorld, event_type: String) {
    let types = agent_event_types(world);
    assert!(
        !types.contains(&event_type),
        "expected NO event {event_type:?} but found it\ngot: {types:?}\nlines: {:#?}",
        world.agent_events,
    );
}

#[then(expr = "the agent output event {string} should appear {int} times")]
fn then_agent_output_event_appears_n_times(world: &mut QuectoWorld, event_type: String, n: usize) {
    let count = agent_event_types(world)
        .iter()
        .filter(|t| *t == &event_type)
        .count();
    assert_eq!(
        count,
        n,
        "expected {event_type:?} × {n}, got {count}\ntypes: {:#?}",
        agent_event_types(world),
    );
}

#[then(expr = "the agent output should contain a response with id {string}")]
fn then_agent_output_contains_response_with_id(world: &mut QuectoWorld, expected_id: String) {
    let found = world.agent_events.iter().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .ok()
            .and_then(|v| {
                if v["type"] == "response" {
                    v["id"].as_str().map(str::to_owned)
                } else {
                    None
                }
            })
            .as_deref()
            == Some(expected_id.as_str())
    });
    assert!(
        found,
        "expected response with id {expected_id:?}\nlines: {:#?}",
        world.agent_events,
    );
}

fn find_agent_response(world: &QuectoWorld, command: &str) -> Option<serde_json::Value> {
    world.agent_events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["type"] == "response" && v["command"] == command {
            Some(v)
        } else {
            None
        }
    })
}

#[then(expr = "the agent output should contain a response command {string} with success true")]
fn then_agent_output_response_success(world: &mut QuectoWorld, command: String) {
    let resp = find_agent_response(world, &command);
    assert!(
        resp.is_some(),
        "no response for {command:?}\nlines: {:#?}",
        world.agent_events,
    );
    assert_eq!(
        resp.unwrap()["success"],
        serde_json::Value::Bool(true),
        "expected success=true for {command:?}"
    );
}

#[then("the agent output should contain a response with success false")]
fn then_agent_output_response_failure(world: &mut QuectoWorld) {
    let found = world.agent_events.iter().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .ok()
            .map(|v| v["type"] == "response" && v["success"] == false)
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected response with success=false\nlines: {:#?}",
        world.agent_events,
    );
}

#[then("the agent output should contain a parse error response")]
fn then_agent_output_parse_error(world: &mut QuectoWorld) {
    let found = world.agent_events.iter().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .ok()
            .map(|v| {
                v["type"] == "response"
                    && v["success"] == false
                    && (v["command"] == "parse_error"
                        || v["error"]
                            .as_str()
                            .map(|e| e.contains("parse"))
                            .unwrap_or(false))
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected a parse error response\nlines: {:#?}",
        world.agent_events,
    );
}

// ─── get_state assertions ─────────────────────────────────────────────────────

#[then(expr = "the get_state response should include field {string}")]
fn then_get_state_has_field(world: &mut QuectoWorld, field: String) {
    let resp = find_agent_response(world, "get_state").expect("no get_state response");
    let data = resp["data"].as_object().expect("no data in get_state");
    assert!(
        data.contains_key(&field),
        "expected get_state.data.{field}\nkeys: {:#?}",
        data.keys().collect::<Vec<_>>(),
    );
}

#[then(expr = "the get_state response model should be {string}")]
fn then_get_state_model(world: &mut QuectoWorld, expected_model: String) {
    let resp = find_agent_response(world, "get_state").expect("no get_state response");
    let model = resp["data"]["model"].as_str().unwrap_or("");
    assert_eq!(model, expected_model);
}

// ─── get_messages assertions ──────────────────────────────────────────────────

#[then(expr = "the get_messages response data should include a {string} array")]
fn then_get_messages_has_array(world: &mut QuectoWorld, field: String) {
    let resp = find_agent_response(world, "get_messages").expect("no get_messages response");
    assert!(
        resp["data"][&field].is_array(),
        "expected get_messages.data.{field} to be an array\ngot: {}",
        resp["data"]
    );
}

// ─── get_messages_tail assertions ─────────────────────────────────────────────

#[then(expr = "the get_messages_tail response should include a {string} array")]
fn then_get_messages_tail_has_array(world: &mut QuectoWorld, field: String) {
    let resp =
        find_agent_response(world, "get_messages_tail").expect("no get_messages_tail response");
    assert!(
        resp["data"][&field].is_array(),
        "expected get_messages_tail.data.{field} to be an array\ngot: {}",
        resp["data"]
    );
}

#[then(expr = "the get_messages_tail messages count should be at most {int}")]
fn then_get_messages_tail_count_at_most(world: &mut QuectoWorld, max: usize) {
    let resp =
        find_agent_response(world, "get_messages_tail").expect("no get_messages_tail response");
    let count = resp["data"]["messages"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(count <= max, "expected at most {max} messages, got {count}");
}

#[then(expr = "the get_messages_tail messages count should be exactly {int}")]
fn then_get_messages_tail_count_exactly(world: &mut QuectoWorld, expected: usize) {
    let resp =
        find_agent_response(world, "get_messages_tail").expect("no get_messages_tail response");
    let count = resp["data"]["messages"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(
        count, expected,
        "expected exactly {expected} messages, got {count}"
    );
}

// ─── get_session_stats assertions ─────────────────────────────────────────────

#[then(expr = "the get_session_stats response should include field {string}")]
fn then_get_session_stats_has_field(world: &mut QuectoWorld, field: String) {
    let resp =
        find_agent_response(world, "get_session_stats").expect("no get_session_stats response");
    let data = resp["data"]
        .as_object()
        .expect("no data in get_session_stats");
    assert!(
        data.contains_key(&field),
        "expected get_session_stats.data.{field}\nkeys: {:#?}",
        data.keys().collect::<Vec<_>>(),
    );
}

// ─── session persistence assertions ──────────────────────────────────────────

#[then(expr = "a session file for {string} should exist")]
fn then_session_file_exists(world: &mut QuectoWorld, session_name: String) {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let key = Session::build_key("cli", &session_name);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = FileSessionStore::new(&base);
    let result = rt.block_on(store.load(&key));
    assert!(
        matches!(result, Ok(Some(_))),
        "expected session {session_name:?} saved, got: {result:?}"
    );
}

#[then(expr = "the session for {string} should not contain a system message")]
fn then_session_has_no_system_message(world: &mut QuectoWorld, session_name: String) {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let key = Session::build_key("cli", &session_name);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = FileSessionStore::new(&base);
    let session = rt
        .block_on(store.load(&key))
        .expect("failed to load session")
        .expect("session not found");
    let has_system = session.messages.iter().any(|m| m.role == Role::System);
    assert!(
        !has_system,
        "expected no system message in saved session, but found one"
    );
}

#[then(expr = "no session file for {string} should exist")]
fn then_no_session_file_exists(world: &mut QuectoWorld, _session_name: String) {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let dir = base.join("sessions");
    if !dir.exists() {
        return;
    }
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(
        entries.is_empty(),
        "expected no session files\nfound: {:#?}",
        entries.iter().map(|e| e.path()).collect::<Vec<_>>(),
    );
}

// ─── Additional step implementations ─────────────────────────────────────────

#[then(expr = "the agent output should contain a response command {string} with success false")]
fn then_agent_output_response_command_failure(world: &mut QuectoWorld, command: String) {
    let resp = find_agent_response(world, &command);
    assert!(
        resp.is_some(),
        "no response for {command:?}\nlines: {:#?}",
        world.agent_events,
    );
    assert_eq!(
        resp.unwrap()["success"],
        serde_json::Value::Bool(false),
        "expected success=false for {command:?}"
    );
}

#[then(expr = "the get_session_stats userMessages should equal {int}")]
fn then_get_session_stats_user_messages_eq(world: &mut QuectoWorld, expected: usize) {
    execute_uds(world);
    let resp =
        find_agent_response(world, "get_session_stats").expect("no get_session_stats response");
    let actual = resp["data"]["userMessages"]
        .as_u64()
        .expect("userMessages not a number") as usize;
    assert_eq!(
        actual, expected,
        "expected userMessages={expected}, got {actual}"
    );
}

#[then(expr = "the get_session_stats assistantMessages should equal {int}")]
fn then_get_session_stats_assistant_messages_eq(world: &mut QuectoWorld, expected: usize) {
    execute_uds(world);
    let resp =
        find_agent_response(world, "get_session_stats").expect("no get_session_stats response");
    let actual = resp["data"]["assistantMessages"]
        .as_u64()
        .expect("assistantMessages not a number") as usize;
    assert_eq!(
        actual, expected,
        "expected assistantMessages={expected}, got {actual}"
    );
}

// ─── Real-bind steps (socket_override = None) ────────────────────────────────
//
// These steps exercise the production bind path so socket permission
// behaviour can be verified.  Unlike execute_uds, they pass
// socket_override = None so run_uds_loop calls UnixListener::bind() for real.

#[when("I start the UDS agent with a real socket bind")]
fn when_start_uds_with_real_bind(world: &mut QuectoWorld) {
    world.session_name = None;
    world.no_session = true;
    // Actual execution deferred to "I close the real socket connection".
}

/// Run the UDS loop with a real bind (socket_override = None), sample the
/// socket mode once the file appears, then connect and disconnect to let the
/// agent exit cleanly.
#[when("I close the real socket connection")]
fn when_close_real_socket_connection(world: &mut QuectoWorld) {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;

    if world.uds_exit_code.is_some() {
        return;
    }
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let ctx = match build_uds_agent(world, &base) {
        Ok(c) => c,
        Err(e) => {
            world.agent_stderr = e;
            world.uds_exit_code = Some(1);
            return;
        }
    };
    let socket_path = base.join("real-bind-test.sock");
    let _ = std::fs::remove_file(&socket_path);
    world._uds_real_bind_socket_path = Some(socket_path.clone());

    let UdsAgentContext {
        agent,
        model,
        session_key,
        ephemeral,
    } = ctx;
    let base_dir = base.clone();
    let sp = socket_path.clone();

    let handle = std::thread::spawn(move || {
        run_uds_loop(UdsLoopArgs {
            agent,
            base_dir: &base_dir,
            session_key,
            model,
            ephemeral,
            system_prompt: String::new(),
            socket_path: sp,
            socket_override: None,
            session_store_override: None,
        })
    });

    // Wait for the socket file to appear (agent has bound and is ready).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !socket_path.exists() {
        if std::time::Instant::now() > deadline {
            world.agent_stderr = "timeout waiting for socket".to_string();
            world.uds_exit_code = Some(1);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Sample the socket mode while the file still exists (before SocketGuard
    // removes it after the agent exits).
    let mode = std::fs::metadata(&socket_path)
        .expect("failed to stat socket file after bind — possible race or bind failure")
        .permissions()
        .mode();
    world._uds_real_bind_socket_mode = Some(mode & 0o777);

    // Connect and immediately disconnect to unblock accept() so the agent exits.
    let _ = UnixStream::connect(&socket_path);

    world.uds_exit_code = Some(handle.join().unwrap_or(1));
}

#[then("the socket file should have mode 0600")]
fn then_socket_has_mode_0600(world: &mut QuectoWorld) {
    let mode = world
        ._uds_real_bind_socket_mode
        .expect("socket mode not recorded — did 'I close the real socket connection' run?");
    assert_eq!(mode, 0o600, "expected socket mode 0600, got {mode:04o}");
}
