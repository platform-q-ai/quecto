//! Repo → container mapping (#2024 S4a): `spawn container: true` selects
//! the container config the launching agent's checkout binds through its
//! trusted `.quecto/config.json` overlay, resolved by the configuration
//! capability (one overlay, one trust record) and reached by the launch
//! policy through its own port. The rig is the real `SpawnTool`, composed
//! for a checkout the way the agent build composes it for its working
//! directory, with fake container scripts that record the argv they got.
use super::*;

use crate::spawn_tool_steps::{execute_spawn_json_without_config, given_script_spawn};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Every `tool` message the fake provider saw from the real agent: what
/// the model was shown as the spawn's result.
#[derive(Default)]
pub(crate) struct RealAgentRun {
    pub(crate) tool_results: Arc<Mutex<Vec<String>>>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) static REAL_AGENT_RUN: Mutex<Option<RealAgentRun>> = Mutex::new(None);

const SPAWN_MARKER: &str = "SPAWN_CONTAINER_TRUE";
const REAL_AGENT_TIMEOUT: Duration = Duration::from_secs(120);

/// A provider whose first answer to a user turn carrying [`SPAWN_MARKER`]
/// is one `spawn {"container": true}` tool call, and whose every other
/// answer (the spawned child's turns included) is a plain reply. The
/// `tool` messages of every request are captured.
fn mount_spawning_provider(server: &wiremock::MockServer, seen: Arc<Mutex<Vec<String>>>) {
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
                let is_spawner = messages.iter().any(|m| {
                    m["role"] == "user"
                        && m["content"]
                            .as_str()
                            .is_some_and(|content| content.contains(SPAWN_MARKER))
                });
                let tool_results = messages.iter().filter(|m| m["role"] == "tool").count();
                let (delta, finish) = if is_spawner && tool_results == 0 {
                    let arguments = serde_json::json!({
                        "agent_id": "container-child", "task": "wait", "container": true, "read_only": true
                    })
                    .to_string();
                    (
                        serde_json::json!({"tool_calls": [{
                            "index": 0, "id": "call-container", "type": "function",
                            "function": {"name": "spawn", "arguments": arguments}
                        }]}),
                        "tool_calls",
                    )
                } else {
                    (serde_json::json!({"content": "REAL_AGENT_DONE"}), "stop")
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
    world_runtime_keep(rt);
}

/// The runtimes a leaked mock server lives on must outlive the scenario.
fn world_runtime_keep(rt: tokio::runtime::Runtime) {
    std::mem::forget(rt);
}

/// The checkout the launching agent works in: the world's hermetic cwd,
/// which `quecto config set --local` writes the overlay into.
fn checkout(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .cwd
        .clone()
        .expect("BDD world should pin a hermetic cwd")
}

/// The fake create/cleanup scripts the global default entry points at, so
/// an overlay entry can reuse them with its own `--repo` argv.
fn fake_scripts(world: &QuectoWorld) -> (String, String) {
    let config_path = world.config_path.clone().expect("config path");
    let global: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    let entry = &global["container_configs"]["default"];
    (
        entry["create"][0].as_str().unwrap().to_string(),
        entry["cleanup"][0].as_str().unwrap().to_string(),
    )
}

fn overlay_entry(world: &QuectoWorld, repo: &str, default: bool) -> serde_json::Value {
    let (create, cleanup) = fake_scripts(world);
    let mut entry = serde_json::json!({
        "create": [create, "--repo", repo],
        "cleanup": [cleanup],
    });
    if default {
        entry["default"] = serde_json::json!(true);
    }
    entry
}

/// `quecto config set --local container_configs.<name> '<entry>'`, the
/// agent-shaped command: it writes the overlay and records its trust.
pub(crate) fn config_set_local(world: &mut QuectoWorld, name: &str, entry: &serde_json::Value) {
    let output = cli::run_with_output(
        vec![
            "quecto".to_string(),
            "config".to_string(),
            "set".to_string(),
            "--local".to_string(),
            format!("container_configs.{name}"),
            entry.to_string(),
        ],
        &world.cli_context,
    );
    assert_eq!(
        output.exit_code, 0,
        "quecto config set --local failed:\nstdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
}

#[given(
    expr = "script-managed subagent spawning is available from a checkout with global default script {string}"
)]
pub(crate) fn given_script_spawn_from_checkout(world: &mut QuectoWorld, script: String) {
    given_script_spawn(world, script, None, None);
    let base = base_path(world);
    let parent_config = PathBuf::from(world.config_path.clone().expect("config path"));
    let registry = world
        .agent_cmd_registry
        .clone()
        .expect("registry from live spawn setup");
    let checkout = checkout(world);
    world.spawn_tool = Some(
        quecto::composition::subagent_lifecycle::compose_launcher_in_checkout(
            SpawnTool::with_base_dir(vec![], base.clone())
                .with_socket_dir(base.join("sockets"))
                .with_registry(registry)
                .with_parent_config_path(Some(parent_config)),
            &checkout,
        ),
    );
}

#[given(
    expr = "the checkout binds itself to container config {string} with repository {string} through quecto config set --local"
)]
fn given_checkout_binds_default(world: &mut QuectoWorld, name: String, repo: String) {
    let entry = overlay_entry(world, &repo, true);
    config_set_local(world, &name, &entry);
}

#[given(
    expr = "the checkout adds non-default container config {string} with repository {string} through quecto config set --local"
)]
fn given_checkout_adds_named(world: &mut QuectoWorld, name: String, repo: String) {
    let entry = overlay_entry(world, &repo, false);
    config_set_local(world, &name, &entry);
}

/// A global entry of the given name beside the global default: the
/// global file is rewritten by hand (no overlay, no trust involved) so
/// the rule "a repo-bound `standard` is the repo's default" (#2035) can
/// be shown not to reach an entry the global file declares.
#[given(expr = "the global configuration also declares non-default container config {string}")]
fn given_global_adds_named(world: &mut QuectoWorld, name: String) {
    let config_path = world.config_path.clone().expect("config path");
    let mut global: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let (create, cleanup) = fake_scripts(world);
    global["container_configs"][name] = serde_json::json!({
        "create": [create, "--repo", "https://example.test/global-standard"],
        "cleanup": [cleanup],
    });
    std::fs::write(&config_path, serde_json::to_string_pretty(&global).unwrap()).unwrap();
}

#[given(
    expr = "the checkout carries an untrusted overlay binding container config {string} with repository {string}"
)]
fn given_checkout_untrusted_overlay(world: &mut QuectoWorld, name: String, repo: String) {
    let entry = overlay_entry(world, &repo, true);
    write_untrusted_overlay(
        world,
        serde_json::json!({"container_configs": {name: entry}}),
    );
}

#[given(expr = "the checkout carries an untrusted overlay pinning only the default model {string}")]
fn given_checkout_untrusted_model_overlay(world: &mut QuectoWorld, model: String) {
    write_untrusted_overlay(
        world,
        serde_json::json!({"agents": {"defaults": {"model": model}}}),
    );
}

/// The checkout's overlay written straight to disk, never trusted.
fn write_untrusted_overlay(world: &QuectoWorld, document: serde_json::Value) {
    let overlay = checkout(world).join(".quecto").join("config.json");
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    std::fs::write(&overlay, serde_json::to_string_pretty(&document).unwrap()).unwrap();
    assert!(
        !base_path(world).join("config-overlay-trust.json").exists(),
        "the overlay must not be trusted"
    );
}

#[when(
    expr = "I spawn script-managed subagent {string} with default selection and no config argument and task {string}"
)]
fn when_spawn_default_no_config(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_spawn_json_without_config(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":true,"read_only":true}),
    );
}

#[when(
    expr = "I spawn script-managed subagent {string} with script {string} and no config argument and task {string}"
)]
fn when_spawn_named_no_config(
    world: &mut QuectoWorld,
    agent_id: String,
    script: String,
    task: String,
) {
    execute_spawn_json_without_config(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":{"mode":"new","container_config":script},"read_only":true}),
    );
}

/// The overlay the checkout carries, as the diagnostics name it.
fn overlay_path(world: &QuectoWorld) -> String {
    checkout(world)
        .join(".quecto")
        .join("config.json")
        .display()
        .to_string()
}

#[given(
    expr = "the checkout carries a trusted overlay labelling both {string} and {string} as default container configs"
)]
fn given_checkout_trusted_invalid_merge(world: &mut QuectoWorld, first: String, second: String) {
    // `config set --local` refuses a merge that is not valid, so the file is
    // written by hand and approved with `quecto config trust`: the trust
    // record is honest, the merge is not.
    let overlay = checkout(world).join(".quecto").join("config.json");
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    let entries = serde_json::json!({
        first: overlay_entry(world, "https://example.test/repo-1", true),
        second: overlay_entry(world, "https://example.test/repo-2", true),
    });
    std::fs::write(
        &overlay,
        serde_json::to_string_pretty(&serde_json::json!({"container_configs": entries})).unwrap(),
    )
    .unwrap();
    let output = cli::run_with_output(
        vec!["quecto".into(), "config".into(), "trust".into()],
        &world.cli_context,
    );
    assert_eq!(
        output.exit_code, 0,
        "quecto config trust failed:\nstdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
}

#[then(expr = "the spawn result should name container config {string}")]
fn then_spawn_names_container_config(world: &mut QuectoWorld, name: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let expected = format!(" container_config={name} ");
    assert!(
        !result.is_error && result.content.contains(&expected),
        "expected {expected:?} in: {}",
        result.content
    );
}

#[then("the spawn result should carry no configuration diagnostics")]
fn then_spawn_carries_no_diagnostics(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        !result.content.contains("Configuration diagnostics")
            && !result.content.contains("was not applied"),
        "{}",
        result.content
    );
}

#[then("the spawn result should carry the configuration diagnostic naming the checkout's overlay")]
fn then_spawn_carries_overlay_diagnostic(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let overlay = overlay_path(world);
    let diagnostic = format!("repo-local config overlay {overlay} is not trusted (sha256 ");
    assert!(
        result.content.contains(&diagnostic)
            && result.content.contains(
                "and was not applied; review it, then run `quecto config trust` from this directory"
            ),
        "expected the diagnostic {diagnostic:?}… in the tool result: {}",
        result.content
    );
    if result.is_error && !result.content.contains("container: true refused") {
        // A refusal quotes the diagnostic in its own sentence; every other
        // failed selection appends it to the error line.
        assert!(
            result
                .content
                .contains("; Configuration diagnostics: repo-local config overlay"),
            "a failed selection appends the diagnostics to the error line: {}",
            result.content
        );
    } else if !result.is_error {
        assert!(
            result
                .content
                .contains("\nConfiguration diagnostics:\nrepo-local config overlay"),
            "a successful spawn lists the diagnostics after the launch line: {}",
            result.content
        );
    }
}

#[then("the spawn result should fail naming the checkout's overlay merged over the global file")]
fn then_spawn_fails_naming_merge(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let expected = format!(
        "failed to load config {} merged over {}: invalid container_configs: ",
        overlay_path(world),
        world.config_path.clone().expect("config path")
    );
    assert!(
        result.is_error && result.content.contains(&expected),
        "expected {expected:?} in: {}",
        result.content
    );
}

#[then(expr = "the spawn result should fail with {string}")]
fn then_spawn_fails_with(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        result.is_error && result.content.contains(&expected),
        "expected an error containing {expected:?}, got: {}",
        result.content
    );
}

#[given(
    expr = "the checkout unbinds container config {string} through quecto config unset --local"
)]
fn given_checkout_unbinds(world: &mut QuectoWorld, name: String) {
    let output = cli::run_with_output(
        vec![
            "quecto".to_string(),
            "config".to_string(),
            "unset".to_string(),
            "--local".to_string(),
            format!("container_configs.{name}"),
        ],
        &world.cli_context,
    );
    assert_eq!(
        output.exit_code, 0,
        "quecto config unset --local failed:\nstdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
}

#[when(
    "a real quecto agent started in the checkout is driven by a fake provider to spawn container true"
)]
fn when_real_agent_spawns_container_true(world: &mut QuectoWorld) {
    // The real binary, the same one the fixture's create script launches
    // as the container child (QUECTO_CHILD_BINARY, set by the Background).
    let binary = PathBuf::from(std::env::var_os("QUECTO_CHILD_BINARY").expect("child binary"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    mount_spawning_provider(&server, seen.clone());
    let uri = server.uri();
    std::mem::forget(server);
    world_runtime_keep(rt);
    // The global file the real agent (and its container child) loads: the
    // fixture's container_configs, the provider pointed at this scenario's
    // fake endpoint.
    let config_path = PathBuf::from(world.config_path.clone().expect("config path"));
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["providers"]["openai"]["api_base"] = serde_json::json!(uri);
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    let checkout = checkout(world);
    let mut child = std::process::Command::new(&binary)
        .args(["agent", "-m", SPAWN_MARKER, "--no-session"])
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
}

#[then("the real agent should have exited successfully")]
fn then_real_agent_exited_ok(_world: &mut QuectoWorld) {
    let run = REAL_AGENT_RUN.lock().unwrap();
    let run = run.as_ref().expect("the real agent ran");
    assert_eq!(
        run.exit_code,
        Some(0),
        "stdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("REAL_AGENT_DONE"),
        "stdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
}

#[then(expr = "the tool result the fake provider received should name container config {string}")]
fn then_provider_saw_container_config(_world: &mut QuectoWorld, name: String) {
    let run = REAL_AGENT_RUN.lock().unwrap();
    let run = run.as_ref().expect("the real agent ran");
    let results = run.tool_results.lock().unwrap();
    let expected = format!(" container_config={name} ");
    assert!(
        results
            .iter()
            .any(|content| content.contains("is running (uuid=") && content.contains(&expected)),
        "expected a spawn result with {expected:?} among the tool results the model saw: {results:?}\nstderr: {}",
        run.stderr
    );
}
