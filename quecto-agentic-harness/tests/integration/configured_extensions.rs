//! Configured extensions (#2446), with the real `quecto` binary and the
//! fixture's stand-in extension (`quecto-test-fixture extension`): the
//! agent launches what its config lists once its socket listens, owns it
//! (the agent's exit, even a SIGKILL, ends it), restarts it unless it exits
//! with 2 or 3, holds turns (never queries) until it has registered, and
//! every locally spawned child launches its parent's list, never its own
//! config's, under its own `{agent_id}`, whose tool the child's inherited
//! policy allows. `--no-extensions` launches none; a placeholder the config
//! does not know refuses the config; no other client may pose as one.
//!
//! Every wait is bounded so a regression fails instead of hanging.
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_secs(90);
const FIXTURE: &str = env!("CARGO_BIN_EXE_quecto-test-fixture");
const EXTENSION: &str = "stand-in";

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes existence of the given pid.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + BOUND;
    while !done() {
        assert!(Instant::now() < deadline, "{what} did not happen in time");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A provider that makes the agent spawn one child, with `child_config` as
/// its `config`, when the prompt says `SPAWN_ONE`; every other turn just
/// answers.
async fn provider(child_config: PathBuf) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let input: serde_json::Value = request.body_json().unwrap();
            let messages = input["messages"].as_array().cloned().unwrap_or_default();
            let spawner = messages.iter().any(|m| {
                m["role"] == "user"
                    && m["content"]
                        .as_str()
                        .is_some_and(|c| c.contains("SPAWN_ONE"))
            });
            let answered = messages.iter().any(|m| m["role"] == "tool");
            let delta = if spawner && !answered {
                let arguments = serde_json::json!({
                    "agent_id": "worker", "task": "wait", "config": child_config,
                })
                .to_string();
                serde_json::json!({"tool_calls": [{
                    "index": 0, "id": "call-worker", "type": "function",
                    "function": {"name": "spawn", "arguments": arguments}
                }]})
            } else {
                serde_json::json!({"content": "DONE"})
            };
            let finish = match delta.get("tool_calls") {
                Some(_) => "tool_calls",
                None => "stop",
            };
            let chunk = serde_json::json!({
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
            });
            wiremock::ResponseTemplate::new(200).set_body_raw(
                format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                "text/event-stream",
            )
        })
        .mount(&server)
        .await;
    server
}

struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    config: PathBuf,
    server: wiremock::MockServer,
    runtime: tokio::runtime::Runtime,
}

impl Fixture {
    /// The model requests the agents made, in order.
    fn requests(&self) -> Vec<serde_json::Value> {
        let received = self.runtime.block_on(self.server.received_requests());
        received
            .unwrap_or_default()
            .iter()
            .map(|request| request.body_json().unwrap())
            .collect()
    }
}

/// A base dir and a config whose configured extensions are `extensions`.
/// The child a `SPAWN_ONE` prompt spawns is handed `child.json`, a config
/// of its own that lists another extension, `rogue`: a child must never
/// launch it.
fn fixture(extensions: serde_json::Value) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = base.join("config.json");
    let child_config = base.join("child.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(provider(child_config.clone()));
    let mut json = serde_json::json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": server.uri()}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": workspace}},
        "extensions": extensions,
    });
    std::fs::write(&config, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    json["extensions"] = serde_json::json!([stand_in("rogue", &[])]);
    std::fs::write(&child_config, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    Fixture {
        _dir: dir,
        base,
        config,
        server,
        runtime,
    }
}

/// A stand-in's config entry named `name`, with `extra` arguments.
fn stand_in(name: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "extension",
        "--socket",
        "{socket}",
        "--agent-id",
        "{agent_id}",
        "--record",
        "{state_dir}/launches",
    ];
    args.extend(extra);
    serde_json::json!({"name": name, "command": FIXTURE, "args": args})
}

/// The one-extension config most tests use.
fn just(extra: &[&str]) -> serde_json::Value {
    serde_json::json!([stand_in(EXTENSION, extra)])
}

/// The state directory of extension `name`'s instance for `agent_id`.
fn state_dir_of(fixture: &Fixture, name: &str, agent_id: &str) -> PathBuf {
    fixture.base.join("extensions").join(name).join(agent_id)
}

fn state_dir(fixture: &Fixture, agent_id: &str) -> PathBuf {
    state_dir_of(fixture, EXTENSION, agent_id)
}

/// Each launch extension `name`'s instance for `agent_id` recorded:
/// `(pid, socket)`.
fn launches_of(fixture: &Fixture, name: &str, agent_id: &str) -> Vec<(u32, PathBuf)> {
    std::fs::read_to_string(state_dir_of(fixture, name, agent_id).join("launches"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.split(' ').collect();
            assert_eq!(parts[0], agent_id, "the instance got its own agent id");
            (parts[1].parse().unwrap(), PathBuf::from(parts[2]))
        })
        .collect()
}

fn launches(fixture: &Fixture, agent_id: &str) -> Vec<(u32, PathBuf)> {
    launches_of(fixture, EXTENSION, agent_id)
}

struct Harness {
    child: std::process::Child,
    socket: PathBuf,
}

impl Harness {
    fn start(fixture: &Fixture, flags: &[&str]) -> Self {
        let socket = fixture.base.join("agent.sock");
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_quecto"))
            .args(["agent", "--mode", "uds", "--socket"])
            .arg(&socket)
            .args(["-s", "top", "--config"])
            .arg(&fixture.config)
            .args(flags)
            .env("QUECTO_BASE_DIR", &fixture.base)
            .env("QUECTO_CHILD_BINARY", env!("CARGO_BIN_EXE_quecto"))
            .env("HOME", &fixture.base)
            .env("XDG_RUNTIME_DIR", &fixture.base)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .expect("spawn uds harness");
        let mut harness = Self { child, socket };
        wait_until("the harness binding its socket", || {
            if let Some(status) = harness.child.try_wait().unwrap() {
                panic!("harness exited before binding its socket: {status}");
            }
            harness.socket.exists()
        });
        harness
    }

    fn signal(&self, signal: &str) {
        let status = std::process::Command::new("kill")
            .args([signal, &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn wait_exit(&mut self) {
        wait_until("the harness exiting", || {
            self.child.try_wait().unwrap().is_some()
        });
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Client {
    stream: UnixStream,
}

impl Client {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("connect");
        stream.set_read_timeout(Some(BOUND)).unwrap();
        Self { stream }
    }

    fn send(&mut self, line: &str) {
        self.stream.write_all(line.as_bytes()).unwrap();
        self.stream.write_all(b"\n").unwrap();
    }

    fn read_until(&mut self, want: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.stream.read(&mut byte) {
                Ok(0) => panic!("the agent closed the connection while waiting"),
                Ok(_) => {}
                Err(error) => panic!("read failed: {error}"),
            }
            if byte[0] != b'\n' {
                buffer.push(byte[0]);
                continue;
            }
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&buffer)
                && want(&value)
            {
                return value;
            }
            buffer.clear();
        }
    }

    /// One answered query's `data`.
    fn query(&mut self, command: &str) -> serde_json::Value {
        self.send(&format!(r#"{{"type":"{command}","id":"q-{command}"}}"#));
        let id = format!("q-{command}");
        self.read_until(|v| v["type"] == "response" && v["id"] == id.as_str())["data"].clone()
    }

    /// The catalogue entry named `name`, once the agent holds it.
    fn tool(&mut self, name: &str) -> serde_json::Value {
        let deadline = Instant::now() + BOUND;
        loop {
            let tools = self.query("get_tool_catalogue")["tools"].clone();
            if let Some(tool) = tools
                .as_array()
                .into_iter()
                .flatten()
                .find(|tool| tool["name"] == name)
            {
                return tool.clone();
            }
            assert!(Instant::now() < deadline, "no tool {name}: {tools}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn startup_warnings(&mut self) -> String {
        self.query("get_state")["startupWarnings"].to_string()
    }
}

#[test]
fn a_configured_extension_is_launched_with_its_placeholders_and_ends_with_the_agent() {
    let fixture = fixture(just(&[]));
    let mut harness = Harness::start(&fixture, &["--persist"]);
    let mut client = Client::connect(&harness.socket);
    let tool = client.tool("echo_main");
    assert_eq!(tool["source"], "uds", "{tool}");
    let launched = launches(&fixture, "main");
    assert_eq!(launched.len(), 1, "one instance: {launched:?}");
    let (pid, socket) = launched[0].clone();
    assert_eq!(
        socket, harness.socket,
        "{{socket}} is the agent's own socket"
    );
    let log = state_dir(&fixture, "main").join("extension.log");
    assert!(log.exists(), "its output goes to {}", log.display());
    harness.signal("-TERM");
    harness.wait_exit();
    wait_until("the extension ending with its agent", || !alive(pid));
    assert!(
        state_dir(&fixture, "main").exists(),
        "a top-level agent's state is kept for its next run"
    );
}

/// The stand-in outlives its socket's close (`--ignore-close`): only the
/// parent-death signal the agent armed ends it when the agent is killed.
#[test]
fn killing_the_agent_kills_its_extension() {
    let fixture = fixture(just(&["--ignore-close"]));
    let harness = Harness::start(&fixture, &["--persist"]);
    Client::connect(&harness.socket).tool("echo_main");
    let (pid, _) = launches(&fixture, "main")[0].clone();
    harness.signal("-KILL");
    wait_until("the extension ending after its agent's SIGKILL", || {
        !alive(pid)
    });
}

/// #2446 review H1: the child is handed `child.json`, which lists `rogue`;
/// it launches its parent's list, never the file's.
#[test]
fn a_spawned_child_launches_its_parents_extensions_whose_tool_its_policy_allows() {
    let fixture = fixture(just(&[]));
    let harness = Harness::start(&fixture, &["--persist"]);
    let mut parent = Client::connect(&harness.socket);
    parent.tool("echo_main");
    parent.send(r#"{"type":"prompt","message":"SPAWN_ONE"}"#);
    let mut child = None;
    wait_until("the child launching its own instance", || {
        child = std::fs::read_dir(fixture.base.join("extensions").join(EXTENSION))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().into_string().unwrap())
            .find(|agent_id| agent_id != "main" && !launches(&fixture, agent_id).is_empty());
        child.is_some()
    });
    let child_id = child.unwrap();
    let (child_pid, child_socket) = launches(&fixture, &child_id)[0].clone();
    assert_ne!(
        child_socket, harness.socket,
        "the child's instance serves the child"
    );
    let tool = Client::connect(&child_socket).tool(&format!("echo_{child_id}"));
    assert_eq!(
        tool["effectiveChildEnabled"], true,
        "the child's inherited policy allows its own instance's tool: {tool}"
    );
    assert!(
        !fixture.base.join("extensions").join("rogue").exists(),
        "the child's own --config names no command it runs"
    );
    let parent_tools = parent.query("get_tool_catalogue")["tools"].to_string();
    assert!(
        !parent_tools.contains(&format!("echo_{child_id}")),
        "the parent's tools stay the parent's"
    );
    assert_eq!(launches(&fixture, "main").len(), 1, "one parent instance");
    drop(harness);
    wait_until("the child's instance ending with the tree", || {
        !alive(child_pid)
    });
    wait_until("the child's state directory going with it", || {
        !state_dir(&fixture, &child_id).exists()
    });
}

#[test]
fn an_extension_that_exits_is_restarted() {
    let fixture = fixture(just(&["--exit-code", "1"]));
    let harness = Harness::start(&fixture, &["--persist"]);
    wait_until("a restart", || launches(&fixture, "main").len() >= 2);
    let warnings = Client::connect(&harness.socket).startup_warnings();
    assert!(warnings.contains("restart"), "{warnings}");
}

#[test]
fn exit_codes_2_and_3_are_never_restarted() {
    for (code, why) in [("2", "command-line error"), ("3", "tools were refused")] {
        let fixture = fixture(just(&["--exit-code", code]));
        let harness = Harness::start(&fixture, &["--persist"]);
        let mut client = Client::connect(&harness.socket);
        let mut warnings = String::new();
        wait_until("the stop warning", || {
            warnings = client.startup_warnings();
            warnings.contains("not restarted")
        });
        assert!(warnings.contains(why), "{warnings}");
        std::thread::sleep(Duration::from_millis(2500));
        assert_eq!(launches(&fixture, "main").len(), 1, "exit {code} is final");
    }
}

/// #2446 review M3(b): a prompt sent before a slow extension registered
/// waits for it, so the model's first request already offers its tool.
#[test]
fn a_first_prompt_waits_for_a_slow_extensions_tools() {
    let fixture = fixture(just(&["--register-delay", "3000"]));
    let harness = Harness::start(&fixture, &["--persist"]);
    let mut client = Client::connect(&harness.socket);
    client.send(r#"{"type":"prompt","id":"p1","message":"hello"}"#);
    client.read_until(|v| v["type"] == "agent_end");
    let first = fixture
        .requests()
        .into_iter()
        .next()
        .expect("a model request");
    let tools = first["tools"].to_string();
    assert!(
        tools.contains("echo_main"),
        "the first request offers the extension's tool: {tools}"
    );
}

/// #2446 review L1: only turns wait for the extensions; a query is answered
/// at once.
#[test]
fn a_query_is_answered_during_the_start_up_wait() {
    let fixture = fixture(just(&["--register-delay", "8000"]));
    let harness = Harness::start(&fixture, &["--persist"]);
    let mut client = Client::connect(&harness.socket);
    let asked = Instant::now();
    client.query("get_state");
    assert!(
        asked.elapsed() < Duration::from_secs(4),
        "get_state took {:?}",
        asked.elapsed()
    );
    assert!(launches(&fixture, "main").len() == 1);
}

/// #2446 review M1: an extension that crashes and restarts while another
/// keeps the start-up wait open gets its tools back: its old connection's
/// disconnect is not held behind the wait.
#[test]
fn an_extension_restarting_during_the_wait_registers_again() {
    let fixture = fixture(serde_json::json!([
        stand_in("slow", &["--tool", "slow_tool", "--register-delay", "6000"]),
        stand_in(
            EXTENSION,
            &["--exit-after-register-once", "{state_dir}/crashed"]
        ),
    ]));
    let harness = Harness::start(&fixture, &["--persist"]);
    wait_until("the restart", || launches(&fixture, "main").len() == 2);
    let mut client = Client::connect(&harness.socket);
    client.tool("echo_main");
    let log = std::fs::read_to_string(state_dir(&fixture, "main").join("extension.log")).unwrap();
    let answers: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("register_tools:"))
        .collect();
    assert_eq!(answers.len(), 2, "{log}");
    assert!(answers[1].contains(r#""success":true"#), "{log}");
    client.tool("slow_tool");
}

/// #2446 review M3(c), L3: a client that is no configured extension cannot
/// register under an extension's stable id, nor a name that is not a tool
/// name.
#[test]
fn no_other_client_poses_as_a_configured_extension() {
    let fixture = fixture(serde_json::json!([]));
    let harness = Harness::start(&fixture, &["--persist"]);
    let mut client = Client::connect(&harness.socket);
    let posing = serde_json::json!({
        "type": "register_tools", "id": "pose",
        "tools": [{"name": "fake", "description": "d",
            "stableId": "tool.v1:uds:22:uds:extension:stand-in:fake"}],
    });
    client.send(&posing.to_string());
    let answer = client.read_until(|v| v["type"] == "response" && v["id"] == "pose");
    assert_eq!(answer["success"], false, "{answer}");
    let colon = serde_json::json!({
        "type": "register_tools", "id": "colon",
        "tools": [{"name": "uds:extension:stand-in", "description": "d"}],
    });
    client.send(&colon.to_string());
    let answer = client.read_until(|v| v["type"] == "response" && v["id"] == "colon");
    assert_eq!(answer["success"], false, "{answer}");
    assert!(
        answer["error"].as_str().unwrap().contains("tool name"),
        "{answer}"
    );
}

#[test]
fn no_extensions_launches_none() {
    let fixture = fixture(just(&[]));
    let harness = Harness::start(&fixture, &["--persist", "--no-extensions"]);
    let mut client = Client::connect(&harness.socket);
    client.query("get_state");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(launches(&fixture, "main").is_empty());
    let tools = client.query("get_tool_catalogue")["tools"].to_string();
    assert!(!tools.contains("echo_main"), "{tools}");
}

/// A config the agent refuses at start-up: it exits non-zero, naming why.
fn refused(extension: serde_json::Value) -> String {
    let fixture = fixture(serde_json::json!([extension]));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_quecto"))
        .args(["agent", "--mode", "uds", "--socket"])
        .arg(fixture.base.join("agent.sock"))
        .arg("--config")
        .arg(&fixture.config)
        .env("QUECTO_BASE_DIR", &fixture.base)
        .env("HOME", &fixture.base)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + BOUND;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the agent did not refuse its config");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!status.success());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    stderr
}

#[test]
fn an_unknown_placeholder_or_a_relative_command_is_a_config_error() {
    let mut unknown = stand_in(EXTENSION, &[]);
    unknown["args"][2] = serde_json::json!("{sockets}");
    let stderr = refused(unknown);
    assert!(stderr.contains("{sockets}"), "{stderr}");
    let stderr = refused(serde_json::json!({
        "name": EXTENSION, "command": "quecto-test-fixture", "env": {"A": "{nope}"}
    }));
    assert!(stderr.contains("absolute"), "{stderr}");
}

/// The agent's own extension is not a client: an agent that ends with its
/// last client still does, and ends its extension with it.
#[test]
fn an_extension_does_not_keep_its_agent_past_the_last_client() {
    let fixture = fixture(just(&[]));
    let mut harness = Harness::start(&fixture, &[]);
    let mut client = Client::connect(&harness.socket);
    client.tool("echo_main");
    let (pid, _) = launches(&fixture, "main")[0].clone();
    drop(client);
    harness.wait_exit();
    wait_until("the extension ending with its agent", || !alive(pid));
}
