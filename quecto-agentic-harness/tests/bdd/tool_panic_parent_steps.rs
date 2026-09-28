//! #2192: a real parent `quecto` harness (UDS, `test-support` build) spawns
//! a local sub-agent through its production spawn tool. The mock provider
//! makes the child call the `panic_probe` tool twice — first a panic inside
//! the call (contained), then one outside any call (fatal) — and makes the
//! parent, told of the end, inspect the ended child with `agent_cmd`.
use super::QuectoWorld;
use cucumber::{then, when};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_secs(120);

/// What the parent saw: the injected end note and each `agent_cmd` result.
#[derive(Debug, Default)]
pub struct ParentRun {
    pub note: String,
    pub state: String,
    pub messages: String,
}

/// The mock provider's turn for one request.
///
/// The child's first run calls the probe inside the call (contained) and
/// answers; that turn is persisted. Told the child ended a turn, the parent
/// prompts it again; the child's second run panics outside any call and
/// aborts. Told it ended unexpectedly, the parent inspects it.
fn respond(body: &Value) -> (Value, &'static str) {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let texts: Vec<String> = messages
        .iter()
        .filter(|m| m["role"] == "user")
        .filter_map(|m| m["content"].as_str().map(str::to_owned))
        .collect();
    let tool_results = messages.iter().filter(|m| m["role"] == "tool").count();
    let last_user = texts.last().cloned().unwrap_or_default();
    let call = |id: &str, name: &str, arguments: Value| {
        (
            json!({"role":"assistant","content":null,"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}),
            "tool_calls",
        )
    };
    let answer = |text: &str| (json!({"role":"assistant","content":text}), "stop");
    let child = texts.iter().any(|text| text.contains("CRASH_CHILD"));
    let told = texts.iter().any(|text| text.contains("ended unexpectedly"));
    let turn_ended = texts.iter().any(|text| text.contains("ended a turn"));
    let spawner = texts.iter().any(|text| text.contains("SPAWN_CRASHER"));
    match (child, spawner, told, turn_ended, tool_results) {
        (true, ..) if last_user.contains("CRASH_NOW") => {
            call("c2", "panic_probe", json!({"where": "outside"}))
        }
        (true, _, _, _, 0) => call("c1", "panic_probe", json!({"where": "tool"})),
        (true, ..) => answer("FIRST_RUN_DONE"),
        (false, true, false, false, 0) => call(
            "p1",
            "spawn",
            json!({"agent_id": "crasher", "task": "CRASH_CHILD: run the probes", "read_only": true}),
        ),
        (false, true, false, true, 1) => call(
            "p2",
            "agent_cmd",
            json!({"command": "prompt", "agent_id": "crasher", "message": "CRASH_NOW"}),
        ),
        (false, true, true, _, 2) => call(
            "p3",
            "agent_cmd",
            json!({"command": "get_state", "agent_id": "crasher"}),
        ),
        (false, true, true, _, 3) => call(
            "p4",
            "agent_cmd",
            // An explicit page: the transcript itself. A default read is
            // the unread report (#2226), the child's last answer.
            json!({"command": "get_messages", "agent_id": "crasher", "count": 40}),
        ),
        _ => answer("DONE"),
    }
}

fn start_provider() -> String {
    let runtime: &'static tokio::runtime::Runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap(),
    ));
    let server: &'static wiremock::MockServer =
        Box::leak(Box::new(runtime.block_on(wiremock::MockServer::start())));
    runtime.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(|request: &wiremock::Request| {
                let body: Value = request.body_json().unwrap();
                let (message, finish) = respond(&body);
                let usage = json!({"prompt_tokens":50,"completion_tokens":30,"total_tokens":80});
                match body["stream"] == true {
                    true => {
                        let chunk = json!({"choices":[{"index":0,"delta":message,"finish_reason":finish}],"usage":usage});
                        wiremock::ResponseTemplate::new(200).set_body_raw(
                            format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                            "text/event-stream",
                        )
                    }
                    false => wiremock::ResponseTemplate::new(200).set_body_json(json!({"id":"x","object":"chat.completion","choices":[{"index":0,"message":message,"finish_reason":finish}],"usage":usage})),
                }
            })
            .mount(server),
    );
    server.uri()
}

/// The text of a `tool_execution_end` event's result.
fn result_text(event: &Value) -> String {
    event["result"]["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

#[when("a parent agent's sub-agent survives one tool panic and then crashes")]
fn parent_run(world: &mut QuectoWorld) {
    let root = world.tool_panic_root();
    let uri = start_provider();
    let config = root.join("base/config.json");
    std::fs::write(
        &config,
        json!({
            "providers": {"openai": {"api_key": "sk-test", "api_base": uri}},
            "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": root.join("work")}},
            "telemetry": {"event_log": {"enabled": true}}
        })
        .to_string(),
    )
    .unwrap();
    let socket = root.join("parent.sock");
    let mut command = Command::new(env!("CARGO_BIN_EXE_quecto"));
    command
        .current_dir(root.join("work"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("home/.config"))
        .env("XDG_RUNTIME_DIR", &root)
        .env("QUECTO_BASE_DIR", root.join("base"))
        .env("QUECTO_CHILD_BINARY", env!("CARGO_BIN_EXE_quecto"))
        .env("QUECTO_TEST_PANIC_PROBE", "1")
        .env("RUST_LOG", "warn")
        .env("RUST_BACKTRACE", "0")
        .args([
            "agent",
            "--mode",
            "uds",
            "-s",
            "parent",
            "--persist",
            "--socket",
        ])
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Last, after `env_clear`: its sub-agent ends fatally on purpose and
    // inherits the switch, so it exits 134 instead of dumping core.
    let mut child = super::tool_panic_steps::without_core_dumps(&mut command)
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !socket.exists() {
        assert!(child.try_wait().unwrap().is_none(), "the parent exited");
        assert!(started.elapsed() < BOUND, "no socket");
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut stream = UnixStream::connect(&socket).unwrap();
    stream.set_read_timeout(Some(BOUND)).unwrap();
    writeln!(
        stream,
        "{}",
        json!({"type":"prompt","message":"SPAWN_CRASHER","id":"p1"})
    )
    .unwrap();
    let mut run = ParentRun::default();
    let mut seen = Vec::new();
    for line in BufReader::new(stream.try_clone().unwrap()).lines() {
        let Ok(line) = line else { break };
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if line.contains("ended unexpectedly") && run.note.is_empty() {
            run.note = line.clone();
        }
        if event["type"] == "tool_execution_end" && event["toolName"] == "agent_cmd" {
            let text = result_text(&event);
            if text.contains("\"command\":\"get_state\"") {
                run.state = text;
            } else if text.contains("\"command\":\"get_messages\"")
                || text.contains("get_messages") && !run.state.is_empty()
            {
                run.messages = text;
            }
        }
        seen.push(match event["type"] == "tool_execution_end" {
            true => line.clone(),
            false => event["type"].as_str().unwrap_or("?").to_owned(),
        });
        if !run.messages.is_empty() {
            break;
        }
        assert!(
            started.elapsed() < BOUND,
            "no inspection within the bound: {seen:?}"
        );
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        !run.messages.is_empty(),
        "the parent never inspected: {seen:?}"
    );
    world.tool_panic_parent = Some(run);
}

fn run(world: &QuectoWorld) -> &ParentRun {
    world.tool_panic_parent.as_ref().expect("the parent ran")
}

#[then(
    "the parent is told the sub-agent ended unexpectedly while its tool call ran, with the panic"
)]
fn told(world: &mut QuectoWorld) {
    let line = &run(world).note;
    let note: Value = serde_json::from_str(line).unwrap_or_else(|_| panic!("a JSON event: {line}"));
    let message = note["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(
            "ended unexpectedly (exit code 134) while tool call 'panic_probe' was running \
             (child-supplied): panicked; the child's recorded panic message: \"panic_probe: a panic outside any \
             tool call\" at "
        ),
        "{message}"
    );
    assert!(
        message.contains("(child-supplied, unverified)"),
        "{message}"
    );
}

#[then("the parent's get_state names the end reason")]
fn state(world: &mut QuectoWorld) {
    let state: Value = serde_json::from_str(&run(world).state)
        .unwrap_or_else(|_| panic!("get_state answered JSON: {}", run(world).state));
    let data = &state["data"];
    assert_eq!(data["status"], "ended", "{state}");
    // The test-support fatal end: `_exit(134)`, never a core-dumping abort.
    assert_eq!(data["exitCode"], 134, "{state}");
    let reason = data["endReason"].as_str().unwrap();
    assert!(
        reason.starts_with(
            "subagent 'crasher' ended unexpectedly (exit code 134) while tool call \
             'panic_probe' was running (child-supplied)"
        ),
        "{reason}"
    );
}

#[then("the parent's get_messages returns the sub-agent's transcript up to the crash")]
fn messages(world: &mut QuectoWorld) {
    let text = &run(world).messages;
    let answer: Value =
        serde_json::from_str(text).unwrap_or_else(|_| panic!("get_messages answered JSON: {text}"));
    let data = &answer["data"];
    assert_eq!(data["source"], "persisted transcript", "{answer}");
    let contents: Vec<&str> = data["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect();
    assert!(
        contents.iter().any(|c| c.contains("CRASH_CHILD")),
        "the task: {contents:?}"
    );
    assert!(
        contents
            .iter()
            .any(|c| c.starts_with("internal error in tool 'panic_probe': ")),
        "the contained call's result survived: {contents:?}"
    );
}
