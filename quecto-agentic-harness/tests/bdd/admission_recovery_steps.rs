//! #2024 S3 real-process recovery: a real broker, real `quecto agent --mode
//! uds` processes (a root bound to the global config and a child launched
//! with a parent-registered admission context and an admission-null config)
//! and a mock provider. Admission is asserted from the outside — the broker's
//! `status` (`live_scopes`, `epoch`) and each agent's `get_state.admission`
//! (`counters.completed`, `authorityStatus`, `epoch`) — so an inherited
//! authority cannot be confused with a bypass. `reset` and a broker kill /
//! restart are exercised for real: the root's next prompt is admitted without
//! an agent restart; the child's fails closed exactly once, naming the fix.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use quecto::infrastructure::admission::{
    Negotiation, ProcessAdmission, process, write_admission_context,
};

use crate::QuectoWorld;

const LIMIT: Duration = Duration::from_secs(30);

pub struct UdsAgent {
    child: Child,
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    stderr_path: PathBuf,
    last_error: Option<String>,
}

impl Drop for UdsAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct AdmissionRecoveryState {
    temp: Option<tempfile::TempDir>,
    broker: Option<Child>,
    mock_uri: Option<String>,
    runtime: Option<tokio::runtime::Runtime>,
    /// The test-side parent binding that mints the child's capability.
    parent: Option<ProcessAdmission>,
    agents: BTreeMap<String, UdsAgent>,
    status_out: String,
}

impl std::fmt::Debug for AdmissionRecoveryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdmissionRecoveryState")
    }
}

impl Drop for AdmissionRecoveryState {
    fn drop(&mut self) {
        self.agents.clear();
        if let Some(mut broker) = self.broker.take() {
            let _ = broker.kill();
            let _ = broker.wait();
        }
    }
}

fn state(world: &mut QuectoWorld) -> &mut AdmissionRecoveryState {
    &mut world.admission_recovery
}

fn base(world: &mut QuectoWorld) -> PathBuf {
    state(world).temp.as_ref().unwrap().path().to_path_buf()
}

fn authority_dir(base: &Path) -> PathBuf {
    base.join("authority")
}

fn qcmd(base: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_quecto"));
    command
        .args(args)
        .env("QUECTO_BASE_DIR", base)
        .env("HOME", base)
        .current_dir(base.join("workspace"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn provider_section(mock: &str, workspace: &Path) -> String {
    format!(
        r#""providers":{{"openai":{{"api_key":"sk-test","api_base":"{mock}"}}}},
"agents":{{"defaults":{{"model":"openai-api/gpt-4o-mini","workspace":{workspace:?}}}}}"#,
        workspace = workspace.to_string_lossy(),
    )
}

fn global_config(base: &Path, mock: &str) -> String {
    format!(
        r#"{{{providers},
"admission":{{"directory":{dir:?},"groups":{{"g":{{"capacity":4,"reserve":0,"min_interval_ms":1,"queue_capacity":16,"queue_timeout_ms":5000,"attempt_timeout_ms":30000,"fallback_base_ms":50,"max_cooldown_ms":500}}}},"aliases":{{"acct":"g"}},"bindings":{{"openai-api":"acct"}}}}}}"#,
        providers = provider_section(mock, &base.join("workspace")),
        dir = authority_dir(base).to_string_lossy(),
    )
}

fn start_mock() -> (String, tokio::runtime::Runtime) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let uri = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(|request: &wiremock::Request| {
                let streaming = request
                    .body_json::<serde_json::Value>()
                    .ok()
                    .and_then(|body| body.get("stream").and_then(serde_json::Value::as_bool))
                    .unwrap_or(false);
                if streaming {
                    let chunk = serde_json::json!({
                        "choices": [{"index": 0, "delta": {"content": "DONE"}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
                    });
                    wiremock::ResponseTemplate::new(200).set_body_raw(
                        format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                        "text/event-stream",
                    )
                } else {
                    wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "chatcmpl-test",
                        "object": "chat.completion",
                        "choices": [{"index": 0, "message": {"role": "assistant", "content": "DONE"}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
                    }))
                }
            })
            .mount(&server)
            .await;
        let uri = server.uri();
        std::mem::forget(server);
        uri
    });
    (uri, rt)
}

fn start_broker(base: &Path) -> Child {
    let socket = authority_dir(base).join("client").join("admission.sock");
    let _ = std::fs::remove_file(&socket);
    let child = qcmd(base, &["admission-broker", "run"])
        .spawn()
        .expect("spawn broker");
    let deadline = Instant::now() + LIMIT;
    while UnixStream::connect(&socket).is_err() {
        assert!(Instant::now() < deadline, "broker never accepted");
        std::thread::sleep(Duration::from_millis(20));
    }
    child
}

#[given("a real admission broker serving a mock provider")]
fn given_broker(world: &mut QuectoWorld) {
    let temp = tempfile::tempdir().unwrap();
    let base_path = temp.path().to_path_buf();
    std::fs::create_dir_all(base_path.join("workspace")).unwrap();
    state(world).temp = Some(temp);
    let (uri, rt) = start_mock();
    state(world).mock_uri = Some(uri.clone());
    state(world).runtime = Some(rt);
    std::fs::write(
        base_path.join("config.json"),
        global_config(&base_path, &uri),
    )
    .unwrap();
    let broker = start_broker(&base_path);
    state(world).broker = Some(broker);
}

fn start_agent(world: &mut QuectoWorld, name: &str, extra: &[&str]) {
    let base_path = base(world);
    let socket = base_path.join(format!("{name}.sock"));
    let stderr_path = base_path.join(format!("{name}.stderr"));
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let mut args = vec!["agent", "--mode", "uds", "--no-session", "--socket"];
    args.push(socket.to_str().unwrap());
    args.extend_from_slice(extra);
    let mut child = qcmd(&base_path, &args)
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .expect("spawn agent");
    let deadline = Instant::now() + LIMIT;
    let stream = loop {
        if let Ok(stream) = UnixStream::connect(&socket) {
            break stream;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "agent {name} exited before its socket was ready: {status}\n{}",
                std::fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        assert!(Instant::now() < deadline, "agent {name} socket timed out");
        std::thread::sleep(Duration::from_millis(20));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    state(world).agents.insert(
        name.to_string(),
        UdsAgent {
            child,
            stream,
            reader,
            stderr_path,
            last_error: None,
        },
    );
}

#[given(expr = "a real root UDS agent {string} bound to the broker")]
fn given_root_agent(world: &mut QuectoWorld, name: String) {
    start_agent(world, &name, &[]);
}

#[given(
    expr = "a real child UDS agent {string} launched with a parent-registered admission context and an admission-null config"
)]
fn given_child_agent(world: &mut QuectoWorld, name: String) {
    let base_path = base(world);
    // A real parent binding to the running broker mints the child's
    // capability, exactly as the spawn tool does before launch.
    let parent = process::negotiate(Negotiation::Root {
        directory: authority_dir(&base_path),
    })
    .expect("parent negotiates with the broker");
    let rt = state(world).runtime.as_ref().unwrap().handle().clone();
    let credential = rt
        .block_on(parent.register_child())
        .expect("parent registers the child");
    let sidecar = base_path.join(format!("{name}-admission.ctx"));
    write_admission_context(&sidecar, &parent.endpoint(), &credential).expect("write sidecar");
    state(world).parent = Some(parent);
    // The child's own config disables admission and still points at the mock.
    let mock = state(world).mock_uri.clone().unwrap();
    let child_config = base_path.join(format!("{name}-config.json"));
    std::fs::write(
        &child_config,
        format!(
            "{{{},\n\"admission\":null}}",
            provider_section(&mock, &base_path.join("workspace"))
        ),
    )
    .unwrap();
    let config = child_config.to_str().unwrap().to_owned();
    let context = sidecar.to_str().unwrap().to_owned();
    start_agent(
        world,
        &name,
        &["--config", &config, "--admission-context", &context],
    );
}

fn agent<'a>(world: &'a mut QuectoWorld, name: &str) -> &'a mut UdsAgent {
    state(world)
        .agents
        .get_mut(name)
        .unwrap_or_else(|| panic!("no agent {name}"))
}

fn read_line(agent: &mut UdsAgent, deadline: Instant) -> Option<serde_json::Value> {
    loop {
        let mut line = String::new();
        match agent.reader.read_line(&mut line) {
            Ok(0) => panic!(
                "the agent closed its socket\n{}",
                std::fs::read_to_string(&agent.stderr_path).unwrap_or_default()
            ),
            Ok(_) => {
                return Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}")));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("socket read failed: {error}"),
        }
        if Instant::now() > deadline {
            return None;
        }
    }
}

/// Send `command` and return the reply correlated by its id.
fn exchange(agent: &mut UdsAgent, mut command: serde_json::Value) -> serde_json::Value {
    let id = format!(
        "rc-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    command["id"] = serde_json::Value::String(id.clone());
    writeln!(agent.stream, "{command}").unwrap();
    let deadline = Instant::now() + LIMIT;
    loop {
        let value = read_line(agent, deadline).unwrap_or_else(|| panic!("no reply to {command}"));
        if value["id"] == id {
            return value;
        }
    }
}

fn get_state(agent: &mut UdsAgent) -> serde_json::Value {
    let reply = exchange(agent, serde_json::json!({"type": "get_state"}));
    assert_eq!(reply["success"], true, "{reply}");
    reply["data"].clone()
}

/// Run one prompt to its end. The agent answers the prompt's own id only on
/// success (after the turn), and emits an `agent_error` response on failure.
fn run_prompt(agent: &mut UdsAgent) {
    agent.last_error = None;
    let id = format!(
        "rc-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let command = serde_json::json!({"type": "prompt", "id": id, "message": "hi"});
    writeln!(agent.stream, "{command}").unwrap();
    let deadline = Instant::now() + LIMIT;
    loop {
        let event = read_line(agent, deadline).unwrap_or_else(|| {
            panic!(
                "the prompt never ended\n{}",
                std::fs::read_to_string(&agent.stderr_path).unwrap_or_default()
            )
        });
        if event["type"] != "response" {
            continue;
        }
        if event["id"] == id {
            assert_eq!(event["success"], true, "{event}");
            return;
        }
        if event["command"] == "agent_error" {
            agent.last_error = Some(event["error"].as_str().unwrap_or("").to_owned());
            return;
        }
    }
}

#[when(expr = "{string} runs a prompt")]
fn when_runs_prompt(world: &mut QuectoWorld, name: String) {
    run_prompt(agent(world, &name));
}

#[then(
    expr = "{string} completed {int} admitted attempt(s) and reports the authority {string} in epoch {int}"
)]
fn then_completed(
    world: &mut QuectoWorld,
    name: String,
    completed: u64,
    status: String,
    epoch: u64,
) {
    let agent = agent(world, &name);
    assert!(
        agent.last_error.is_none(),
        "{name}'s prompt failed: {:?}",
        agent.last_error
    );
    let admission = get_state(agent)["admission"].clone();
    assert_eq!(
        admission["counters"]["completed"], completed,
        "{name} admission: {admission}"
    );
    assert_eq!(admission["authorityStatus"], status, "{admission}");
    assert_eq!(admission["connected"], status == "connected", "{admission}");
    assert_eq!(admission["epoch"], epoch, "{admission}");
}

#[then(expr = "{string} reports the authority {string}")]
fn then_reports_status(world: &mut QuectoWorld, name: String, status: String) {
    // The link notices a loss/revocation on its own; give it a moment.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let admission = get_state(agent(world, &name))["admission"].clone();
        if admission["authorityStatus"] == status {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{name} never reported {status}: {admission}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[then(expr = "{string}'s prompt failed closed once, telling its parent to respawn it")]
fn then_child_failed_closed(world: &mut QuectoWorld, name: String) {
    let agent = agent(world, &name);
    let error = agent
        .last_error
        .clone()
        .unwrap_or_else(|| panic!("{name}'s prompt must fail closed, not run"));
    assert!(error.contains("respawn"), "{error}");
    assert!(error.contains("reset"), "{error}");
    let admission = get_state(agent)["admission"].clone();
    assert_eq!(admission["authorityStatus"], "unavailable", "{admission}");
    assert_eq!(admission["counters"]["refused"], 1, "{admission}");
    assert_eq!(admission["counters"]["completed"], 1, "{admission}");
}

fn broker_status(world: &mut QuectoWorld) -> serde_json::Value {
    let base_path = base(world);
    let output = qcmd(&base_path, &["admission-broker", "status"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    state(world).status_out = out.clone();
    serde_json::from_str(out.trim()).unwrap()
}

#[then(expr = "the broker reports epoch {int} with at least {int} live scopes")]
fn then_broker_at_least(world: &mut QuectoWorld, epoch: u64, scopes: u64) {
    let status = broker_status(world);
    assert_eq!(status["epoch"], epoch, "{status}");
    assert!(
        status["live_scopes"].as_u64().unwrap() >= scopes,
        "{status}"
    );
}

#[then(expr = "the broker reports epoch {int} with exactly {int} live scope(s)")]
fn then_broker_exactly(world: &mut QuectoWorld, epoch: u64, scopes: u64) {
    let status = broker_status(world);
    assert_eq!(status["epoch"], epoch, "{status}");
    assert_eq!(status["live_scopes"], scopes, "{status}");
}

#[when("the operator resets the broker")]
fn when_reset(world: &mut QuectoWorld) {
    let base_path = base(world);
    let output = qcmd(&base_path, &["admission-broker", "reset"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[when("the broker is killed and started again")]
fn when_broker_restarted(world: &mut QuectoWorld) {
    let mut broker = state(world).broker.take().unwrap();
    broker.kill().unwrap();
    broker.wait().unwrap();
    let base_path = base(world);
    let broker = start_broker(&base_path);
    state(world).broker = Some(broker);
}
