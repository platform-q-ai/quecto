//! Environments outlive sessions (#2024 S4d), real process: the harness
//! itself is ended without cleanup after committing a container, and the
//! next real harness lists, joins and (through the CLI) kills it. The
//! fake script set is `container_persistence_steps`'.
use super::*;

use crate::container_mapping_steps::{REAL_AGENT_RUN, RealAgentRun};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ─── Real process: the harness itself restarts ───────────────────────────────

const SPAWN_MARKER: &str = "PERSIST_SPAWN_CONTAINER";
const JOIN_MARKER: &str = "PERSIST_JOIN_CONTAINER";
const REAL_AGENT_TIMEOUT: Duration = Duration::from_secs(120);

/// A provider that, on a turn carrying [`SPAWN_MARKER`], answers one
/// `spawn container: true` and then — after the tool result — stalls for
/// the scenario's kill window; on a turn carrying [`JOIN_MARKER`] it
/// answers `agent_cmd get_containers`, then `spawn` into `C1`, then a
/// reply. Every `tool` message is captured.
fn mount_provider(server: &wiremock::MockServer, seen: Arc<Mutex<Vec<String>>>) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                let input: serde_json::Value = request.body_json().unwrap();
                let messages = input["messages"].as_array().cloned().unwrap_or_default();
                for tool_message in messages.iter().filter(|m| m["role"] == "tool") {
                    if let Some(content) = tool_message["content"].as_str() {
                        let mut seen = seen.lock().unwrap();
                        if !seen.iter().any(|s| s == content) {
                            seen.push(content.to_string());
                        }
                    }
                }
                let user_has = |marker: &str| {
                    messages.iter().any(|m| {
                        m["role"] == "user"
                            && m["content"]
                                .as_str()
                                .is_some_and(|content| content.contains(marker))
                    })
                };
                let tool_results = messages.iter().filter(|m| m["role"] == "tool").count();
                let call = |name: &str, arguments: serde_json::Value| {
                    serde_json::json!({"tool_calls": [{
                        "index": 0, "id": format!("call-{name}-{tool_results}"), "type": "function",
                        "function": {"name": name, "arguments": arguments.to_string()}
                    }]})
                };
                let mut delay = Duration::ZERO;
                let (delta, finish) = if user_has(SPAWN_MARKER) {
                    match tool_results {
                        0 => (
                            call(
                                "spawn",
                                serde_json::json!({"agent_id": "persist-child", "task": "wait", "container": true, "read_only": true}),
                            ),
                            "tool_calls",
                        ),
                        _ => {
                            // The kill window: the scenario ends this
                            // harness before it can tear its child down.
                            delay = Duration::from_secs(60);
                            (serde_json::json!({"content": "REAL_AGENT_DONE"}), "stop")
                        }
                    }
                } else if user_has(JOIN_MARKER) {
                    match tool_results {
                        0 => (
                            call(
                                "agent_cmd",
                                serde_json::json!({"agent_id": "*", "command": "get_containers"}),
                            ),
                            "tool_calls",
                        ),
                        1 => (
                            call(
                                "spawn",
                                serde_json::json!({"agent_id": "persist-joiner", "task": "wait", "container": {"mode": "existing", "ref": "C1"}, "read_only": true}),
                            ),
                            "tool_calls",
                        ),
                        _ => (serde_json::json!({"content": "REAL_AGENT_DONE"}), "stop"),
                    }
                } else {
                    (serde_json::json!({"content": "REAL_AGENT_DONE"}), "stop")
                };
                let template = if input["stream"] == true {
                    let chunk = serde_json::json!({
                        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                        "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
                    });
                    wiremock::ResponseTemplate::new(200).set_body_raw(
                        format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                        "text/event-stream",
                    )
                } else {
                    let mut message = delta.clone();
                    message["role"] = serde_json::json!("assistant");
                    wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "chatcmpl-persist", "object": "chat.completion",
                        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
                        "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
                    }))
                };
                template.set_delay(delay)
            })
            .mount(server),
    );
    std::mem::forget(rt);
}

/// Point the session config's provider at a fresh fake endpoint (once per
/// scenario) and return the captured tool results.
fn provider_for_real_agents(world: &mut QuectoWorld) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    mount_provider(&server, seen.clone());
    let uri = server.uri();
    std::mem::forget(server);
    std::mem::forget(rt);
    let config_path = PathBuf::from(world.config_path.clone().expect("config path"));
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["providers"]["openai"]["api_base"] = serde_json::json!(uri);
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    seen
}

fn start_real_agent(world: &QuectoWorld, session: &str, prompt: &str) -> std::process::Child {
    let binary = PathBuf::from(std::env::var_os("QUECTO_CHILD_BINARY").expect("child binary"));
    let config_path = PathBuf::from(world.config_path.clone().expect("config path"));
    std::process::Command::new(&binary)
        .args(["agent", "-m", prompt, "-s", session, "--config"])
        .arg(&config_path)
        .current_dir(world.cli_context.cwd.clone().expect("cwd"))
        .env("QUECTO_BASE_DIR", base_path(world))
        .env("QUECTO_CHILD_BINARY", &binary)
        .env_remove("QUECTO_RUNTIME_CONFIG_PATH")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start the real quecto agent")
}

#[given(
    "a real quecto agent driven by a fake provider spawns container true and is then killed without cleanup"
)]
fn given_real_agent_spawns_then_dies(world: &mut QuectoWorld) {
    let seen = provider_for_real_agents(world);
    let mut child = start_real_agent(world, "persist-one", SPAWN_MARKER);
    // Wait for the create to be committed durably, then end the harness
    // the way a crash does: no shutdown, no teardown.
    let started = std::time::Instant::now();
    let registry = base_path(world).join("environments.json");
    loop {
        let committed = std::fs::read_to_string(&registry)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|doc| doc["environments"]["C1"]["status"] == "running");
        if committed {
            break;
        }
        if let Some(status) = child.try_wait().expect("poll the real agent") {
            let output = child.wait_with_output().unwrap();
            panic!(
                "the real agent exited ({status}) before committing C1\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(
            started.elapsed() < REAL_AGENT_TIMEOUT,
            "C1 was not committed within {REAL_AGENT_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().expect("end the real agent without cleanup");
    let _ = child.wait();
    *REAL_AGENT_RUN.lock().unwrap() = Some(RealAgentRun {
        tool_results: seen,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
    });
}

#[when(
    expr = "a real quecto agent started as session {string} is driven by the fake provider to list containers and join {string}"
)]
fn when_real_agent_joins(world: &mut QuectoWorld, session: String, env_ref: String) {
    assert_eq!(env_ref, "C1", "the provider joins C1");
    let seen = REAL_AGENT_RUN
        .lock()
        .unwrap()
        .as_ref()
        .expect("the first real agent ran")
        .tool_results
        .clone();
    let mut child = start_real_agent(world, &session, JOIN_MARKER);
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the real agent") {
            break status;
        }
        assert!(
            started.elapsed() < REAL_AGENT_TIMEOUT,
            "the real agent did not exit within {REAL_AGENT_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = child.wait_with_output().unwrap();
    *REAL_AGENT_RUN.lock().unwrap() = Some(RealAgentRun {
        tool_results: seen,
        exit_code: status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    });
}

#[then(
    expr = "the tool result the fake provider received should list container {string} as restored"
)]
fn then_provider_saw_restored(_world: &mut QuectoWorld, env_ref: String) {
    let run = REAL_AGENT_RUN.lock().unwrap();
    let run = run.as_ref().expect("the real agent ran");
    let results = run.tool_results.lock().unwrap();
    let listed = results.iter().any(|content| {
        serde_json::from_str::<serde_json::Value>(content).is_ok_and(|value| {
            value["containers"].as_array().is_some_and(|containers| {
                containers.iter().any(|c| {
                    c["ref"] == env_ref.as_str() && c["restored"] == true && c["status"] == "empty"
                })
            })
        })
    });
    assert!(
        listed,
        "expected a get_containers result listing {env_ref} restored among: {results:?}\nstderr: {}",
        run.stderr
    );
}

#[then(expr = "the tool result the fake provider received should be a join into {string}")]
fn then_provider_saw_join(_world: &mut QuectoWorld, env_ref: String) {
    let run = REAL_AGENT_RUN.lock().unwrap();
    let run = run.as_ref().expect("the real agent ran");
    let results = run.tool_results.lock().unwrap();
    let expected = format!("environment_ref={env_ref}");
    assert!(
        results.iter().any(|content| {
            content.contains("is running (uuid=") && content.contains(&expected)
        }),
        "expected a spawn result joining {env_ref} among: {results:?}\nstderr: {}",
        run.stderr
    );
}
