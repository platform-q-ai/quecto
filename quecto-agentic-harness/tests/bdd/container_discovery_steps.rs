//! Container discoverability for agents (#2024 S4c): the spawn description
//! carries the effective container-config roster of the launching agent's
//! checkout, `agent_cmd get_container_configs` returns that set with each
//! entry's source and repository, and the spawn/swarm descriptions say
//! what a new container is. The rig is the real `SpawnTool` and
//! `AgentCmdTool`, composed for a checkout the way the agent build
//! composes them, over the S4a fixture (fake create scripts, global
//! `default` + `alternate`, overlay written by `quecto config set --local`).
use super::*;

use crate::container_mapping_steps::{REAL_AGENT_RUN, RealAgentRun, config_set_local};
use crate::spawn_env_steps::run_container_command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ROSTER_PREFIX: &str = "Available container configs: ";
const DISCOVERY_MARKER: &str = "RUN_A_SUBAGENT_IN_THIS_REPOS_CONTAINER";
const REAL_AGENT_TIMEOUT: Duration = Duration::from_secs(120);

/// What the fake provider observed of the real agent's requests: the
/// `spawn` tool description it was offered, and the tool calls it issued
/// in order with the results the harness returned for them.
#[derive(Default)]
pub(crate) struct DiscoveryTranscript {
    pub(crate) spawn_description: Option<String>,
    /// `(tool name, arguments JSON)` in the order the fake issued them.
    pub(crate) calls: Vec<(String, String)>,
}

pub(crate) static DISCOVERY_TRANSCRIPT: Mutex<Option<Arc<Mutex<DiscoveryTranscript>>>> =
    Mutex::new(None);

fn checkout(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .cwd
        .clone()
        .expect("BDD world should pin a hermetic cwd")
}

fn overlay_path(world: &QuectoWorld) -> String {
    checkout(world)
        .join(".quecto")
        .join("config.json")
        .display()
        .to_string()
}

/// The roster line of the composed spawn tool's description: the one line
/// starting with the roster prefix.
fn roster_line(world: &QuectoWorld) -> String {
    let description = spawn_description(world);
    description
        .lines()
        .find(|line| line.starts_with(ROSTER_PREFIX))
        .unwrap_or_else(|| panic!("no roster line in the spawn description:\n{description}"))
        .to_string()
}

fn spawn_description(world: &QuectoWorld) -> String {
    world
        .spawn_tool
        .as_ref()
        .expect("spawn tool")
        .definition()
        .description
        .to_string()
}

fn listing(world: &QuectoWorld) -> serde_json::Value {
    let result = world
        .container_cmd_result
        .as_ref()
        .expect("get_container_configs result");
    assert!(
        !result.is_error,
        "get_container_configs failed: {}",
        result.content
    );
    serde_json::from_str(&result.content).unwrap_or_else(|e| {
        panic!(
            "get_container_configs should return JSON: {e}; got {}",
            result.content
        )
    })
}

fn listing_entry(world: &QuectoWorld, name: &str) -> Option<serde_json::Value> {
    listing(world)["container_configs"]
        .as_array()
        .expect("container_configs is an array")
        .iter()
        .find(|entry| entry["name"].as_str() == Some(name))
        .cloned()
}

#[when("the launcher is composed for the checkout with container discovery")]
fn when_launcher_composed_with_discovery(world: &mut QuectoWorld) {
    let base = base_path(world);
    let parent_config = PathBuf::from(world.config_path.clone().expect("config path"));
    let registry = world
        .agent_cmd_registry
        .clone()
        .expect("registry from live spawn setup");
    let checkout = checkout(world);
    let spawn = quecto::composition::subagent_lifecycle::compose_launcher_in_checkout(
        SpawnTool::with_base_dir(vec![], base.clone())
            .with_socket_dir(base.join("sockets"))
            .with_registry(registry.clone())
            .with_parent_config_path(Some(parent_config)),
        &checkout,
    );
    let roster = spawn
        .container_config_roster()
        .expect("the checkout launcher composes the roster query");
    world.agent_cmd_tool = Some(
        quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new(registry)
            .with_container_config_roster(Some(roster)),
    );
    world.spawn_tool = Some(spawn);
}

#[given(
    expr = "the checkout adds {int} non-default container configs named {string} through quecto config set --local"
)]
fn given_checkout_adds_many(world: &mut QuectoWorld, count: usize, prefix: String) {
    let config_path = world.config_path.clone().expect("config path");
    let global: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    let entry = &global["container_configs"]["default"];
    let create = entry["create"][0].as_str().unwrap().to_string();
    let cleanup = entry["cleanup"][0].as_str().unwrap().to_string();
    for index in 1..=count {
        let name = format!("{prefix}-{index:02}");
        let entry = serde_json::json!({"create": [create], "cleanup": [cleanup]});
        config_set_local(world, &name, &entry);
    }
}

#[then(expr = "the spawn description should carry the roster line {string}")]
fn then_roster_line_is(world: &mut QuectoWorld, expected: String) {
    let line = roster_line(world);
    assert_eq!(line, expected);
}

#[then(expr = "the spawn description roster line should be at most {int} characters")]
fn then_roster_line_at_most(world: &mut QuectoWorld, max: usize) {
    let line = roster_line(world);
    assert!(
        line.chars().count() <= max,
        "roster line is {} characters: {line}",
        line.chars().count()
    );
}

#[then(expr = "the spawn description roster line should end with {string}")]
fn then_roster_line_ends_with(world: &mut QuectoWorld, suffix: String) {
    let line = roster_line(world);
    assert!(line.ends_with(&suffix), "{line}");
}

#[then(expr = "the spawn description roster line should name {string}")]
fn then_roster_line_names(world: &mut QuectoWorld, needle: String) {
    let line = roster_line(world);
    assert!(line.contains(&needle), "{line}");
}

#[then(expr = "the spawn description should say {string}")]
fn then_spawn_description_says(world: &mut QuectoWorld, needle: String) {
    let description = spawn_description(world);
    assert!(
        description.contains(&needle),
        "spawn description misses {needle:?}:\n{description}"
    );
}

#[then(expr = "the swarm description should say {string}")]
fn then_swarm_description_says(_world: &mut QuectoWorld, needle: String) {
    let description =
        include_str!("../../src/infrastructure/tools/swarm_helpers/tool_description.txt");
    assert!(
        description.contains(&needle),
        "swarm description misses {needle:?}:\n{description}"
    );
}

#[then(expr = "the agent_cmd description should say {string}")]
fn then_agent_cmd_description_says(world: &mut QuectoWorld, needle: String) {
    let description = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd tool")
        .definition()
        .description
        .to_string();
    assert!(
        description.contains(&needle),
        "agent_cmd description misses {needle:?}:\n{description}"
    );
}

#[when("I run agent_cmd get_container_configs")]
fn when_get_container_configs(world: &mut QuectoWorld) {
    let result = run_container_command(
        world,
        serde_json::json!({"agent_id": "*", "command": "get_container_configs"}),
    );
    world.container_cmd_result = Some(result);
}

#[then("the container config listing should not be an error")]
fn then_listing_ok(world: &mut QuectoWorld) {
    let listing = listing(world);
    assert!(
        listing["container_configs"].is_array(),
        "container_configs is an array: {listing}"
    );
}

#[then(expr = "the container config listing should fail with {string}")]
fn then_listing_fails(world: &mut QuectoWorld, expected: String) {
    let result = world
        .container_cmd_result
        .as_ref()
        .expect("get_container_configs result");
    assert!(
        result.is_error && result.content.contains(&expected),
        "expected an error containing {expected:?}, got: {}",
        result.content
    );
}

#[then(
    expr = "the container config listing should have entry {string} with default {word}, source {string} and repository {string}"
)]
fn then_listing_entry_with_repo(
    world: &mut QuectoWorld,
    name: String,
    default: String,
    source: String,
    repository: String,
) {
    let entry = listing_entry(world, &name).unwrap_or_else(|| panic!("no entry {name}"));
    assert_eq!(
        entry["default"],
        serde_json::json!(default == "true"),
        "{entry}"
    );
    assert_eq!(entry["source"], serde_json::json!(source), "{entry}");
    assert_eq!(
        entry["repository"],
        serde_json::json!(repository),
        "{entry}"
    );
}

#[then(
    expr = "the container config listing should have entry {string} with default {word}, source {string} and no repository"
)]
fn then_listing_entry_no_repo(
    world: &mut QuectoWorld,
    name: String,
    default: String,
    source: String,
) {
    let entry = listing_entry(world, &name).unwrap_or_else(|| panic!("no entry {name}"));
    assert_eq!(
        entry["default"],
        serde_json::json!(default == "true"),
        "{entry}"
    );
    assert_eq!(entry["source"], serde_json::json!(source), "{entry}");
    assert!(entry["repository"].is_null(), "{entry}");
}

#[then(expr = "the container config listing should not have entry {string}")]
fn then_listing_lacks_entry(world: &mut QuectoWorld, name: String) {
    assert!(listing_entry(world, &name).is_none(), "{}", listing(world));
}

#[then("the container config listing should report overlay_withheld false with no diagnostics")]
fn then_listing_not_withheld(world: &mut QuectoWorld) {
    let listing = listing(world);
    assert_eq!(
        listing["overlay_withheld"],
        serde_json::json!(false),
        "{listing}"
    );
    assert_eq!(listing["diagnostics"], serde_json::json!([]), "{listing}");
}

#[then(
    "the container config listing should report overlay_withheld true with a diagnostic naming the checkout's overlay"
)]
fn then_listing_withheld(world: &mut QuectoWorld) {
    let listing = listing(world);
    assert_eq!(
        listing["overlay_withheld"],
        serde_json::json!(true),
        "{listing}"
    );
    let overlay = overlay_path(world);
    let diagnostics = listing["diagnostics"]
        .as_array()
        .expect("diagnostics array");
    assert!(
        diagnostics.iter().any(|line| {
            line.as_str()
                .is_some_and(|line| line.contains(&overlay) && line.contains("quecto config trust"))
        }),
        "{listing}"
    );
}

// ── Real agent acceptance ──────────────────────────────────────────────────

/// A provider that plays the model asked to "run a subagent in this repo's
/// container": on the user turn carrying [`DISCOVERY_MARKER`] it first
/// calls `agent_cmd get_container_configs`, then `spawn {"container":
/// true}`, then replies; every other conversation (the container child's
/// turns included) gets a plain reply. The `spawn` description the request
/// offered, the calls it issued, and every `tool` message are captured.
fn mount_discovering_provider(
    server: &wiremock::MockServer,
    transcript: Arc<Mutex<DiscoveryTranscript>>,
    seen: Arc<Mutex<Vec<String>>>,
) {
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
                let is_discoverer = messages.iter().any(|m| {
                    m["role"] == "user"
                        && m["content"]
                            .as_str()
                            .is_some_and(|content| content.contains(DISCOVERY_MARKER))
                });
                if is_discoverer {
                    let mut transcript = transcript.lock().unwrap();
                    if transcript.spawn_description.is_none() {
                        transcript.spawn_description = input["tools"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|tool| tool["function"]["name"] == "spawn")
                            .and_then(|tool| tool["function"]["description"].as_str())
                            .map(str::to_string);
                    }
                }
                let tool_results = messages.iter().filter(|m| m["role"] == "tool").count();
                let call = if !is_discoverer {
                    None
                } else if tool_results == 0 {
                    Some((
                        "agent_cmd",
                        serde_json::json!({"agent_id": "*", "command": "get_container_configs"}),
                    ))
                } else if tool_results == 1 {
                    Some((
                        "spawn",
                        serde_json::json!({
                            "agent_id": "repo-container-child", "task": "wait",
                            "container": true, "read_only": true
                        }),
                    ))
                } else {
                    None
                };
                let (delta, finish) = match call {
                    Some((name, arguments)) => {
                        let arguments = arguments.to_string();
                        transcript
                            .lock()
                            .unwrap()
                            .calls
                            .push((name.to_string(), arguments.clone()));
                        (
                            serde_json::json!({"tool_calls": [{
                                "index": 0, "id": format!("call-{name}-{tool_results}"),
                                "type": "function",
                                "function": {"name": name, "arguments": arguments}
                            }]}),
                            "tool_calls",
                        )
                    }
                    None => (serde_json::json!({"content": "REAL_AGENT_DONE"}), "stop"),
                };
                if input["stream"] == true {
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
                        "id": "chatcmpl-real-agent", "object": "chat.completion",
                        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
                        "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
                    }))
                }
            })
            .mount(server),
    );
    std::mem::forget(rt);
}

#[when(
    "a real quecto agent started in the checkout is asked by a fake provider to run a subagent in this repo's container"
)]
fn when_real_agent_discovers_and_spawns(world: &mut QuectoWorld) {
    let binary = PathBuf::from(std::env::var_os("QUECTO_CHILD_BINARY").expect("child binary"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let transcript = Arc::new(Mutex::new(DiscoveryTranscript::default()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    mount_discovering_provider(&server, transcript.clone(), seen.clone());
    let uri = server.uri();
    std::mem::forget(server);
    std::mem::forget(rt);
    let config_path = PathBuf::from(world.config_path.clone().expect("config path"));
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["providers"]["openai"]["api_base"] = serde_json::json!(uri);
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    let checkout = checkout(world);
    let mut child = std::process::Command::new(&binary)
        .args(["agent", "-m", DISCOVERY_MARKER, "--no-session"])
        .current_dir(&checkout)
        .env("QUECTO_BASE_DIR", base_path(world))
        .env("QUECTO_CHILD_BINARY", &binary)
        .env_remove("QUECTO_RUNTIME_CONFIG_PATH")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start the real quecto agent");
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
    let output = child
        .wait_with_output()
        .expect("collect the real agent's output");
    *REAL_AGENT_RUN.lock().unwrap() = Some(RealAgentRun {
        tool_results: seen,
        exit_code: status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    });
    *DISCOVERY_TRANSCRIPT.lock().unwrap() = Some(transcript);
}

fn transcript() -> Arc<Mutex<DiscoveryTranscript>> {
    DISCOVERY_TRANSCRIPT
        .lock()
        .unwrap()
        .clone()
        .expect("the real agent ran")
}

#[then(expr = "the spawn description the fake provider received should carry {string}")]
fn then_provider_saw_roster(_world: &mut QuectoWorld, needle: String) {
    let transcript = transcript();
    let transcript = transcript.lock().unwrap();
    let description = transcript
        .spawn_description
        .as_ref()
        .expect("the provider was offered the spawn tool");
    assert!(
        description.contains(&needle),
        "expected {needle:?} in the spawn description the model saw:\n{description}"
    );
}

#[then(
    expr = "the fake provider's get_container_configs result should list entry {string} from source {string}"
)]
fn then_provider_saw_listing(_world: &mut QuectoWorld, name: String, source: String) {
    let run = REAL_AGENT_RUN.lock().unwrap();
    let run = run.as_ref().expect("the real agent ran");
    let results = run.tool_results.lock().unwrap();
    let listed = results.iter().any(|content| {
        serde_json::from_str::<serde_json::Value>(content).is_ok_and(|value| {
            value["container_configs"]
                .as_array()
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry["name"].as_str() == Some(name.as_str())
                            && entry["source"].as_str() == Some(source.as_str())
                    })
                })
        })
    });
    assert!(
        listed,
        "expected a get_container_configs result listing {name} from {source} among: {results:?}\nstderr: {}",
        run.stderr
    );
}

#[then("the fake provider's first spawn call should have used container true")]
fn then_provider_first_spawn_container_true(_world: &mut QuectoWorld) {
    let transcript = transcript();
    let transcript = transcript.lock().unwrap();
    let (_, arguments) = transcript
        .calls
        .iter()
        .find(|(name, _)| name == "spawn")
        .expect("the model issued a spawn call");
    let arguments: serde_json::Value = serde_json::from_str(arguments).unwrap();
    assert_eq!(arguments["container"], serde_json::json!(true));
    assert_eq!(
        transcript.calls.first().map(|(name, _)| name.as_str()),
        Some("agent_cmd"),
        "the model consulted get_container_configs first: {:?}",
        transcript.calls
    );
}
