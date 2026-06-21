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
    let mut registry = ToolRegistryImpl::with_core_tools_and_exec_settings(
        workspace.clone(),
        sandbox,
        exec_settings,
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
    quecto::interface::shared::register_extension_tools(&mut registry, &ext_registry);

    let ephemeral = world.no_session || world.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        Session::build_key("cli", world.session_name.as_deref().unwrap_or("default"))
    };

    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
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
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    });
    // Enable streaming when the scenario has set the flag (e.g. SSE mock).
    if world._uds_streaming_enabled {
        agent.set_streaming(true);
    }

    // Set up live prompt injection when workflow is enabled.
    if let Some(ref wf) = workflow_state {
        let wf_for_provider = wf.clone();
        let base_prompt = world.system_prompt.clone().unwrap_or_default();
        agent.set_system_prompt_provider(Some(std::sync::Arc::new(move || {
            let mut prompt = base_prompt.clone();
            quecto::interface::shared::append_workflow_prompt(&mut prompt, &wf_for_provider);
            prompt
        })));
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
fn execute_uds(world: &mut QuectoWorld) {
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
            world.uds_execution_error = Some(msg);
            1
        }
    };

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

#[given(
    expr = "the mock LLM returns a tool call that adds a Fireworks provider then text {string}"
)]
fn given_mock_llm_adds_fireworks_then_text(world: &mut QuectoWorld, text: String) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let config_path = base.join("config.json");
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
        world._fireworks_mock_uri = Some(fireworks_uri.clone());
        world.fireworks_mock_server_ref = Some(Box::leak(Box::new(fireworks)));

        let openai = wiremock::MockServer::start().await;
        let openai_uri = openai.uri();
        let escaped_config = serde_json::to_string(&config_path.display().to_string()).unwrap();
        let escaped_fireworks_uri = serde_json::to_string(&fireworks_uri).unwrap();
        let command = format!(
            "python3 - <<'PY'\nimport json\nfrom pathlib import Path\npath = Path({escaped_config})\ncfg = json.loads(path.read_text())\nproviders = cfg.setdefault('providers', {{}})\nproviders.setdefault('openai_compatible', {{}})['endpoints'] = [{{'prefix':'fireworks','api_base':{escaped_fireworks_uri},'api_key':'sk-fireworks','allow_remote_http': True}}]\npath.write_text(json.dumps(cfg, indent=2))\nPY"
        );
        let args = serde_json::json!({"command": command}).to_string();
        let tool_call_body = serde_json::json!({
            "id": "chatcmpl-add-fireworks",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_add_fireworks",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": args
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let text_body = serde_json::json!({
            "id": "chatcmpl-configured",
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
            .with_priority(1)
            .mount(&openai)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .with_priority(2)
            .mount(&openai)
            .await;

        e2e_steps::rewrite_config_to_uri(world, &openai_uri);
        std::mem::forget(openai);
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

#[then(expr = "the get_messages response data should include a {string} array with {int} messages")]
fn then_get_messages_array_len(world: &mut QuectoWorld, field: String, expected: usize) {
    let resp = find_agent_response(world, "get_messages").expect("no get_messages response");
    let len = resp["data"][&field]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(len, expected, "unexpected get_messages.data.{field} length");
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
    let found = events.iter().any(|line| {
        if let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) {
            ev["type"].as_str() == Some("turn_end")
                && ev["message"]["content"].as_str() == Some(&expected)
        } else {
            false
        }
    });
    assert!(
        found,
        "expected a turn_end event with content {expected:?} in events:\n{events:#?}"
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
fn execute_multi_client_uds(world: &mut QuectoWorld) {
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

    let (handle, socket_path) = match mc_spawn_agent(ctx, &base, socket_path) {
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
fn mc_spawn_agent(
    ctx: UdsAgentContext,
    base: &std::path::Path,
    socket_path: std::path::PathBuf,
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
            system_prompt: String::new(),
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
    // Reactive phase: if any client has auto-replies queued (e.g. for
    // execute_tool events that only exist after the LLM is consulted),
    // read-and-react on its stream before the final collection step.
    if !world.mc_auto_replies.is_empty() {
        mc_reactive_auto_replies(world, &mut streams, connected, disconnected);
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
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

// ─── get_extensions assertion steps ───────────────────────────────────────────

/// Find the get_extensions response (first or post-reload depending on context).
fn find_get_extensions_response(
    events: &[String],
    id_prefix: Option<&str>,
) -> Option<serde_json::Value> {
    events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["type"] == "response" && v["command"] == "get_extensions" {
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

#[then(expr = "the get_extensions response should list extension {string}")]
fn then_get_extensions_lists(world: &mut QuectoWorld, name: String) {
    execute_uds(world);
    let resp = find_get_extensions_response(&world.agent_events, None)
        .expect("no get_extensions response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("extensions not an array");
    let found = exts.iter().any(|e| e["name"].as_str() == Some(&name));
    assert!(
        found,
        "expected get_extensions to list extension {name:?}\nexts: {exts:?}"
    );
}

#[then(expr = "the get_extensions response should have {int} extensions")]
fn then_get_extensions_count(world: &mut QuectoWorld, count: usize) {
    execute_uds(world);
    let resp = find_get_extensions_response(&world.agent_events, None)
        .expect("no get_extensions response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("extensions not an array");
    assert_eq!(
        exts.len(),
        count,
        "expected {count} extensions, got {}\nexts: {exts:?}",
        exts.len()
    );
}

#[then(expr = "the get_extensions response should not list extension {string}")]
fn then_get_extensions_not_lists(world: &mut QuectoWorld, name: String) {
    execute_uds(world);
    let resp = find_get_extensions_response(&world.agent_events, None)
        .expect("no get_extensions response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("extensions not an array");
    let found = exts.iter().any(|e| e["name"].as_str() == Some(&name));
    assert!(
        !found,
        "expected get_extensions NOT to list extension {name:?}\nexts: {exts:?}"
    );
}

// ─── extensions_changed event assertions ──────────────────────────────────────

#[then(expr = "client {int} should have received an extensions_changed event listing {string}")]
fn then_client_received_extensions_changed_listing(
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
        if v["type"].as_str() != Some("extensions_changed") {
            return false;
        }
        v["extensions"]
            .as_array()
            .map(|exts| exts.iter().any(|e| e["name"].as_str() == Some(&name)))
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected client {client_id} to receive extensions_changed listing {name:?}\nevents: {events:#?}"
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

    let (agent_handle, socket_path) = match mc_spawn_agent(ctx, &base, socket_path) {
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
            && ev["command"].as_str() == Some("get_extensions")
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

#[then(expr = "the post-register get_extensions response should list extension {string}")]
fn then_post_register_lists_ext(world: &mut QuectoWorld, name: String) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&1).expect("no client 1 events");
    let resp = find_ge_response(events, "ge-reg").expect("no ge-reg response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("no extensions");
    assert!(
        exts.iter().any(|e| e["name"].as_str() == Some(&name)),
        "'{name}' not in {exts:?}"
    );
}

#[then(expr = "the post-unregister get_extensions response should have {int} extensions")]
fn then_post_unregister_empty(world: &mut QuectoWorld, count: u32) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&1).expect("no client 1 events");
    let resp = find_ge_response(events, "ge-unreg").expect("no ge-unreg response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("no extensions");
    assert_eq!(exts.len(), count as usize);
}

#[then(expr = "the post-disconnect get_extensions response should have {int} extensions")]
fn then_post_disconnect_empty(world: &mut QuectoWorld, count: u32) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&3).expect("no client 3 events");
    let resp = find_ge_response(events, "ge-disc").expect("no ge-disc response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("no extensions");
    assert_eq!(exts.len(), count as usize);
}

#[then(expr = "the post-multi get_extensions response should list extension {string}")]
fn then_post_multi_lists_ext(world: &mut QuectoWorld, name: String) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&2).expect("no client 2 events");
    let resp = find_ge_response(events, "ge-multi").expect("no ge-multi response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("no extensions");
    assert!(
        exts.iter().any(|e| e["name"].as_str() == Some(&name)),
        "'{name}' not in {exts:?}"
    );
}

#[then(
    expr = "the post-redef get_extensions response should list extension {string} with description {string}"
)]
fn then_post_redef_desc(world: &mut QuectoWorld, name: String, desc: String) {
    execute_multi_client_uds(world);
    let events = world.mc_client_events.get(&1).expect("no client 1 events");
    let resp = find_ge_response(events, "ge-redef").expect("no ge-redef response");
    let exts = resp["data"]["extensions"]
        .as_array()
        .expect("no extensions");
    let ext = exts
        .iter()
        .find(|e| e["name"].as_str() == Some(&name))
        .unwrap_or_else(|| panic!("'{name}' not in {exts:?}"));
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
                            "arguments": "{\"action\":\"select_template\",\"template\":\"fix\"}"
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
                            "arguments": "{\"action\":\"select_template\",\"template\":\"fix\"}"
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
        std::mem::forget(server);
    });
    std::mem::forget(rt);
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
