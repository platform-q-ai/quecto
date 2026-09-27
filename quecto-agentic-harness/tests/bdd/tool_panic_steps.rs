//! #2192: a real `quecto` process (a `test-support` build, with the
//! `panic_probe` fault-injection tool switched on) against a mock provider
//! that asks for one `panic_probe` call, then answers.
use super::QuectoWorld;
use cucumber::{given, then, when};
use quecto::infrastructure::persistence::audit_log::AuditLog;
use serde_json::{Value, json};
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// One scenario's workspace, its mock provider and the agent's end.
#[derive(Debug)]
pub struct ToolPanicRun {
    root: tempfile::TempDir,
    uri: String,
    output: Option<Output>,
}

impl ToolPanicRun {
    fn dir(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// Every record the base directory's event logs hold.
    fn records(&self) -> Vec<Value> {
        let Ok(entries) = std::fs::read_dir(self.dir("base/audit")) else {
            return Vec::new();
        };
        entries
            .map(|entry| entry.unwrap().path())
            .flat_map(|path| {
                std::fs::read_to_string(path)
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .map(|line| serde_json::from_str(&line).unwrap())
            .collect()
    }

    fn output(&self) -> &Output {
        self.output.as_ref().expect("the agent ran")
    }
}

/// Keep an agent this feature expects to end fatally — and every sub-agent
/// it launches, which inherit both — from dumping core: the harness's one
/// test-support helper. Apply it last: an `env_clear` after it would drop
/// the switch.
pub use quecto::interface::panic_hook::test_support::without_core_dumps;

/// The provider: the first turn calls `panic_probe` where the prompt says
/// (`PROBE:tool` or `PROBE:outside`), every later turn answers.
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
                let messages = body["messages"].as_array().cloned().unwrap_or_default();
                let ran = messages.iter().any(|m| m["role"] == "tool");
                let place = messages
                    .iter()
                    .filter_map(|m| m["content"].as_str())
                    .find_map(|text| text.split("PROBE:").nth(1))
                    .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned());
                let (message, finish) = match (ran, place) {
                    (false, Some(place)) => (
                        json!({"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call-probe","type":"function","function":{"name":"panic_probe","arguments":json!({"where": place}).to_string()}}]}),
                        "tool_calls",
                    ),
                    _ => (json!({"role":"assistant","content":"RECOVERED"}), "stop"),
                };
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

impl QuectoWorld {
    /// The tool panic workspace's root directory.
    pub fn tool_panic_root(&self) -> PathBuf {
        self.tool_panic
            .as_ref()
            .expect("a tool panic workspace")
            .root
            .path()
            .to_path_buf()
    }
}

#[given("a tool panic workspace")]
fn workspace(world: &mut QuectoWorld) {
    // The agents here abort on purpose: they must run without core dumps.
    let limit = without_core_dumps(Command::new("sh").args(["-c", "ulimit -c; ulimit -Hc"]))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&limit.stdout), "0\n0\n");
    let root = tempfile::tempdir().unwrap();
    for dir in ["home", "base", "work"] {
        std::fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    let uri = start_provider();
    let config = json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": uri}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": root.path().join("work")}},
        "telemetry": {"event_log": {"enabled": true}}
    });
    std::fs::write(root.path().join("base/config.json"), config.to_string()).unwrap();
    world.tool_panic = Some(ToolPanicRun {
        root,
        uri,
        output: None,
    });
}

fn one_shot(world: &mut QuectoWorld, place: &str) {
    let run = world.tool_panic.as_mut().unwrap();
    assert!(!run.uri.is_empty());
    let mut command = Command::new(env!("CARGO_BIN_EXE_quecto"));
    command
        .current_dir(run.dir("work"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", run.dir("home"))
        .env("XDG_CONFIG_HOME", run.dir("home/.config"))
        .env("XDG_RUNTIME_DIR", run.root.path())
        .env("QUECTO_BASE_DIR", run.dir("base"))
        .env("QUECTO_TEST_PANIC_PROBE", "1")
        .env("RUST_LOG", "warn")
        .env("RUST_BACKTRACE", "0")
        .args(["agent", "-s", "probe", "-m"])
        .arg(format!("Run the probe. PROBE:{place}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Last, after `env_clear`, so the switch reaches the agent.
    let mut child = without_core_dumps(&mut command).spawn().unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("the one-shot agent did not finish within 60 s");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    run.output = Some(child.wait_with_output().unwrap());
}

#[when("a one-shot agent calls a tool that panics")]
fn calls_panicking_tool(world: &mut QuectoWorld) {
    one_shot(world, "tool");
}

#[when("a one-shot agent calls a tool whose side thread panics")]
fn calls_fatal_tool(world: &mut QuectoWorld) {
    one_shot(world, "outside");
}

#[then("the agent finished its answer after the failed call")]
fn finished(world: &mut QuectoWorld) {
    let output = world.tool_panic.as_ref().unwrap().output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{:?} {stderr}", output.status);
    assert!(stdout.contains("RECOVERED"), "{stdout} {stderr}");
}

#[then("the event log records the panic with the tool and its location")]
fn logged(world: &mut QuectoWorld) {
    let records = world.tool_panic.as_ref().unwrap().records();
    let result = records
        .iter()
        .find(|record| record["event"] == "tool_result" && record["tool"] == "panic_probe")
        .unwrap_or_else(|| panic!("no panic_probe result in {records:?}"));
    assert_eq!(result["is_error"], true, "{result}");
    let preview = result["content_preview"].as_str().unwrap();
    assert!(
        preview.starts_with("internal error in tool 'panic_probe': ")
            && preview.contains("is not a char boundary")
            && preview.contains("; the call stopped at the panic, and any partial effects"),
        "{preview}"
    );
    let error = records
        .iter()
        .find(|record| record["event"] == "error" && record["source"] == "tool_panic")
        .unwrap_or_else(|| panic!("no tool_panic error in {records:?}"));
    assert_eq!(error["tool"], "panic_probe", "{error}");
    assert!(
        error["location"]
            .as_str()
            .is_some_and(|at| at.contains("panic_probe.rs:")),
        "{error}"
    );
}

#[then("the agent was aborted")]
fn aborted(world: &mut QuectoWorld) {
    let output = world.tool_panic.as_ref().unwrap().output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Ended fatally, as an abort would, but by the test-support `_exit`
    // that leaves no core dump.
    assert_eq!(
        (output.status.code(), output.status.signal()),
        (
            Some(quecto::interface::panic_hook::test_support::FATAL_EXIT_CODE),
            None
        ),
        "{:?} {stderr}",
        output.status
    );
    assert!(
        stderr.contains("panic_probe: a panic outside any tool call"),
        "{stderr}"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("RECOVERED"),
        "nothing ran after the fatal panic"
    );
}

#[then("its crash record and event log say why it died")]
fn crash_recorded(world: &mut QuectoWorld) {
    let run = world.tool_panic.as_ref().unwrap();
    let name = AuditLog::crash_record_name("cli:probe");
    let text = std::fs::read_to_string(run.dir(&format!("base/audit/crash/{name}")))
        .expect("a crash record in the crash directory beside the event logs");
    let record: Value = serde_json::from_str(text.trim_end()).unwrap();
    assert_eq!(
        record["session"], "cli:probe",
        "it names its session exactly"
    );
    assert_eq!(
        record["message"],
        "panic_probe: a panic outside any tool call"
    );
    assert!(
        record["location"]
            .as_str()
            .is_some_and(|at| at.contains("panic_probe.rs:")),
        "{record}"
    );
    // Outside any call, it is attributed to none: the running one is named.
    assert_eq!(record.get("call"), None, "{record}");
    assert_eq!(record["running"], json!(["panic_probe"]), "{record}");
    let records = run.records();
    let error = records
        .iter()
        .find(|event| event["event"] == "error" && event["source"] == "panic")
        .unwrap_or_else(|| panic!("no fatal error event in {records:?}"));
    assert_eq!(
        error["message"],
        "panic_probe: a panic outside any tool call"
    );
    assert_eq!(
        error.get("tool"),
        Some(&Value::Null),
        "attributed to no call: {error}"
    );
}
