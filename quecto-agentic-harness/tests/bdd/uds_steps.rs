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
use quecto::interface::cli::provider_reload::{ProviderReloadInputs, seeded_provider_reload};
use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};
use wiremock::Request;

// ─── Execution helper ────────────────────────────────────────────────────────

/// Prepared agent + session context for `execute_uds`.
struct UdsAgentContext {
    agent: AgentLoopImpl,
    model: String,
    session_key: String,
    ephemeral: bool,
    ext_registry: std::sync::Arc<
        std::sync::Mutex<quecto::infrastructure::extensions::registry::ExtensionRegistry>,
    >,
    persist: bool,
    workflow_state: Option<quecto::interface::shared::WorkflowStateHandle>,
    workflow_config: Option<quecto::domain::workflow::WorkflowConfig>,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    provider_reload: quecto::interface::cli::provider_reload::ProviderReload,
    provider_reload_inputs: ProviderReloadInputs,
}

/// Build the agent and session key from world state + config.
/// Returns `Err(message)` on any configuration failure.
fn build_uds_agent(world: &QuectoWorld, base: &std::path::Path) -> Result<UdsAgentContext, String> {
    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let config_path = base.join("config.json");
    let config = Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides)
        .map_err(|e| format!("failed to load config: {e}"))?;

    let http_client = reqwest::Client::new();
    let provider = build_agent_provider(&config, base, &http_client)
        .map_err(|e| format!("provider error: {e}"))?;
    let provider_reload = seeded_provider_reload(&config_path, provider.clone());
    let provider_reload_inputs =
        ProviderReloadInputs::new(config_path, base.to_path_buf(), env_overrides, http_client);

    let workspace = std::path::PathBuf::from(config.workspace_path());
    let model = config.agents.defaults.model.clone();
    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let mut registry = quecto::infrastructure::extensions::native::build_official_tool_registry(
        workspace.clone(),
        sandbox,
        quecto::infrastructure::tools::bash::ExecOptions {
            max_capture_bytes: exec_settings,
            ..Default::default()
        },
    );

    // Create broadcast channel early so the workflow emitter can use it (#598).
    let broadcast_tx = if world._workflow_enabled {
        let (tx, _) = tokio::sync::broadcast::channel::<String>(256);
        Some(tx)
    } else {
        None
    };

    // Build workflow event emitter from broadcast channel (#598).
    let wf_emitter = broadcast_tx.as_ref().map(|tx| {
        quecto::infrastructure::tools::workflow_tool::broadcast_emitter(tx.clone(), None, None)
    });

    // Register workflow engine when scenario requests it (#568–#577).
    let workflow_state = if world._workflow_enabled {
        match quecto::interface::shared::register_workflow_tool(
            &mut registry,
            config.workflow.clone(),
            true, // guards enabled
            wf_emitter,
        ) {
            Ok(handle) => Some(handle),
            Err(e) => {
                return Err(format!("workflow init failed: {e}"));
            }
        }
    } else {
        None
    };
    let workflow_config = if world._workflow_enabled {
        Some(config.workflow.clone())
    } else {
        None
    };

    // Build empty extension registry (script extensions removed in #353).
    let ext_registry = quecto::infrastructure::extensions::registry::ExtensionRegistry::new();
    quecto::interface::shared::register_bundled_native_extension_tools(
        &mut registry,
        &ext_registry,
    );

    let ephemeral = world.no_session || world.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        Session::build_key("cli", world.session_name.as_deref().unwrap_or("default"))
    };

    let max_tool_iterations = if world.auto_mock_manual_llm && world._workflow_enabled {
        32
    } else {
        config.agents.defaults.max_tool_iterations
    };
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: None,
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
    })
    .with_max_tool_iterations(max_tool_iterations);
    // Enable streaming when the scenario has set the flag (e.g. SSE mock).
    if world._uds_streaming_enabled {
        agent.set_streaming(true);
    }

    // #1113 cache-safe prompting: workflow state is never rendered into the
    // system prompt. Mirror `--workflow` by arming the idle-boundary template
    // selector nudge instead.
    if let Some(ref wf) = workflow_state {
        if let Ok(mut engine) = wf.lock() {
            engine.set_selector_nudge(true);
        }
    }

    Ok(UdsAgentContext {
        agent,
        model,
        session_key,
        ephemeral,
        ext_registry: std::sync::Arc::new(std::sync::Mutex::new(ext_registry)),
        persist: false,
        workflow_state,
        workflow_config,
        broadcast_tx,
        provider_reload,
        provider_reload_inputs,
    })
}

/// Build an agent and run the UDS loop with a real `UnixStream` socket pair.
/// The "server" half is passed to `run_uds_loop` via `socket_override`; the
/// "client" half is used to write commands and read events.
///
/// Stores event lines, stderr text, and exit code into `world`. Idempotent.
pub(crate) fn execute_uds(world: &mut QuectoWorld) {
    if world.uds_exit_code.is_some() {
        return;
    }

    // Delegate to the real-LLM UDS executor when the scenario uses real credentials.
    if world._real_llm_uds {
        execute_real_llm_uds(world);
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

    let mut current_provider_available = true;
    let prompts: Vec<String> = world
        .uds_commands
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|command| {
            let kind = command["type"].as_str()?;
            if kind == "set_model" {
                let provider = command["provider"].as_str().or_else(|| {
                    command["model"]
                        .as_str()?
                        .split_once('/')
                        .map(|(provider, _)| provider)
                });
                current_provider_available = provider.is_none_or(|provider| {
                    matches!(
                        provider,
                        "openai" | "openai-api" | "anthropic" | "anthropic-api"
                    )
                });
                return None;
            }
            if matches!(kind, "prompt" | "follow_up") && current_provider_available {
                command["message"].as_str().map(ToString::to_string)
            } else {
                None
            }
        })
        .collect();
    if world.auto_mock_manual_llm
        && prompts
            .iter()
            .any(|prompt| prompt.contains("UDS_TOKENS_OK"))
    {
        world._uds_streaming_enabled = true;
    }
    e2e_steps::mount_auto_mock_responses_for_messages(world, &prompts);
    if world.auto_mock_manual_llm && world._workflow_enabled {
        drive_single_client_over_real_socket(world);
        return;
    }

    let ctx = match build_uds_agent(world, &base) {
        Ok(c) => c,
        Err(e) => {
            world.uds_execution_error = Some(e.clone());
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
        ext_registry,
        persist: _,
        workflow_state,
        workflow_config,
        broadcast_tx: _,
        mut provider_reload,
        provider_reload_inputs,
    } = ctx;

    if world.uds_add_fireworks_before_loop {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let fireworks_uri = world
            ._fireworks_mock_uri
            .as_ref()
            .expect("fireworks mock URI not set");
        let config_path = base.join("config.json");
        let mut config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).expect("read config"))
                .expect("parse config");
        let providers = config
            .as_object_mut()
            .expect("config object")
            .entry("providers")
            .or_insert_with(|| serde_json::json!({}));
        let providers = providers.as_object_mut().expect("providers object");
        providers.insert(
            "openai_compatible".to_string(),
            serde_json::json!({
                "endpoints": [{
                    "prefix": "fireworks",
                    "api_base": fireworks_uri,
                    "api_key": "sk-fireworks",
                    "allow_remote_http": true,
                }]
            }),
        );
        std::fs::write(
            &config_path,
            serde_json::to_string_pretty(&config).expect("serialize config"),
        )
        .expect("write Fireworks config");
    }
    if world.uds_invalid_config_before_loop {
        std::fs::write(base.join("config.json"), "{ invalid json").expect("write invalid config");
    }

    // Create a socket path in the temp dir.
    let socket_path = base.join("test-agent.sock");
    // Remove any leftover from a previous (failed) run.
    let _ = std::fs::remove_file(&socket_path);

    // Create a connected UnixStream pair.
    let (server_stream, client_stream) =
        std::os::unix::net::UnixStream::pair().expect("UnixStream::pair failed");

    // Build stdin bytes from the accumulated commands, in the scenario's
    // wire framing (#1059): legacy newline lines by default, length-prefixed
    // frames (or raw garbage) when the scenario scripted such a client.
    let stdin_bytes: Vec<u8> = crate::uds_framing_steps::build_wire_client_bytes(world);

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
            ext_registry: Some(ext_registry),
            persist: false,
            notification_rx: None,
            subagent_registry: None,
            workflow_state,
            workflow_config,
            broadcast_tx: None,
            provider_reload: Some(&mut provider_reload),
            provider_reload_inputs: Some(&provider_reload_inputs),
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

    let exit = match exit_code.join() {
        Ok(code) => code,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "UDS helper thread panicked".to_string()
            };
            world.uds_execution_error = Some(msg.clone());
            1
        }
    };

    world.agent_events = crate::uds_framing_steps::parse_wire_events(world, &response_bytes);
    world.uds_exit_code = Some(exit);

    // Simulate what the production binary prints to stderr: the announcement
    // (protocol-version line + socket-path line, #1059). In tests the
    // socket_override path skips the real eprint!, so we inject the SAME
    // production-built string here so announcement steps assert on the real
    // format.
    world.agent_stderr = quecto::interface::cli::uds_wire::socket_announcement(&socket_path);

    // Capture socket path for transport assertions.
    world._uds_socket_path = Some(socket_path);
}

/// Drive a single client end-to-end over a *real* Unix socket, so the request
/// actually flows through the multi-client dispatch loop
/// (`uds_multi::handle_client_msg`) rather than the `socket_override`
/// single-client shortcut. Collected events land in `world.agent_events`.
fn drive_single_client_over_real_socket(world: &mut QuectoWorld) {
    world.mc_mode = true;
    world.mc_connected_clients = vec![1];
    world.mc_disconnected_clients.clear();
    world
        .mc_client_commands
        .insert(1, world.uds_commands.clone());
    world.mc_client_events.entry(1).or_default();

    execute_multi_client_uds(world);

    world.uds_exit_code = world.mc_exit_code;
    world.agent_events = world.mc_client_events.get(&1).cloned().unwrap_or_default();
    world.stdout = world.agent_events.join("\n");
}

fn uds_parse_error_text_from_events(events: &[String]) -> String {
    events
        .iter()
        .find_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            if v["type"] == "response" && v["command"] == "parse_error" {
                v["error"].as_str().map(str::to_owned)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("expected parse_error response in events: {events:#?}"))
}

fn uds_event_types_from_lines(events: &[String]) -> Vec<String> {
    events
        .iter()
        .filter_map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v["type"].as_str().map(str::to_owned))
        })
        .collect()
}

fn reset_uds_run(world: &mut QuectoWorld, commands: Vec<String>) {
    world.uds_commands = commands;
    world.agent_events.clear();
    world.agent_stderr.clear();
    world.uds_execution_error = None;
    world.uds_exit_code = None;
    world.stdout.clear();
    world.mc_mode = false;
    world.mc_exit_code = None;
    world.mc_client_events.clear();
    world.mc_client_commands.clear();
    world.mc_connected_clients.clear();
    world.mc_disconnected_clients.clear();
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

#[given(expr = "the config default model is {string}")]
fn given_config_default_model(world: &mut QuectoWorld, model: String) {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let config_path = base.join("config.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).expect("read config"))
            .expect("parse config");
    config["agents"]["defaults"]["model"] = serde_json::json!(model);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write config");
}

#[given(expr = "a models registry with Fireworks model {string}")]
fn given_models_registry_with_fireworks_model(world: &mut QuectoWorld, model_id: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let fireworks = wiremock::MockServer::start().await;
        let fireworks_uri = fireworks.uri();
        let fw_body = serde_json::json!({
            "id": "chatcmpl-fireworks",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "fireworks registry ok" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 }
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(fw_body))
            .mount(&fireworks)
            .await;
        world._fireworks_mock_uri = Some(fireworks_uri.clone());
        world.fireworks_mock_server_ref = Some(Box::leak(Box::new(fireworks)));
    });
    std::mem::forget(rt);

    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("temp base directory not set");
    let registry = serde_json::json!({
        "providers": {
            "fireworks": {
                "baseUrl": world._fireworks_mock_uri.as_ref().unwrap(),
                "apiKey": "sk-fireworks",
                "api": "openai-completions",
                "models": [{ "id": model_id, "name": "Fireworks Test Model" }]
            }
        }
    });
    std::fs::write(
        base.join("models.json"),
        serde_json::to_string_pretty(&registry).unwrap(),
    )
    .expect("write models.json");
}

#[given("the config file will be updated to add a Fireworks provider before the UDS command loop")]
fn given_config_add_fireworks_before_loop(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let fireworks = wiremock::MockServer::start().await;
        let fireworks_uri = fireworks.uri();
        let fw_body = serde_json::json!({
            "id": "chatcmpl-fireworks",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "fireworks ok" },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(fw_body))
            .mount(&fireworks)
            .await;
        world._fireworks_mock_uri = Some(fireworks_uri);
        world.fireworks_mock_server_ref = Some(Box::leak(Box::new(fireworks)));
    });
    std::mem::forget(rt);
    world.uds_add_fireworks_before_loop = true;
}

#[given("the config file is replaced with invalid JSON before the UDS command loop")]
fn given_config_replaced_with_invalid_json(world: &mut QuectoWorld) {
    world.uds_invalid_config_before_loop = true;
}

#[then("the Fireworks provider should have received a chat completion request")]
fn then_fireworks_received_request(world: &mut QuectoWorld) {
    let server = world
        .fireworks_mock_server_ref
        .expect("fireworks mock server not configured");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let requests = rt
        .block_on(server.received_requests())
        .expect("received requests should be available");
    std::mem::forget(rt);
    let count = requests
        .iter()
        .filter(|request: &&Request| {
            request.method.as_str() == "POST" && request.url.path() == "/chat/completions"
        })
        .count();
    assert!(
        count > 0,
        "expected Fireworks mock to receive a chat completion request; requests: {requests:#?}\nevents: {:#?}\nstderr: {}\nexecution_error: {:?}\nconfig: {}",
        world.agent_events,
        world.agent_stderr,
        world.uds_execution_error,
        std::fs::read_to_string(
            world
                .cli_context
                .base_dir
                .as_ref()
                .unwrap()
                .join("config.json")
        )
        .unwrap_or_default()
    );
}

/// Mount a delayed response on the existing wiremock server.  The delay causes
/// the agent's LLM call to block long enough for a concurrent `abort` or
/// `steer` command to arrive and cancel it.
#[given(expr = "the mock LLM will delay its response by {int} seconds")]
fn given_mock_llm_delayed_response(world: &mut QuectoWorld, delay_secs: u64) {
    world._bounded_delay_secs = Some(delay_secs);
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

#[when(expr = "I send get_state with id {string}")]
fn when_send_get_state_with_id(world: &mut QuectoWorld, id: String) {
    let cmd = serde_json::json!({"type": "get_state", "id": id});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send rewind_to messageIndex {int} with id {string}")]
fn when_send_rewind_to(world: &mut QuectoWorld, message_index: usize, id: String) {
    let cmd = serde_json::json!({"type": "rewind_to", "id": id, "messageIndex": message_index});
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

#[when(expr = "I send set_effort {string}")]
fn when_send_set_effort(world: &mut QuectoWorld, effort: String) {
    // The id is generated internally: no Then step correlates on it, so it is
    // kept out of the scenario text (mirrors `I send set_model {string}`).
    let cmd = serde_json::json!({"type": "set_effort", "id": "se-auto", "effort": effort});
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

#[when(expr = "I send get_messages with count {int} and id {string}")]
fn when_send_get_messages_with_count(world: &mut QuectoWorld, count: usize, id: String) {
    let cmd = serde_json::json!({"type": "get_messages", "id": id, "count": count});
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

/// Close the connection after driving the queued commands over a *real* socket,
/// so they flow through the multi-client dispatch loop
/// (`uds_multi::handle_client_msg`) instead of the single-client
/// `socket_override` shortcut (#994 criterion 2).
#[when("I close the UDS connection through the multi-client dispatch loop")]
fn when_close_uds_connection_multi_client(world: &mut QuectoWorld) {
    drive_single_client_over_real_socket(world);
}

#[when("I send the same malformed command through both UDS connection modes")]
fn when_same_malformed_command_through_both_modes(world: &mut QuectoWorld) {
    let malformed = "{not valid json".to_string();

    reset_uds_run(world, vec![malformed.clone()]);
    execute_uds(world);
    let single = uds_parse_error_text_from_events(&world.agent_events);

    reset_uds_run(world, vec![malformed]);
    drive_single_client_over_real_socket(world);
    let multi = uds_parse_error_text_from_events(&world.agent_events);

    world.uds_compare_parse_errors = Some((single, multi));
}

#[when("I send the same prompt through both UDS event delivery modes")]
fn when_same_prompt_through_both_event_delivery_modes(world: &mut QuectoWorld) {
    let prompt = serde_json::json!({"type": "prompt", "message": "hello"}).to_string();

    reset_uds_run(world, vec![prompt.clone()]);
    execute_uds(world);
    let writer = uds_event_types_from_lines(&world.agent_events);

    reset_uds_run(world, vec![prompt]);
    drive_single_client_over_real_socket(world);
    let broadcast = uds_event_types_from_lines(&world.agent_events);

    world.uds_compare_event_types = Some((writer, broadcast));
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

/// #1047: every line the agent emitted must be receivable by the TUI client,
/// i.e. fit within the protocol event line cap INCLUDING its trailing newline
/// (captured `agent_events` lines have the newline stripped, hence `< cap`).
#[then("every emitted event line should fit within the event line cap")]
fn then_every_event_line_fits_cap(world: &mut QuectoWorld) {
    use quecto::interface::cli::protocol::EVENT_LINE_CAP_BYTES;
    assert!(
        !world.agent_events.is_empty(),
        "expected the agent to have emitted event lines"
    );
    for line in &world.agent_events {
        assert!(
            line.len() < EVENT_LINE_CAP_BYTES,
            "an emitted event line exceeds the event line cap and would be \
             dropped unread by the TUI client (#1047): {} bytes (cap {}), \
             line head: {}…",
            line.len() + 1,
            EVENT_LINE_CAP_BYTES,
            &line[..80.min(line.len())],
        );
    }
}

#[then(expr = "the agent output should contain an event of type {string}")]
fn then_agent_output_contains_event_type(world: &mut QuectoWorld, event_type: String) {
    // Framing scenarios (#1059) model the disconnect in a Given, so the
    // deferred run is triggered here instead of by an explicit close step.
    crate::uds_framing_steps::ensure_wire_client_executed(world);
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

/// Find the first response event for a given command name.
pub fn find_agent_response(world: &QuectoWorld, command: &str) -> Option<serde_json::Value> {
    world.agent_events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["type"] == "response" && v["command"] == command {
            Some(v)
        } else {
            None
        }
    })
}

#[then(expr = "the agent output should contain a response command {string} with model {string}")]
fn then_agent_output_response_contains_model(
    world: &mut QuectoWorld,
    command: String,
    model: String,
) {
    let resp = find_agent_response(world, &command).unwrap_or_else(|| {
        panic!(
            "no response for {command:?}\nlines: {:#?}",
            world.agent_events
        )
    });
    let models = resp
        .get("data")
        .and_then(|d| d.get("models"))
        .and_then(|v| v.as_array())
        .expect("response data.models array");
    assert!(
        models
            .iter()
            .any(|m| m.get("model").and_then(|v| v.as_str()) == Some(model.as_str())),
        "expected response command {command:?} to contain model {model:?}\nresponse: {resp:#?}"
    );
}

#[then(expr = "the agent output should contain a response command {string} with success true")]
fn then_agent_output_response_success(world: &mut QuectoWorld, command: String) {
    let resp = find_agent_response(world, &command);
    assert!(
        resp.is_some(),
        "no response for {command:?}\nlines: {:#?}\nexecution_error: {:?}\nstderr: {}\nexit: {:?}",
        world.agent_events,
        world.uds_execution_error,
        world.agent_stderr,
        world.uds_exit_code,
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

/// #994 criterion 2: the multi-client dispatch loop must preserve the detailed
/// serde parse-error text (`parse error: …`), consistent with the single-client
/// loop, rather than substituting a generic `"invalid JSON command"` string.
#[then("the parse error response should preserve the detailed error text")]
fn then_parse_error_preserves_detail(world: &mut QuectoWorld) {
    let err = world.agent_events.iter().find_map(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .ok()
            .and_then(|v| {
                if v["type"] == "response" && v["command"] == "parse_error" {
                    v["error"].as_str().map(str::to_owned)
                } else {
                    None
                }
            })
    });
    let err = err.unwrap_or_else(|| {
        panic!(
            "expected a parse_error response\nlines: {:#?}",
            world.agent_events
        )
    });
    assert!(
        err.contains("parse error:"),
        "parse_error must preserve the detailed serde text, got: {err:?}\nlines: {:#?}",
        world.agent_events,
    );
    assert_ne!(
        err, "invalid JSON command",
        "the generic placeholder text must not be emitted (#994 criterion 2)"
    );
}

#[then("both responses should contain the same parse error text")]
fn then_both_responses_same_parse_error_text(world: &mut QuectoWorld) {
    let (single, multi) = world
        .uds_compare_parse_errors
        .as_ref()
        .expect("missing captured parse-error comparison");
    assert_eq!(multi, single);
    assert!(
        single.contains("parse error:"),
        "unexpected parse error: {single:?}"
    );
    assert_ne!(single, "invalid JSON command");
}

#[then("both clients should receive the same event sequence")]
fn then_both_clients_receive_same_event_sequence(world: &mut QuectoWorld) {
    let (writer, broadcast) = world
        .uds_compare_event_types
        .as_ref()
        .expect("missing captured event sequence comparison");
    assert_eq!(broadcast, writer);
    assert!(
        writer.iter().any(|t| t == "agent_start") && writer.iter().any(|t| t == "agent_end"),
        "event sequence should include the visible agent lifecycle: {writer:?}"
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

// ─── set_effort assertions (#1067) ────────────────────────────────────────────

#[then(expr = "the get_state response effort should be {string}")]
fn then_get_state_effort(world: &mut QuectoWorld, expected_effort: String) {
    let resp = find_agent_response(world, "get_state").expect("no get_state response");
    let effort = resp["data"]["effort"].as_str().unwrap_or("");
    assert_eq!(
        effort, expected_effort,
        "get_state data.effort mismatch\ndata: {}",
        resp["data"]
    );
}

/// Unset effort surfaces as an explicit null (never a missing key), so
/// clients can distinguish "provider default" from a missing capability.
#[then("the get_state response effort should be unset")]
fn then_get_state_effort_unset(world: &mut QuectoWorld) {
    let resp = find_agent_response(world, "get_state").expect("no get_state response");
    let data = resp["data"]
        .as_object()
        .expect("get_state response has no data object");
    assert!(
        data.contains_key("effort"),
        "get_state data must include an effort key\ndata: {}",
        resp["data"]
    );
    assert!(
        data["effort"].is_null(),
        "unset effort must be null, got: {}",
        data["effort"]
    );
}

/// Scans ALL set_effort responses (not just the first) for a failed one whose
/// error message lists EXACTLY the expected provider-scoped vocabulary
/// (#1067). Token-exact set comparison — substring matching would let "xhigh"
/// satisfy a missing "high" entry.
#[then(
    expr = "the agent output should contain a failed set_effort response listing the valid effort levels {string}"
)]
fn then_failed_set_effort_lists_levels(world: &mut QuectoWorld, expected_csv: String) {
    let expected: Vec<&str> = expected_csv.split(',').map(str::trim).collect();
    let found = world.agent_events.iter().any(|l| {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else {
            return false;
        };
        if v["type"] != "response" || v["command"] != "set_effort" || v["success"] != false {
            return false;
        }
        let err = v["error"].as_str().unwrap_or("");
        let Some((_, listed)) = err.split_once("valid levels:") else {
            return false;
        };
        let listed: Vec<&str> = listed.split(',').map(str::trim).collect();
        listed == expected
    });
    assert!(
        found,
        "expected a failed set_effort response listing exactly [{expected_csv}]\nlines: {:#?}",
        world.agent_events,
    );
}

#[when(expr = "client {int} sends set_effort {string}")]
fn when_client_sends_set_effort(world: &mut QuectoWorld, client_id: u32, effort: String) {
    let cmd = serde_json::json!({"type": "set_effort", "id": "se-auto", "effort": effort});
    world
        .mc_client_commands
        .entry(client_id)
        .or_default()
        .push(cmd.to_string());
}

#[then(expr = "client {int} get_state response effort should be {string}")]
fn then_client_get_state_effort(world: &mut QuectoWorld, client_id: u32, expected: String) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let effort = events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["type"] == "response" && v["command"] == "get_state" {
            Some(v["data"]["effort"].as_str().unwrap_or("").to_string())
        } else {
            None
        }
    });
    assert_eq!(
        effort.as_deref(),
        Some(expected.as_str()),
        "client {client_id} get_state data.effort mismatch\nevents: {events:#?}"
    );
}

/// Anthropic mock that CAPTURES requests (#1067): unlike the fire-and-forget
/// mocks, keeps a leaked server ref so Then steps can inspect what the agent
/// actually sent to the LLM.
#[given(expr = "a capturing Anthropic mock LLM returning text {string}")]
fn given_capturing_anthropic_mock(world: &mut QuectoWorld, content: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = serde_json::json!({
            "id": "msg_mock",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": content }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        e2e_steps::rewrite_config_to_provider_uri(world, "anthropic", &new_uri);
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        world.wiremock_server_ref = Some(leaked);
    });
    std::mem::forget(rt);
}

fn captured_anthropic_bodies(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let server = world
        .wiremock_server_ref
        .expect("no capturing Anthropic mock configured");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let requests = rt
        .block_on(server.received_requests())
        .expect("request recording not enabled");
    std::mem::forget(rt);
    requests
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/v1/messages")
        .map(|r| serde_json::from_slice(&r.body).unwrap_or_default())
        .collect()
}

#[then(expr = "Anthropic request {int} should carry reasoning effort {string}")]
fn then_anthropic_request_effort(world: &mut QuectoWorld, index: usize, expected: String) {
    let bodies = captured_anthropic_bodies(world);
    let body = bodies.get(index - 1).unwrap_or_else(|| {
        panic!(
            "no Anthropic request {index}; got {} requests",
            bodies.len()
        )
    });
    assert_eq!(
        body["output_config"]["effort"].as_str(),
        Some(expected.as_str()),
        "request {index} must carry output_config.effort {expected:?}, body: {body}"
    );
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

#[then(expr = "the get_messages response data should include a {string} array with {int} messages")]
fn then_get_messages_array_len(world: &mut QuectoWorld, field: String, expected: usize) {
    let resp = find_agent_response(world, "get_messages").expect("no get_messages response");
    let len = resp["data"][&field]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(len, expected, "unexpected get_messages.data.{field} length");
}

#[then(expr = "the get_messages messages count should be at most {int}")]
fn then_get_messages_count_at_most(world: &mut QuectoWorld, max: usize) {
    let resp = find_agent_response(world, "get_messages").expect("no get_messages response");
    let messages = resp["data"]["messages"]
        .as_array()
        .expect("get_messages.data.messages array");
    let count = messages.len();
    assert!(count <= max, "expected at most {max} messages, got {count}");
}

#[then(expr = "the get_messages messages count should be exactly {int}")]
fn then_get_messages_count_exactly(world: &mut QuectoWorld, expected: usize) {
    let resp = find_agent_response(world, "get_messages").expect("no get_messages response");
    let count = resp["data"]["messages"]
        .as_array()
        .expect("get_messages.data.messages array")
        .len();
    assert_eq!(
        count, expected,
        "expected exactly {expected} messages, got {count}"
    );
}

#[then(expr = "the get_messages messages should contain content {string} before {string}")]
fn then_get_messages_content_order(world: &mut QuectoWorld, first: String, second: String) {
    let resp = find_agent_response(world, "get_messages").expect("no get_messages response");
    let messages = resp["data"]["messages"]
        .as_array()
        .expect("get_messages.data.messages array");
    let contents: Vec<String> = messages
        .iter()
        .filter_map(|m| m["content"].as_str().map(str::to_owned))
        .collect();
    let first_idx = contents
        .iter()
        .position(|c| c.contains(&first))
        .unwrap_or_else(|| panic!("missing content {first:?} in {contents:?}"));
    let second_idx = contents
        .iter()
        .position(|c| c.contains(&second))
        .unwrap_or_else(|| panic!("missing content {second:?} in {contents:?}"));
    assert!(
        first_idx < second_idx,
        "expected {first:?} before {second:?}: {contents:?}"
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

fn uds_session_key(session_name: &str) -> String {
    Session::build_key("cli", session_name)
}

fn save_uds_session(world: &QuectoWorld, session: &Session) {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = FileSessionStore::new(&base);
    rt.block_on(store.save(session))
        .expect("failed to save session");
}

fn load_uds_session(world: &QuectoWorld, session_name: &str) -> Session {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let key = uds_session_key(session_name);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = FileSessionStore::new(&base);
    rt.block_on(store.load(&key))
        .expect("failed to load session")
        .expect("session not found")
}

#[given(
    expr = "session {string} already contains user message {string} and assistant message {string}"
)]
fn given_session_already_contains_messages(
    world: &mut QuectoWorld,
    session_name: String,
    user: String,
    assistant: String,
) {
    save_uds_session(
        world,
        &Session {
            key: uds_session_key(&session_name),
            messages: vec![Message::user(user), Message::assistant(assistant, vec![])],
            workflow_run: None,
        },
    );
}

#[given(expr = "session {string} has workflow {string} with {int} completed steps")]
fn given_session_has_workflow_progress(
    world: &mut QuectoWorld,
    session_name: String,
    template_id: String,
    completed: usize,
) {
    save_uds_session(
        world,
        &Session {
            key: uds_session_key(&session_name),
            messages: vec![Message::user("workflow context")],
            workflow_run: Some(quecto::domain::workflow::WorkflowRunPersisted {
                template_id: Some(template_id),
                done: (0..7).map(|i| i < completed).collect(),
                active_issue: None,
            }),
        },
    );
    world._workflow_enabled = true;
}

#[then(
    expr = "the session for {string} should contain user message {string} and assistant message {string}"
)]
fn then_session_should_contain_messages(
    world: &mut QuectoWorld,
    session_name: String,
    user: String,
    assistant: String,
) {
    let session = load_uds_session(world, &session_name);
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.content == user),
        "saved session should retain loaded user message {user:?}: {:#?}",
        session.messages
    );
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant && m.content == assistant),
        "saved session should include new assistant message {assistant:?}: {:#?}",
        session.messages
    );
}

#[then(expr = "the session {string} should retain workflow {string} with {int} completed steps")]
fn then_session_should_retain_workflow_progress(
    world: &mut QuectoWorld,
    session_name: String,
    template_id: String,
    completed: usize,
) {
    let session = load_uds_session(world, &session_name);
    let run = session
        .workflow_run
        .expect("workflow_run should be persisted after UDS load/save");
    assert_eq!(run.template_id.as_deref(), Some(template_id.as_str()));
    assert_eq!(
        run.done.iter().filter(|d| **d).count(),
        completed,
        "completed workflow steps should survive load/save"
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
        ext_registry,
        persist: _,
        workflow_state,
        workflow_config,
        broadcast_tx: _,
        mut provider_reload,
        provider_reload_inputs,
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
            ext_registry: Some(ext_registry),
            persist: false,
            notification_rx: None,
            subagent_registry: None,
            workflow_state,
            workflow_config,
            broadcast_tx: None,
            provider_reload: Some(&mut provider_reload),
            provider_reload_inputs: Some(&provider_reload_inputs),
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

// ─── Socket path length validation ───────────────────────────────────────────

#[when("I run quecto agent --mode uds with an overlong socket path")]
fn when_run_agent_overlong_socket(world: &mut QuectoWorld) {
    // Build a path that exceeds the 104-byte sockaddr_un.sun_path limit.
    let base = base_path(world);
    let long_name = "x".repeat(110);
    let long_path = base.join(format!("{long_name}.sock"));
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--mode".to_string(),
        "uds".to_string(),
        "--socket".to_string(),
        long_path.to_string_lossy().into_owned(),
    ];
    let output = quecto::interface::cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    // Set uds_exit_code so execute_uds() short-circuits in Then steps.
    world.uds_exit_code = Some(output.exit_code);
    world.stdout = output.stdout;
    world.agent_stderr.push_str(&output.stderr);
    world.stderr = output.stderr;
}

// ─── Token streaming steps ──────────────────────────────────────────────────

#[given(expr = "UDS streaming is enabled")]
fn given_uds_streaming_enabled(world: &mut QuectoWorld) {
    world._uds_streaming_enabled = true;
}

#[given(expr = "the mock LLM returns a streaming response with tokens {string} {string}")]
fn given_mock_llm_streaming_tokens(world: &mut QuectoWorld, tok1: String, tok2: String) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set — ensure a config step ran first"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // Build SSE body: two token deltas + [DONE]
        let sse_body = format!(
            "data: {{\"id\":\"chatcmpl-1\",\"choices\":[{{\"delta\":{{\"content\":\"{tok1}\"}}}}]}}\n\n\
             data: {{\"id\":\"chatcmpl-1\",\"choices\":[{{\"delta\":{{\"content\":\"{tok2}\"}}}}]}}\n\n\
             data: [DONE]\n\n"
        );

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        super::e2e_steps::rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[then(expr = "the agent output should contain a token event with {string}")]
fn then_agent_output_contains_token(world: &mut QuectoWorld, expected: String) {
    execute_uds(world);
    let events = &world.agent_events;
    let found = events.iter().any(|line| {
        if let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) {
            ev["type"].as_str() == Some("token") && ev["token"].as_str() == Some(&expected)
        } else {
            false
        }
    });
    assert!(
        found,
        "expected a token event with {expected:?} in events:\n{events:#?}"
    );
}

#[then(expr = "the agent output should contain a turn_end event with content {string}")]
fn then_agent_output_contains_turn_end(world: &mut QuectoWorld, expected: String) {
    execute_uds(world);
    let events = &world.agent_events;
    // Reconstruct the streamed assistant text so the "content emptied" branch is
    // tied to the expected body actually being OBSERVED — not merely to any refs
    // existing, which greened every expected string (#1060 review). A
    // non-streaming turn emits a synthetic token carrying the full text.
    let streamed: String = events
        .iter()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            (v["type"].as_str() == Some("token"))
                .then(|| v["token"].as_str().unwrap_or("").to_string())
        })
        .collect();
    let found = events.iter().any(|line| {
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        if ev["type"].as_str() != Some("turn_end") {
            return false;
        }
        let content = ev["message"]["content"].as_str().unwrap_or("");
        let bounded_refs = ev
            .get("message")
            .and_then(|m| m.get("messageRefs"))
            .and_then(|r| r.as_array())
            .is_some_and(|a| {
                !a.is_empty() && a.iter().all(|r| r.as_str().is_some_and(|s| !s.is_empty()))
            });
        // Legacy: body still carried. #1060: body emptied, identified by refs,
        // AND the expected text was observed in the run's token stream.
        content == expected || (content.is_empty() && bounded_refs && streamed.contains(&expected))
    });
    assert!(
        found,
        "expected a turn_end event with content {expected:?} \
         (emptied-content path requires the text in the token stream: {streamed:?}) \
         in events:\n{events:#?}"
    );
}

/// Assert that at least one `turn_end` event carries a numeric `contextTokens`
/// field and a numeric `maxContextTokens` field. The TUI footer's context gauge
/// depends on both being present even for streaming OpenAI-compatible providers
/// (e.g. Fireworks) whose SSE stream does not carry per-turn `usage`.
#[then("the turn_end event should include numeric contextTokens and maxContextTokens")]
fn then_turn_end_includes_context_tokens(world: &mut QuectoWorld) {
    execute_uds(world);
    let events = &world.agent_events;
    let found = events.iter().any(|line| {
        if let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) {
            ev["type"].as_str() == Some("turn_end")
                && ev["message"]["contextTokens"].as_u64().is_some()
                && ev["message"]["maxContextTokens"].as_u64().is_some()
        } else {
            false
        }
    });
    assert!(
        found,
        "expected a turn_end event carrying numeric contextTokens and \
         maxContextTokens in events:\n{events:#?}"
    );
}

// ─── Multi-client UDS steps (#318) ────────────────────────────────────────────

/// Start the UDS agent in multi-client mode (real listener, not socket_override).
/// The agent binds a real socket and accepts multiple connections.
#[when("I start the multi-client UDS agent")]
fn when_start_multi_client_uds(world: &mut QuectoWorld) {
    world.mc_mode = true;
    world.no_session = true;
}

/// Register a client for multi-client mode.
#[when(expr = "client {int} connects")]
fn when_client_connects(world: &mut QuectoWorld, client_id: u32) {
    world.mc_connected_clients.push(client_id);
    world.mc_client_commands.entry(client_id).or_default();
    world.mc_client_events.entry(client_id).or_default();
}

/// Queue a prompt command on a specific client.
#[when(expr = "client {int} sends prompt {string}")]
fn when_client_sends_prompt(world: &mut QuectoWorld, client_id: u32, message: String) {
    let cmd = serde_json::json!({"type": "prompt", "message": message});
    world
        .mc_client_commands
        .entry(client_id)
        .or_default()
        .push(cmd.to_string());
}

/// Queue a prompt command with an id on a specific client.
#[when(expr = "client {int} sends prompt with id {string} and message {string}")]
fn when_client_sends_prompt_with_id(
    world: &mut QuectoWorld,
    client_id: u32,
    id: String,
    message: String,
) {
    let cmd = serde_json::json!({"type": "prompt", "id": id, "message": message});
    world
        .mc_client_commands
        .entry(client_id)
        .or_default()
        .push(cmd.to_string());
}

/// Mark a client as disconnected (will close its connection before others).
#[when(expr = "client {int} disconnects")]
fn when_client_disconnects(world: &mut QuectoWorld, client_id: u32) {
    world.mc_disconnected_clients.push(client_id);
}

/// Start the UDS agent in multi-client mode with --persist (#348).
#[when("I start the multi-client UDS agent with persist")]
fn when_start_multi_client_uds_persist(world: &mut QuectoWorld) {
    world.mc_mode = true;
    world.no_session = true;
    world._mc_persist = true;
}

/// Connect a new client after all previous clients have disconnected.
/// This exercises the persist path: the agent must still be alive.
#[when(expr = "a new client {int} connects after all clients disconnected")]
fn when_new_client_connects_after_disconnect(world: &mut QuectoWorld, client_id: u32) {
    // Mark as a "reconnect" client — execute_multi_client_uds will handle
    // connecting this client after the disconnected clients are dropped.
    world._mc_reconnect_clients.push(client_id);
    world.mc_client_commands.entry(client_id).or_default();
    world.mc_client_events.entry(client_id).or_default();
}

/// Close all multi-client connections and wait for the agent to exit.
#[when("I close all UDS clients")]
fn when_close_all_uds_clients(world: &mut QuectoWorld) {
    if world._mc_live_busy || world._mc_live_socket.is_some() {
        uds_bounded_events_steps::finalize_mc_live_pub(world);
        return;
    }
    execute_multi_client_uds(world);
}

/// Execute the multi-client UDS test scenario.
///
/// 1. Bind a real socket (like production)
/// 2. Spawn the UDS loop in a thread
/// 3. Connect N clients sequentially
/// 4. Send each client's queued commands
/// 5. Handle disconnections
/// 6. Close remaining clients
/// 7. Collect events per-client
pub(crate) fn execute_multi_client_uds(world: &mut QuectoWorld) {
    if world.mc_exit_code.is_some() {
        return;
    }

    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");

    if !base.join("config.json").exists() {
        world.agent_stderr = "config not found".to_string();
        world.mc_exit_code = Some(1);
        return;
    }

    let mut ctx = match build_uds_agent(world, &base) {
        Ok(c) => c,
        Err(e) => {
            world.agent_stderr = e;
            world.mc_exit_code = Some(1);
            return;
        }
    };
    ctx.persist = world._mc_persist;

    let socket_path = base.join("mc-test-agent.sock");
    let _ = std::fs::remove_file(&socket_path);

    let (handle, socket_path) = match mc_spawn_agent(
        ctx,
        &base,
        socket_path,
        world.system_prompt.clone().unwrap_or_default(),
    ) {
        Ok(pair) => pair,
        Err(msg) => {
            world.agent_stderr = msg;
            world.mc_exit_code = Some(1);
            return;
        }
    };

    let connected = world.mc_connected_clients.clone();
    let disconnected = world.mc_disconnected_clients.clone();
    let commands = world.mc_client_commands.clone();

    let reconnect_clients = world._mc_reconnect_clients.clone();
    let persist = world._mc_persist;

    mc_drive_clients(
        world,
        McClientActions {
            socket_path: &socket_path,
            connected: &connected,
            disconnected: &disconnected,
            commands: &commands,
            reconnect_clients: &reconnect_clients,
            persist,
        },
    );

    if persist {
        // In persist mode the agent won't exit on its own after all clients
        // disconnect.  Wait briefly, then treat exit code as 0 (success) and
        // let the detached thread be cleaned up when the test process exits.
        // We use a polling join with a short deadline.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            if handle.is_finished() {
                let exit = handle.join().unwrap_or(1);
                world.mc_exit_code = Some(exit);
                world.uds_exit_code = Some(exit);
                break;
            }
            if std::time::Instant::now() > deadline {
                // Agent is still alive — that's the expected persist behavior.
                world.mc_exit_code = Some(0);
                world.uds_exit_code = Some(0);
                // Remove socket so accept loop errors out and thread can exit.
                let _ = std::fs::remove_file(&socket_path);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    } else {
        let exit = handle.join().unwrap_or(1);
        world.mc_exit_code = Some(exit);
        world.uds_exit_code = Some(exit);
    }
}

/// Spawn the UDS agent loop in a background thread and wait for the socket.
/// `system_prompt` is the scenario's static base prompt (empty when unset) —
/// with cache-safe prompting (#1113) it is the ONLY system prompt the model
/// ever sees, byte-identical across every turn.
fn mc_spawn_agent(
    ctx: UdsAgentContext,
    base: &std::path::Path,
    socket_path: std::path::PathBuf,
    system_prompt: String,
) -> Result<(std::thread::JoinHandle<i32>, std::path::PathBuf), String> {
    let UdsAgentContext {
        agent,
        model,
        session_key,
        ephemeral,
        ext_registry,
        persist,
        workflow_state,
        workflow_config,
        broadcast_tx,
        mut provider_reload,
        provider_reload_inputs,
    } = ctx;
    let base_for_thread = base.to_path_buf();
    let sp = socket_path.clone();
    let handle = std::thread::spawn(move || {
        quecto::interface::cli::uds::run_uds_loop(UdsLoopArgs {
            agent,
            base_dir: &base_for_thread,
            session_key,
            model,
            ephemeral,
            system_prompt,
            socket_path: sp,
            socket_override: None,
            session_store_override: None,
            ext_registry: Some(ext_registry),
            persist,
            notification_rx: None,
            subagent_registry: None,
            workflow_state,
            workflow_config,
            broadcast_tx,
            provider_reload: Some(&mut provider_reload),
            provider_reload_inputs: Some(&provider_reload_inputs),
        })
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !socket_path.exists() {
        if std::time::Instant::now() > deadline {
            return Err("timeout waiting for socket".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    Ok((handle, socket_path))
}

/// Collected multi-client test parameters.
struct McClientActions<'a> {
    socket_path: &'a std::path::Path,
    connected: &'a [u32],
    disconnected: &'a [u32],
    commands: &'a HashMap<u32, Vec<String>>,
    /// Clients to connect after all `disconnected` clients are dropped (#348).
    reconnect_clients: &'a [u32],
    /// Whether the agent was started with --persist (#348).
    persist: bool,
}

/// Connect clients, send commands, handle disconnections, collect events.
fn mc_drive_clients(world: &mut QuectoWorld, actions: McClientActions<'_>) {
    use std::os::unix::net::UnixStream;

    let McClientActions {
        socket_path,
        connected,
        disconnected,
        commands,
        reconnect_clients,
        persist,
    } = actions;

    let mut streams: HashMap<u32, UnixStream> = HashMap::new();
    for &cid in connected {
        match UnixStream::connect(socket_path) {
            Ok(s) => {
                s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();
                s.set_nonblocking(false).ok();
                streams.insert(cid, s);
            }
            Err(e) => {
                world.agent_stderr = format!("client {cid} connect failed: {e}");
                world.mc_exit_code = Some(1);
                return;
            }
        }
    }

    mc_disconnect_early(&mut streams, disconnected);
    mc_send_commands(&mut streams, connected, disconnected, commands);
    if world.auto_mock_manual_llm && world._workflow_enabled {
        mc_collect_events_live(
            world,
            &mut streams,
            connected,
            disconnected,
            std::time::Duration::from_secs(30),
        );
        return;
    }
    // Reactive phase: if any client has auto-replies queued (e.g. for
    // execute_tool events that only exist after the LLM is consulted),
    // read-and-react on its stream before the final collection step.
    if !world.mc_auto_replies.is_empty() {
        mc_reactive_auto_replies(world, &mut streams, connected, disconnected);
    }
    let settle_secs = if world.auto_mock_manual_llm && world._workflow_enabled {
        20
    } else {
        2
    };
    std::thread::sleep(std::time::Duration::from_secs(settle_secs));
    mc_collect_events(world, &mut streams, connected, disconnected);

    // Persist mode: reconnect clients after all initial clients have disconnected (#348).
    if persist && !reconnect_clients.is_empty() {
        // Give the agent a moment to process the disconnections.
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Connect reconnect clients.
        for &cid in reconnect_clients {
            match UnixStream::connect(socket_path) {
                Ok(s) => {
                    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .ok();
                    s.set_nonblocking(false).ok();
                    streams.insert(cid, s);
                }
                Err(e) => {
                    world.agent_stderr = format!("reconnect client {cid} connect failed: {e}");
                    world.mc_exit_code = Some(1);
                    return;
                }
            }
        }

        // Send commands for reconnect clients.
        mc_send_commands(&mut streams, reconnect_clients, &[], commands);
        std::thread::sleep(std::time::Duration::from_secs(2));
        // Collect events and close reconnect clients.
        mc_collect_events(world, &mut streams, reconnect_clients, &[]);
    }
}

fn mc_collect_events_live(
    world: &mut QuectoWorld,
    streams: &mut HashMap<u32, std::os::unix::net::UnixStream>,
    connected: &[u32],
    disconnected: &[u32],
    timeout: std::time::Duration,
) {
    use std::io::BufRead;

    let deadline = std::time::Instant::now() + timeout;
    for &cid in connected {
        if disconnected.contains(&cid) {
            continue;
        }
        let Some(stream) = streams.remove(&cid) else {
            continue;
        };
        let Ok(reader_stream) = stream.try_clone() else {
            continue;
        };
        let expected_agent_end = world
            .mc_client_commands
            .get(&cid)
            .map(|commands| {
                commands
                    .iter()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .filter(|command| {
                        matches!(command["type"].as_str(), Some("prompt" | "follow_up"))
                    })
                    .count()
            })
            .unwrap_or(0);
        let expected_responses = world
            .mc_client_commands
            .get(&cid)
            .map(|commands| {
                commands
                    .iter()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .filter(|command| {
                        !matches!(command["type"].as_str(), Some("prompt" | "follow_up"))
                    })
                    .count()
            })
            .unwrap_or(0);
        let mut agent_end_count = 0;
        let mut response_count = 0;
        reader_stream
            .set_read_timeout(Some(std::time::Duration::from_millis(250)))
            .ok();
        let mut reader = std::io::BufReader::new(reader_stream);
        let events = world.mc_client_events.entry(cid).or_default();
        loop {
            if std::time::Instant::now() > deadline {
                break;
            }
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let line = line.trim_end().to_string();
                    if !line.is_empty() {
                        let is_agent_end = line.contains(r#""type":"agent_end""#);
                        let is_response = serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .is_some_and(|event| {
                                event["type"].as_str() == Some("response")
                                    && event["command"].as_str() != Some("prompt")
                            });
                        events.push(line);
                        if is_agent_end {
                            agent_end_count += 1;
                        }
                        if is_response {
                            response_count += 1;
                        }
                        if (expected_agent_end == 0 || agent_end_count >= expected_agent_end)
                            && response_count >= expected_responses
                        {
                            break;
                        }
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
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

/// Disconnect clients marked for early disconnection.
fn mc_disconnect_early(
    streams: &mut HashMap<u32, std::os::unix::net::UnixStream>,
    disconnected: &[u32],
) {
    for &cid in disconnected {
        if let Some(stream) = streams.remove(&cid) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            drop(stream);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

/// Send queued commands to connected clients.
fn mc_send_commands(
    streams: &mut HashMap<u32, std::os::unix::net::UnixStream>,
    connected: &[u32],
    disconnected: &[u32],
    commands: &HashMap<u32, Vec<String>>,
) {
    use std::io::Write;
    for &cid in connected {
        if disconnected.contains(&cid) {
            continue;
        }
        if let (Some(cmds), Some(stream)) = (commands.get(&cid), streams.get_mut(&cid)) {
            for cmd in cmds {
                let _ = stream.write_all(format!("{cmd}\n").as_bytes());
            }
            let _ = stream.flush();
        }
    }
}

/// Read events from connected clients and close their connections.
fn mc_collect_events(
    world: &mut QuectoWorld,
    streams: &mut HashMap<u32, std::os::unix::net::UnixStream>,
    connected: &[u32],
    disconnected: &[u32],
) {
    use std::io::{BufRead, BufReader};
    for &cid in connected {
        if disconnected.contains(&cid) {
            continue;
        }
        if let Some(stream) = streams.remove(&cid) {
            let _ = stream.shutdown(std::net::Shutdown::Write);
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .ok();
            let events: Vec<String> = BufReader::new(&stream)
                .lines()
                .take_while(|l| l.is_ok())
                .filter_map(|l| l.ok())
                .filter(|l| !l.is_empty())
                .collect();
            // Append — the reactive phase may already have populated
            // some events for this client; preserve them.
            world
                .mc_client_events
                .entry(cid)
                .or_default()
                .extend(events);
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Reactive auto-replies: watch each client's stream for `execute_tool`
/// events and send `tool_result` back using the captured `toolCallId`.
/// Lines read here are saved into `world.mc_client_events` so the final
/// collection step doesn't lose them.
///
/// Deadline is conservative — the LLM roundtrip is mocked and fast, but
/// the agent's internal event pump introduces a small amount of latency.
fn mc_reactive_auto_replies(
    world: &mut QuectoWorld,
    streams: &mut HashMap<u32, std::os::unix::net::UnixStream>,
    connected: &[u32],
    disconnected: &[u32],
) {
    use std::io::{BufRead, BufReader, Write};

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);

    // Snapshot pending replies per client (Vec<(tool_name, content)>).
    let mut pending: HashMap<u32, Vec<(String, String)>> = world.mc_auto_replies.clone();

    for &cid in connected {
        if disconnected.contains(&cid) {
            continue;
        }
        let Some(stream) = streams.get_mut(&cid) else {
            continue;
        };
        let Some(replies) = pending.get_mut(&cid) else {
            continue;
        };
        if replies.is_empty() {
            continue;
        }

        // Short per-read timeout so the loop can check its deadline.
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(250)))
            .ok();

        let stream_for_read = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut reader = BufReader::new(stream_for_read);

        while !replies.is_empty() && std::time::Instant::now() < deadline {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break, // stream closed
                Ok(_) => {}
                Err(_) => {
                    // timed out on this read; loop again
                    continue;
                }
            }
            let trimmed = line.trim_end_matches('\n').to_string();
            if trimmed.is_empty() {
                continue;
            }
            // Save every line so mc_collect_events doesn't lose it.
            world
                .mc_client_events
                .entry(cid)
                .or_default()
                .push(trimmed.clone());

            // Is this an execute_tool event we should reply to?
            let Ok(ev) = serde_json::from_str::<serde_json::Value>(&trimmed) else {
                continue;
            };
            if ev["type"].as_str() != Some("execute_tool") {
                continue;
            }
            let Some(tool_name) = ev["toolName"].as_str() else {
                continue;
            };
            let Some(idx) = replies.iter().position(|(t, _)| t == tool_name) else {
                continue;
            };
            let (_, content) = replies.remove(idx);
            let tool_call_id = ev["toolCallId"].as_str().unwrap_or("").to_string();

            let reply = serde_json::json!({
                "type": "tool_result",
                "toolCallId": tool_call_id,
                "content": content,
                "isError": false,
            });
            let _ = writeln!(stream, "{reply}");
            let _ = stream.flush();
        }
    }
}

/// Helper: get event types for a specific client.
fn mc_client_event_types(world: &QuectoWorld, client_id: u32) -> Vec<String> {
    world
        .mc_client_events
        .get(&client_id)
        .map(|events| {
            events
                .iter()
                .filter_map(|l| {
                    serde_json::from_str::<serde_json::Value>(l)
                        .ok()
                        .and_then(|v| v["type"].as_str().map(str::to_owned))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[then(expr = "client {int} should have received an event of type {string}")]
fn then_client_received_event_type(world: &mut QuectoWorld, client_id: u32, event_type: String) {
    execute_multi_client_uds(world);
    let types = mc_client_event_types(world, client_id);
    assert!(
        types.contains(&event_type),
        "expected client {client_id} to have received event {event_type:?}\ngot: {types:?}\nevents: {:#?}",
        world.mc_client_events.get(&client_id),
    );
}

#[then(expr = "client {int} should have received a response with id {string}")]
fn then_client_received_response_with_id(
    world: &mut QuectoWorld,
    client_id: u32,
    expected_id: String,
) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let found = events.iter().any(|l| {
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
        "expected client {client_id} to have received response with id {expected_id:?}\nevents: {events:#?}",
    );
}

// ─── tool_call_id assertion steps (#318) ──────────────────────────────────────

/// Helper: find a specific event type's JSON in a list of event lines.
fn find_event_json(events: &[String], event_type: &str) -> Option<serde_json::Value> {
    events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["type"].as_str() == Some(event_type) {
            Some(v)
        } else {
            None
        }
    })
}

#[then(
    expr = "client {int} should have received a tool_execution_start with a non-empty tool_call_id"
)]
fn then_client_received_tool_start_with_id(world: &mut QuectoWorld, client_id: u32) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let ev = find_event_json(&events, "tool_execution_start");
    assert!(
        ev.is_some(),
        "expected client {client_id} to have received tool_execution_start\nevents: {events:#?}"
    );
    let ev_val = ev.unwrap();
    let tool_call_id = ev_val["toolCallId"].as_str().unwrap_or("");
    assert!(
        !tool_call_id.is_empty(),
        "expected non-empty toolCallId in tool_execution_start\nevents: {events:#?}"
    );
}

#[then(
    expr = "client {int} should have received a tool_execution_end with a non-empty tool_call_id"
)]
fn then_client_received_tool_end_with_id(world: &mut QuectoWorld, client_id: u32) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let ev = find_event_json(&events, "tool_execution_end");
    assert!(
        ev.is_some(),
        "expected client {client_id} to have received tool_execution_end\nevents: {events:#?}"
    );
    let ev_val = ev.unwrap();
    let tool_call_id = ev_val["toolCallId"].as_str().unwrap_or("");
    assert!(
        !tool_call_id.is_empty(),
        "expected non-empty toolCallId in tool_execution_end\nevents: {events:#?}"
    );
}

#[then("the agent output should contain a tool_execution_start with a non-empty tool_call_id")]
fn then_agent_output_tool_start_with_id(world: &mut QuectoWorld) {
    execute_uds(world);
    let ev = find_event_json(&world.agent_events, "tool_execution_start");
    assert!(
        ev.is_some(),
        "expected tool_execution_start event\nevents: {:#?}",
        world.agent_events
    );
    let ev_val = ev.unwrap();
    let tool_call_id = ev_val["toolCallId"].as_str().unwrap_or("");
    assert!(
        !tool_call_id.is_empty(),
        "expected non-empty toolCallId in tool_execution_start\nevents: {:#?}",
        world.agent_events
    );
}

/// Queue a command on a specific multi-client connection.
#[when(expr = "client {int} sends command {string} with id {string}")]
fn when_client_sends_command_with_id(
    world: &mut QuectoWorld,
    client_id: u32,
    command: String,
    id: String,
) {
    let cmd = serde_json::json!({"type": command, "id": id});
    world
        .mc_client_commands
        .entry(client_id)
        .or_default()
        .push(cmd.to_string());
}

/// Mock LLM returns a tool call to a named extension tool, then text.
#[given(expr = "the mock LLM returns a tool call to {string} then a text response {string}")]
fn given_mock_llm_extension_tool_call_then_text(
    world: &mut QuectoWorld,
    tool_name: String,
    text: String,
) {
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
                        "id": format!("call_{tool_name}"),
                        "type": "function",
                        "function": {
                            "name": tool_name,
                            "arguments": "{\"input\":\"test\"}"
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

// ─── get_tool_catalogue assertion steps ───────────────────────────────────────────

/// Find the get_tool_catalogue response (first or post-reload depending on context).
fn find_get_tool_catalogue_response(
    events: &[String],
    id_prefix: Option<&str>,
) -> Option<serde_json::Value> {
    events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["type"] == "response" && v["command"] == "get_tool_catalogue" {
            if let Some(prefix) = id_prefix {
                if v["id"].as_str()?.starts_with(prefix) {
                    return Some(v);
                }
                return None;
            }
            Some(v)
        } else {
            None
        }
    })
}

#[then(expr = "the get_tool_catalogue response should list tool {string}")]
fn then_get_tool_catalogue_lists(world: &mut QuectoWorld, name: String) {
    execute_uds(world);
    let resp = find_get_tool_catalogue_response(&world.agent_events, None)
        .expect("no get_tool_catalogue response");
    let tools = resp["data"]["tools"]
        .as_array()
        .expect("tools not an array");
    let found = tools.iter().any(|e| e["name"].as_str() == Some(&name));
    assert!(
        found,
        "expected get_tool_catalogue to list tool {name:?}\ntools: {tools:?}"
    );
}

#[then(expr = "the get_tool_catalogue response should have {int} tools")]
fn then_get_tool_catalogue_count(world: &mut QuectoWorld, count: usize) {
    execute_uds(world);
    let resp = find_get_tool_catalogue_response(&world.agent_events, None)
        .expect("no get_tool_catalogue response");
    let tools = resp["data"]["tools"]
        .as_array()
        .expect("tools not an array");
    assert_eq!(
        tools.len(),
        count,
        "expected {count} tools, got {}\ntools: {tools:?}",
        tools.len()
    );
}

#[then(expr = "the get_tool_catalogue response should not list tool {string}")]
fn then_get_tool_catalogue_not_lists(world: &mut QuectoWorld, name: String) {
    execute_uds(world);
    let resp = find_get_tool_catalogue_response(&world.agent_events, None)
        .expect("no get_tool_catalogue response");
    let tools = resp["data"]["tools"]
        .as_array()
        .expect("tools not an array");
    let found = tools.iter().any(|e| e["name"].as_str() == Some(&name));
    assert!(
        !found,
        "expected get_tool_catalogue NOT to list tool {name:?}\ntools: {tools:?}"
    );
}

#[then(expr = "the get_tool_catalogue response for {string} should include rich catalogue state")]
fn then_get_tool_catalogue_entry_has_rich_state(world: &mut QuectoWorld, name: String) {
    execute_uds(world);
    let resp = find_get_tool_catalogue_response(&world.agent_events, None)
        .expect("no get_tool_catalogue response");
    let tools = resp["data"]["tools"]
        .as_array()
        .expect("tools not an array");
    let entry = tools
        .iter()
        .find(|e| e["name"].as_str() == Some(&name))
        .unwrap_or_else(|| panic!("'{name}' not in {tools:?}"));
    for field in [
        "stableId",
        "label",
        "description",
        "inputSchema",
        "source",
        "owner",
        "providerId",
        "lifecycle",
        "runtimeAvailability",
        "effectiveEnabled",
        "health",
    ] {
        assert!(
            entry.get(field).is_some(),
            "expected rich catalogue field {field:?} in {entry:?}"
        );
    }
}

// ─── tool_catalogue_changed event assertions ──────────────────────────────────────

#[then(expr = "client {int} should have received a tool catalogue update listing tool {string}")]
fn then_client_received_tool_catalogue_changed_listing(
    world: &mut QuectoWorld,
    client_id: u32,
    name: String,
) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let found = events.iter().any(|l| {
        let v: serde_json::Value = match serde_json::from_str(l) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if v["type"].as_str() != Some("tool_catalogue_changed") {
            return false;
        }
        v["changedTools"]
            .as_array()
            .map(|tools| tools.iter().any(|tool| tool.as_str() == Some(&name)))
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected client {client_id} to receive tool_catalogue_changed listing {name:?}\nevents: {events:#?}"
    );
}

// ─── Extension tool execution assertions ──────────────────────────────────────

#[then(expr = "the agent output should contain a tool_execution_start with tool name {string}")]
fn then_agent_output_tool_start_with_name(world: &mut QuectoWorld, name: String) {
    execute_uds(world);
    let found = world.agent_events.iter().any(|l| {
        let v: serde_json::Value = match serde_json::from_str(l) {
            Ok(v) => v,
            Err(_) => return false,
        };
        v["type"].as_str() == Some("tool_execution_start") && v["toolName"].as_str() == Some(&name)
    });
    assert!(
        found,
        "expected tool_execution_start with toolName={name:?}\nevents: {:#?}",
        world.agent_events
    );
}

#[then(expr = "the agent output should contain a tool_execution_end with tool name {string}")]
fn then_agent_output_tool_end_with_name(world: &mut QuectoWorld, name: String) {
    execute_uds(world);
    let found = world.agent_events.iter().any(|l| {
        let v: serde_json::Value = match serde_json::from_str(l) {
            Ok(v) => v,
            Err(_) => return false,
        };
        v["type"].as_str() == Some("tool_execution_end") && v["toolName"].as_str() == Some(&name)
    });
    assert!(
        found,
        "expected tool_execution_end with toolName={name:?}\nevents: {:#?}",
        world.agent_events
    );
}

// ─── Multi-client command response assertions ─────────────────────────────────

#[then(expr = "client {int} should have received a response command {string} with success true")]
fn then_client_received_response_command_success(
    world: &mut QuectoWorld,
    client_id: u32,
    command: String,
) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let found = events.iter().any(|l| {
        let v: serde_json::Value = match serde_json::from_str(l) {
            Ok(v) => v,
            Err(_) => return false,
        };
        v["type"] == "response" && v["command"] == command && v["success"] == true
    });
    assert!(
        found,
        "expected client {client_id} to receive response command {command:?} with success=true\nevents: {events:#?}"
    );
}

#[then(expr = "client {int} should have received a response command {string} with success false")]
fn then_client_received_response_command_failure(
    world: &mut QuectoWorld,
    client_id: u32,
    command: String,
) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let found = events.iter().any(|l| {
        let v: serde_json::Value = match serde_json::from_str(l) {
            Ok(v) => v,
            Err(_) => return false,
        };
        v["type"] == "response" && v["command"] == command && v["success"] == false
    });
    assert!(
        found,
        "expected client {client_id} to receive response command {command:?} with success=false\nevents: {events:#?}"
    );
}

// ─── Real-LLM UDS executor ───────────────────────────────────────────────────
//
// Uses real OAuth credentials and a real socket bind.  Sends commands
// sequentially, waiting for each prompt to complete before sending the next.

/// Execute the real-LLM UDS test: spawn agent with real socket, send commands
/// one at a time, wait for prompt completions, collect all events.
/// Shared state for the real-LLM UDS reader thread.
struct RealLlmReaderState {
    events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    prompt_completions: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Timestamp of the last event received — used to detect quiet periods
    /// when waiting for asynchronous follow-up processing.
    last_event_time: std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
}

/// Spawn a reader thread that collects events and tracks prompt completions.
fn spawn_real_llm_reader(
    mut reader: std::io::BufReader<std::os::unix::net::UnixStream>,
) -> (std::thread::JoinHandle<()>, RealLlmReaderState) {
    use std::io::BufRead;
    let state = RealLlmReaderState {
        events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        prompt_completions: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
        last_event_time: std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
    };
    let events = state.events.clone();
    let completions = state.prompt_completions.clone();
    let last_event = state.last_event_time.clone();

    let handle = std::thread::spawn(move || {
        let mut buf = String::new();
        loop {
            buf.clear();
            match reader.read_line(&mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    let l = buf.trim().to_string();
                    if l.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&l) {
                        let t = v["type"].as_str().unwrap_or("");
                        let cmd = v["command"].as_str().unwrap_or("");
                        if t == "response" && (cmd == "prompt" || cmd == "agent_error") {
                            completions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    *last_event.lock().unwrap() = std::time::Instant::now();
                    events.lock().unwrap().push(l);
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    break;
                }
                Err(_) => break,
            }
        }
    });
    (handle, state)
}

/// Arguments for [`send_real_llm_commands`].
struct SendCommandsArgs<'a> {
    commands: &'a [String],
    writer: &'a mut std::os::unix::net::UnixStream,
    state: &'a RealLlmReaderState,
    stderr: &'a mut String,
    has_follow_ups: bool,
}

/// Send commands to a real-LLM UDS agent, waiting for prompt completions.
///
/// When `has_follow_ups` is true, waits for a quiet period (no new events for
/// 3s) after all prompts complete.  This handles follow_up scenarios where the
/// server processes pending messages asynchronously after the prompt response
/// — those pending runs produce their own agent_end events, and we must not
/// shut down the write side until they finish (because the server aborts the
/// client writer task on read-EOF, which would prevent us from receiving the
/// follow-up events).
fn send_real_llm_commands(args: SendCommandsArgs<'_>) {
    let SendCommandsArgs {
        commands,
        writer,
        state,
        stderr,
        has_follow_ups,
    } = args;
    use std::io::Write;
    let mut expected_completions: u32 = 0;
    for cmd_str in commands {
        let _ = writer.write_all(format!("{cmd_str}\n").as_bytes());
        let _ = writer.flush();

        let is_prompt = serde_json::from_str::<serde_json::Value>(cmd_str)
            .map(|v| v["type"].as_str() == Some("prompt"))
            .unwrap_or(false);

        if is_prompt {
            expected_completions += 1;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                if std::time::Instant::now() > deadline {
                    stderr.push_str("timeout waiting for prompt completion\n");
                    break;
                }
                let current = state
                    .prompt_completions
                    .load(std::sync::atomic::Ordering::SeqCst);
                if current >= expected_completions {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    // When follow_ups are queued, wait for a quiet period so the server has
    // time to process pending messages (which run asynchronously after the
    // prompt response is emitted).
    if has_follow_ups {
        let quiet_duration = std::time::Duration::from_secs(3);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if std::time::Instant::now() > deadline {
                stderr.push_str("timeout waiting for follow-up processing\n");
                break;
            }
            let elapsed = state.last_event_time.lock().unwrap().elapsed();
            if elapsed >= quiet_duration {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
}

fn execute_real_llm_uds(world: &mut QuectoWorld) {
    use std::io::BufReader;
    use std::os::unix::net::UnixStream;

    if world.uds_exit_code.is_some() {
        return;
    }

    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a real LLM UDS workspace is configured'");

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

    let socket_path = base.join("real-llm-uds.sock");
    let _ = std::fs::remove_file(&socket_path);

    let (agent_handle, socket_path) = match mc_spawn_agent(
        ctx,
        &base,
        socket_path,
        world.system_prompt.clone().unwrap_or_default(),
    ) {
        Ok(pair) => pair,
        Err(msg) => {
            world.agent_stderr = msg;
            world.uds_exit_code = Some(1);
            return;
        }
    };

    // Connect a single client
    let stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            world.agent_stderr = format!("connect failed: {e}");
            world.uds_exit_code = Some(1);
            return;
        }
    };
    stream.set_nonblocking(false).ok();
    let reader_stream = stream.try_clone().expect("clone stream for reader");
    // SO_RCVTIMEO must be set on the clone used for reading — try_clone
    // does not inherit socket options from the original FD.
    reader_stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut writer = stream;

    let (reader_handle, state) = spawn_real_llm_reader(BufReader::new(reader_stream));

    let commands = world.uds_commands.clone();

    // Count follow_up commands — they produce extra agent_end events
    // after the explicit prompt completes, so we need to wait for them.
    let has_follow_ups = commands.iter().any(|c| {
        serde_json::from_str::<serde_json::Value>(c)
            .map(|v| v["type"].as_str() == Some("follow_up"))
            .unwrap_or(false)
    });

    send_real_llm_commands(SendCommandsArgs {
        commands: &commands,
        writer: &mut writer,
        state: &state,
        stderr: &mut world.agent_stderr,
        has_follow_ups,
    });

    // Give the server time to flush responses for non-prompt commands,
    // then shut down the write half to signal EOF.
    std::thread::sleep(std::time::Duration::from_secs(1));
    let _ = writer.shutdown(std::net::Shutdown::Write);

    let _ = reader_handle.join();
    let exit = agent_handle.join().unwrap_or(1);

    world.agent_events = state.events.lock().unwrap().clone();
    world.uds_exit_code = Some(exit);
    world.mc_exit_code = Some(exit);
    world
        .agent_stderr
        .push_str(&format!("quecto-agent-socket: {}\n", socket_path.display()));
    world._uds_socket_path = Some(socket_path);
}

// ─── UDS extension protocol steps (#352) ──────────────────────────────────────

#[when(expr = "client {int} sends register_tools with tool {string} described as {string}")]
fn when_client_sends_register_tools(
    world: &mut QuectoWorld,
    client_id: u32,
    tool_name: String,
    description: String,
) {
    let cmd = serde_json::json!({
        "type": "register_tools",
        "id": format!("rt-{client_id}"),
        "tools": [{"name": tool_name, "description": description}]
    });
    world
        .mc_client_commands
        .entry(client_id)
        .or_default()
        .push(cmd.to_string());
}

#[when(expr = "client {int} sends unregister_tools with tool {string}")]
fn when_client_sends_unregister_tools(world: &mut QuectoWorld, client_id: u32, tool_name: String) {
    let cmd = serde_json::json!({
        "type": "unregister_tools",
        "id": format!("ut-{client_id}"),
        "tools": [tool_name]
    });
    world
        .mc_client_commands
        .entry(client_id)
        .or_default()
        .push(cmd.to_string());
}

fn find_ge_response(events: &[String], id_prefix: &str) -> Option<serde_json::Value> {
    events.iter().find_map(|line| {
        let ev: serde_json::Value = serde_json::from_str(line).ok()?;
        if ev["type"].as_str() == Some("response")
            && ev["command"].as_str() == Some("get_tool_catalogue")
            && ev["success"].as_bool() == Some(true)
            && ev["id"]
                .as_str()
                .is_some_and(|id| id.starts_with(id_prefix))
        {
            Some(ev)
        } else {
            None
        }
    })
}

#[then(expr = "the tool catalogue response {string} should list tool {string}")]
fn then_catalogue_response_lists_tool(world: &mut QuectoWorld, response_id: String, name: String) {
    execute_multi_client_uds(world);
    let client_id = if response_id == "ge-multi" { 2 } else { 1 };
    let events = world
        .mc_client_events
        .get(&client_id)
        .unwrap_or_else(|| panic!("no client {client_id} events"));
    let resp = find_ge_response(events, &response_id)
        .unwrap_or_else(|| panic!("no {response_id} response"));
    let tools = resp["data"]["tools"].as_array().expect("no tools");
    assert!(
        tools.iter().any(|e| e["name"].as_str() == Some(&name)),
        "'{name}' not in {tools:?}"
    );
}

#[then(expr = "the tool catalogue response {string} should contain {int} tools")]
fn then_catalogue_response_count(world: &mut QuectoWorld, response_id: String, count: u32) {
    execute_multi_client_uds(world);
    let client_id = if response_id == "ge-disc" { 3 } else { 1 };
    let events = world
        .mc_client_events
        .get(&client_id)
        .unwrap_or_else(|| panic!("no client {client_id} events"));
    let resp = find_ge_response(events, &response_id)
        .unwrap_or_else(|| panic!("no {response_id} response"));
    let tools = resp["data"]["tools"].as_array().expect("no tools");
    assert_eq!(tools.len(), count as usize);
}

#[then(expr = "the tool catalogue response {string} should not list tool {string}")]
fn then_catalogue_response_not_lists_tool(
    world: &mut QuectoWorld,
    response_id: String,
    name: String,
) {
    execute_multi_client_uds(world);
    let client_id = if response_id == "ge-disc" { 3 } else { 1 };
    let events = world
        .mc_client_events
        .get(&client_id)
        .unwrap_or_else(|| panic!("no client {client_id} events"));
    let resp = find_ge_response(events, &response_id)
        .unwrap_or_else(|| panic!("no {response_id} response"));
    let tools = resp["data"]["tools"].as_array().expect("no tools");
    assert!(
        !tools.iter().any(|e| e["name"].as_str() == Some(&name)),
        "'{name}' unexpectedly present in {tools:?}"
    );
}

#[then(
    expr = "the tool catalogue response {string} should list tool {string} with description {string}"
)]
fn then_catalogue_response_desc(
    world: &mut QuectoWorld,
    response_id: String,
    name: String,
    desc: String,
) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&1).expect("no client 1 events");
    let resp = find_ge_response(events, &response_id)
        .unwrap_or_else(|| panic!("no {response_id} response"));
    let tools = resp["data"]["tools"].as_array().expect("no tools");
    let ext = tools
        .iter()
        .find(|e| e["name"].as_str() == Some(&name))
        .unwrap_or_else(|| panic!("'{name}' not in {tools:?}"));
    assert_eq!(ext["description"].as_str(), Some(desc.as_str()));
}

// ─── Workflow broadcast event steps (#598) ────────────────────────────────────

/// Start the multi-client UDS agent with workflow enabled.
#[when("I start the multi-client UDS agent with workflow enabled")]
fn when_start_mc_uds_with_workflow(world: &mut QuectoWorld) {
    world.mc_mode = true;
    world.no_session = true;
    world._workflow_enabled = true;
}

/// Same, with an explicit static system prompt (#1113): the cache-safe
/// scenarios assert on the system message of every LLM request, so the
/// session must carry a real (non-empty) system prompt.
#[when(
    expr = "I start the multi-client UDS agent with workflow enabled and system prompt {string}"
)]
fn when_start_mc_uds_with_workflow_and_system(world: &mut QuectoWorld, system: String) {
    world.mc_mode = true;
    world.no_session = true;
    world._workflow_enabled = true;
    world.system_prompt = Some(system);
}

/// Mock: LLM returns a tool call for `workflow select_template feature`, then a text reply.
#[given(expr = "the mock LLM returns a tool call for workflow select_template then text {string}")]
fn given_mock_llm_workflow_select(world: &mut QuectoWorld, text: String) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        let tool_call_body = serde_json::json!({
            "id": "chatcmpl-wf-select",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_wf_select",
                        "type": "function",
                        "function": {
                            "name": "workflow",
                            "arguments": "{\"action\":\"select_template\",\"template\":\"feature\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let text_body = serde_json::json!({
            "id": "chatcmpl-wf-text",
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

/// Mock: LLM returns select_template, then check step 1, then text reply.
#[given(expr = "the mock LLM returns tool calls for workflow select then check then text {string}")]
fn given_mock_llm_workflow_select_check(world: &mut QuectoWorld, text: String) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        let select_body = serde_json::json!({
            "id": "chatcmpl-wf-s",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_wf_sel",
                        "type": "function",
                        "function": {
                            "name": "workflow",
                            "arguments": "{\"action\":\"select_template\",\"template\":\"feature\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let check_body = serde_json::json!({
            "id": "chatcmpl-wf-c",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_wf_chk",
                        "type": "function",
                        "function": {
                            "name": "workflow",
                            "arguments": "{\"action\":\"check\",\"step\":1}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}
        });
        let text_body = serde_json::json!({
            "id": "chatcmpl-wf-t",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": text },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 25, "completion_tokens": 5, "total_tokens": 30}
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(select_body))
            .up_to_n_times(1)
            .with_priority(2)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(check_body))
            .up_to_n_times(1)
            .with_priority(3)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .mount(&server)
            .await;

        e2e_steps::rewrite_config_to_uri(world, &new_uri);
        // Keep the server reachable so #1113 steps can inspect the system
        // prompts the agent actually sent (leaked: BDD process is short-lived).
        world.wiremock_server_ref = Some(Box::leak(Box::new(server)));
    });
    std::mem::forget(rt);
}

/// Mock (#1113): LLM selects an inline template, checks steps 1–3 in order,
/// then replies with a final text. Also used to prove the system prompt stays
/// byte-identical across the whole run, so the server ref is stored.
#[given(
    expr = "the mock LLM selects template {string}, checks all three steps, then replies {string}"
)]
fn given_mock_llm_workflow_full_completion(
    world: &mut QuectoWorld,
    template: String,
    text: String,
) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        fn tool_call_body(id: &str, call_id: &str, arguments: &str) -> serde_json::Value {
            serde_json::json!({
                "id": id,
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": call_id,
                            "type": "function",
                            "function": { "name": "workflow", "arguments": arguments }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            })
        }

        let select_args = format!("{{\"action\":\"select_template\",\"template\":\"{template}\"}}");
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(tool_call_body(
                    "chatcmpl-wf-sel",
                    "call_wf_sel",
                    &select_args,
                )),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        for step in 1..=3u8 {
            let check_args = format!("{{\"action\":\"check\",\"step\":{step}}}");
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(
                    wiremock::ResponseTemplate::new(200).set_body_json(tool_call_body(
                        &format!("chatcmpl-wf-c{step}"),
                        &format!("call_wf_c{step}"),
                        &check_args,
                    )),
                )
                .up_to_n_times(1)
                .with_priority(1 + step)
                .mount(&server)
                .await;
        }
        let text_body = serde_json::json!({
            "id": "chatcmpl-wf-done",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": text },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 25, "completion_tokens": 5, "total_tokens": 30}
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .mount(&server)
            .await;

        e2e_steps::rewrite_config_to_uri(world, &new_uri);
        world.wiremock_server_ref = Some(Box::leak(Box::new(server)));
    });
    std::mem::forget(rt);
}

/// #1113 (AC6): rewrite the scenario config so it defines an inline
/// three-step `workflow.templates` entry — inline configs must keep loading
/// and behaving identically with cache-safe prompting.
#[given(expr = "the config file defines an inline three-step workflow template {string}")]
fn given_config_defines_inline_three_step_template(world: &mut QuectoWorld, id: String) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("no base dir — add 'Given a temp base directory'");
    let path = base.join("config.json");
    let raw = std::fs::read_to_string(&path).expect("config.json must exist before this step");
    let mut config: serde_json::Value = serde_json::from_str(&raw).expect("config.json is JSON");
    config["workflow"] = serde_json::json!({
        "templates": [{
            "id": id,
            "label": "Inline Three Step",
            "description": "Inline template exercising cache-safe prompting (#1113)",
            "steps": [
                {"key": "one", "label": "Inline step one", "phase": "red",
                 "guidance": "inline guidance one"},
                {"key": "two", "label": "Inline step two", "phase": "green",
                 "guidance": "inline guidance two"},
                {"key": "three", "label": "Inline step three", "phase": "review",
                 "guidance": "inline guidance three"}
            ]
        }]
    });
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
        .expect("rewrite config.json with inline workflow templates");
}

/// #1113 (AC1 / PRD G1): every chat request the agent sent to the LLM must
/// carry byte-identical, non-empty system-message content — the workflow
/// engine must never mutate the rendered system prompt between turns. Each
/// request must actually carry a system message with plain string content:
/// an implementation that drops (or restructures) the system message must
/// fail here rather than pass vacuously on equal empty strings.
#[then("every LLM request of the session should carry a byte-identical system prompt")]
fn then_every_llm_request_has_identical_system_prompt(world: &mut QuectoWorld) {
    let server = world
        .wiremock_server_ref
        .expect("mock server ref not stored — use a #1113-aware workflow mock step");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let requests = rt
        .block_on(server.received_requests())
        .expect("received requests should be available");
    std::mem::forget(rt);
    let system_prompts: Vec<String> = requests
        .iter()
        .filter(|request: &&Request| {
            request.method.as_str() == "POST" && request.url.path() == "/chat/completions"
        })
        .map(|request| {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("chat request body is JSON");
            let systems: Vec<String> = body["messages"]
                .as_array()
                .expect("chat request has a messages array")
                .iter()
                .filter(|m| m["role"] == "system")
                .map(|m| {
                    let content = m["content"].as_str().unwrap_or_else(|| {
                        panic!("system message content must be a plain string: {m}")
                    });
                    assert!(
                        !content.is_empty(),
                        "system message content must be non-empty"
                    );
                    content.to_string()
                })
                .collect();
            assert!(
                !systems.is_empty(),
                "every LLM request must carry a system message; request body: {body}"
            );
            systems.join("\n---\n")
        })
        .collect();
    assert!(
        system_prompts.len() >= 2,
        "expected at least two LLM calls to compare system prompts across; got {}\nevents: {:#?}\nstderr: {}",
        system_prompts.len(),
        world.agent_events,
        world.agent_stderr
    );
    let first = &system_prompts[0];
    for (i, prompt) in system_prompts.iter().enumerate() {
        assert_eq!(
            prompt,
            first,
            "system prompt mutated between LLM call 1 and call {} — workflow state must not be injected into the system prompt (#1113)",
            i + 1
        );
    }
}

/// Mock (#1113 AC3): the model's FIRST reply is plain text with no
/// `select_template` call — the session reaches its first idle boundary
/// unselected, so the selector must arrive via the idle-boundary nudge. The
/// second reply (the nudged turn) selects a template; a text catch-all ends
/// the run. The server ref is stored so the selector-delivery assertion can
/// inspect the requests the agent actually sent.
#[given(
    expr = "the mock LLM replies {string} without selecting, then selects template {string}, then replies {string}"
)]
fn given_mock_llm_selects_only_after_nudge(
    world: &mut QuectoWorld,
    first: String,
    template: String,
    last: String,
) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        fn text_body(id: &str, text: &str) -> serde_json::Value {
            serde_json::json!({
                "id": id,
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": text },
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            })
        }
        let select_body = serde_json::json!({
            "id": "chatcmpl-wf-nudged-select",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_wf_nudged_sel",
                        "type": "function",
                        "function": {
                            "name": "workflow",
                            "arguments": format!("{{\"action\":\"select_template\",\"template\":\"{template}\"}}")
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(text_body("chatcmpl-wf-unselected", &first)),
            )
            .up_to_n_times(1)
            .with_priority(2)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(select_body))
            .up_to_n_times(1)
            .with_priority(3)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(text_body("chatcmpl-wf-done", &last)),
            )
            .mount(&server)
            .await;

        e2e_steps::rewrite_config_to_uri(world, &new_uri);
        world.wiremock_server_ref = Some(Box::leak(Box::new(server)));
    });
    std::mem::forget(rt);
}

/// #1113 AC3 regression: the selector nudge must fire even with workflow
/// auto-continue disabled — it is the sole proactive selection channel.
#[given("the config file disables workflow auto-continue")]
fn given_config_disables_workflow_auto_continue(world: &mut QuectoWorld) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("no base dir — add 'Given a temp base directory'");
    let path = base.join("config.json");
    let raw = std::fs::read_to_string(&path).expect("config.json must exist before this step");
    let mut config: serde_json::Value = serde_json::from_str(&raw).expect("config.json is JSON");
    config["workflow"]["auto_continue"] = serde_json::json!(false);
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
        .expect("rewrite config.json with workflow auto-continue disabled");
}

/// #1113 AC3, arming-to-dispatch integration: the template selector must
/// reach the model as an APPENDED (non-system) message on a request AFTER the
/// first — proving the idle-boundary nudge actually triggered a further LLM
/// request carrying the selector, not that the harness front-loaded it. The
/// first request is asserted selector-free so a session that injected the
/// selector up front (the retired system-prompt mechanism) fails here.
#[then("a nudged LLM request should carry the workflow template selector")]
fn then_nudged_llm_request_carries_template_selector(world: &mut QuectoWorld) {
    let server = world
        .wiremock_server_ref
        .expect("mock server ref not stored — use a #1113-aware workflow mock step");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let requests = rt
        .block_on(server.received_requests())
        .expect("received requests should be available");
    std::mem::forget(rt);
    const SELECTOR_MARKER: &str = "No workflow template is selected";
    let chat_messages: Vec<Vec<(String, String)>> = requests
        .iter()
        .filter(|request: &&Request| {
            request.method.as_str() == "POST" && request.url.path() == "/chat/completions"
        })
        .map(|request| {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("chat request body is JSON");
            body["messages"]
                .as_array()
                .expect("chat request has a messages array")
                .iter()
                .map(|m| {
                    (
                        m["role"].as_str().unwrap_or_default().to_string(),
                        m["content"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .collect();
    assert!(
        chat_messages.len() >= 2,
        "the idle-boundary nudge must trigger at least one further LLM request; got {}\nevents: {:#?}\nstderr: {}",
        chat_messages.len(),
        world.agent_events,
        world.agent_stderr
    );
    let first = &chat_messages[0];
    assert!(
        !first
            .iter()
            .any(|(_, content)| content.contains(SELECTOR_MARKER)),
        "the FIRST request must not carry the selector — it must arrive via a later idle-boundary nudge: {first:#?}"
    );
    let nudged = chat_messages[1..].iter().flatten().find(|(_, content)| {
        content.contains(SELECTOR_MARKER) && content.contains("select_template")
    });
    let (role, content) = nudged.unwrap_or_else(|| {
        panic!(
            "no later LLM request carried the template selector in its history\nrequests: {chat_messages:#?}\nevents: {:#?}\nstderr: {}",
            world.agent_events, world.agent_stderr
        )
    });
    assert_eq!(
        role, "user",
        "the selector must arrive as an appended user message (append-only channel), not '{role}': {content}"
    );
    assert!(
        content.contains("Available templates:"),
        "the selector nudge must list the available templates: {content}"
    );
}

/// Mock: LLM returns a tool call for `workflow status`, then a text reply.
#[given(expr = "the mock LLM returns a tool call for workflow status then text {string}")]
fn given_mock_llm_workflow_status(world: &mut QuectoWorld, text: String) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        let tool_call_body = serde_json::json!({
            "id": "chatcmpl-wf-status",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_wf_status",
                        "type": "function",
                        "function": {
                            "name": "workflow",
                            "arguments": "{\"action\":\"status\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let text_body = serde_json::json!({
            "id": "chatcmpl-wf-text2",
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

/// Helper: find all workflow_state events in a client's received event lines.
fn mc_client_workflow_events(world: &QuectoWorld, client_id: u32) -> Vec<serde_json::Value> {
    world
        .mc_client_events
        .get(&client_id)
        .map(|events| {
            events
                .iter()
                .filter_map(|l| {
                    let v: serde_json::Value = serde_json::from_str(l).ok()?;
                    if v["type"].as_str() == Some("workflow_state") {
                        Some(v)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[then(expr = "client {int} should have received a workflow_state event with mode {string}")]
fn then_client_received_workflow_mode(
    world: &mut QuectoWorld,
    client_id: u32,
    expected_mode: String,
) {
    execute_multi_client_uds(world);
    let wf_events = mc_client_workflow_events(world, client_id);
    let found = wf_events
        .iter()
        .any(|ev| ev["mode"].as_str() == Some(expected_mode.as_str()));
    assert!(
        found,
        "expected client {client_id} to receive workflow_state with mode={expected_mode:?}\nworkflow events: {wf_events:#?}\nall events: {:#?}",
        world.mc_client_events.get(&client_id),
    );
}

#[then(expr = "client {int} should have received a workflow_state event with progress done {int}")]
fn then_client_received_workflow_progress(
    world: &mut QuectoWorld,
    client_id: u32,
    expected_done: u64,
) {
    execute_multi_client_uds(world);
    let wf_events = mc_client_workflow_events(world, client_id);
    let found = wf_events
        .iter()
        .any(|ev| ev["progress"]["done"].as_u64() == Some(expected_done));
    assert!(
        found,
        "expected client {client_id} to receive workflow_state with progress.done={expected_done}\nworkflow events: {wf_events:#?}\nall events: {:#?}",
        world.mc_client_events.get(&client_id),
    );
}

#[then(expr = "client {int} should not have received a workflow_state event")]
fn then_client_not_received_workflow(world: &mut QuectoWorld, client_id: u32) {
    execute_multi_client_uds(world);
    let wf_events = mc_client_workflow_events(world, client_id);
    assert!(
        wf_events.is_empty(),
        "expected client {client_id} to have received NO workflow_state events, got: {wf_events:#?}",
    );
}

// ─── UDS extension execute_tool round-trip (FIX) ─────────────────────────────

/// Parameterised version of "the mock LLM returns a tool call then a text
/// response": lets us drive the agent into invoking ANY tool, not just bash.
#[given(
    expr = "the mock LLM returns a tool call for tool {string} with arguments {string} then text {string}"
)]
fn given_mock_llm_tool_call_named(
    world: &mut QuectoWorld,
    tool_name: String,
    arguments_raw: String,
    text: String,
) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );

    // Gherkin's {} placeholder gives us the raw argument string — strip any
    // surrounding quotes so callers can write either  {"city":"X"}  or  "…"
    let arguments = arguments_raw;

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
                        "id": format!("call_{tool_name}_1"),
                        "type": "function",
                        "function": {
                            "name": tool_name,
                            "arguments": arguments,
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

/// Queue a reactive auto-reply: when the harness later observes an
/// `execute_tool` event with the matching tool name on this client's
/// stream, it will auto-send a `tool_result` carrying `content`.
#[when(expr = "client {int} replies to execute_tool for {string} with content {string}")]
fn when_client_auto_reply(
    world: &mut QuectoWorld,
    client_id: u32,
    tool_name: String,
    content: String,
) {
    world
        .mc_auto_replies
        .entry(client_id)
        .or_default()
        .push((tool_name, content));
}

fn find_execute_tool_for(events: &[String], tool_name: &str) -> Option<serde_json::Value> {
    events.iter().find_map(|line| {
        let ev: serde_json::Value = serde_json::from_str(line).ok()?;
        if ev["type"].as_str() == Some("execute_tool") && ev["toolName"].as_str() == Some(tool_name)
        {
            Some(ev)
        } else {
            None
        }
    })
}

#[then(expr = "client {int} should have received an execute_tool for tool {string}")]
fn then_client_received_execute_tool(world: &mut QuectoWorld, client_id: u32, tool_name: String) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .expect("no events for client");
    let ev = find_execute_tool_for(events, &tool_name);
    assert!(
        ev.is_some(),
        "expected client {client_id} to receive execute_tool for tool {tool_name:?}\nevents: {events:#?}",
    );
}

#[then(expr = "client {int} should not have received an execute_tool for tool {string}")]
fn then_client_did_not_receive_execute_tool(
    world: &mut QuectoWorld,
    client_id: u32,
    tool_name: String,
) {
    execute_multi_client_uds(world);
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let leaked = find_execute_tool_for(&events, &tool_name);
    assert!(
        leaked.is_none(),
        "client {client_id} unexpectedly received an execute_tool for tool {tool_name:?}\nevents: {events:#?}",
    );
}

#[then(expr = "the execute_tool event for {string} should carry arguments containing {string}")]
fn then_execute_tool_args_contain(world: &mut QuectoWorld, tool_name: String, needle: String) {
    execute_multi_client_uds(world);
    let mut found_args: Option<String> = None;
    for events in world.mc_client_events.values() {
        if let Some(ev) = find_execute_tool_for(events, &tool_name) {
            if let Some(args) = ev["arguments"].as_str() {
                found_args = Some(args.to_string());
                break;
            }
        }
    }
    let args = found_args
        .unwrap_or_else(|| panic!("no execute_tool for {tool_name:?} observed on any client"));
    assert!(
        args.contains(&needle),
        "execute_tool arguments for {tool_name:?} did not contain {needle:?}\nactual: {args}",
    );
}

#[then(
    expr = "the tool catalogue response {string} should list tool {string} from source {string}"
)]
fn then_catalogue_response_lists_tool_source(
    world: &mut QuectoWorld,
    response_id: String,
    name: String,
    source: String,
) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&1).expect("no client 1 events");
    let resp = find_ge_response(events, &response_id)
        .unwrap_or_else(|| panic!("no {response_id} response"));
    let tools = resp["data"]["tools"].as_array().expect("no tools");
    let ext = tools
        .iter()
        .find(|e| e["name"].as_str() == Some(&name))
        .unwrap_or_else(|| panic!("'{name}' not in {tools:?}"));
    assert_eq!(ext["source"].as_str(), Some(source.as_str()));
}

#[then(expr = "the registered tool {string} should have a UDS client owner")]
fn then_post_register_lists_tool_owner(world: &mut QuectoWorld, name: String) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&1).expect("no client 1 events");
    let resp = find_ge_response(events, "ge-reg").expect("no ge-reg response");
    let tools = resp["data"]["tools"].as_array().expect("no tools");
    let ext = tools
        .iter()
        .find(|e| e["name"].as_str() == Some(&name))
        .unwrap_or_else(|| panic!("'{name}' not in {tools:?}"));
    let owner = ext["owner"].as_str().expect("missing owner");
    assert!(
        owner.starts_with("uds:client:"),
        "expected UDS client owner for {name:?}; got {owner:?}"
    );
}
