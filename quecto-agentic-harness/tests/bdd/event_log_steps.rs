//! #2150: which real agents keep the event log. A real `quecto` process,
//! isolated HOME and base directory, a mock provider that asks for one
//! `grep` call, then answers.
use super::QuectoWorld;
use cucumber::{given, then, when};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// One scenario's workspace: its directories and its mock provider.
#[derive(Debug)]
pub struct EventLogRun {
    root: tempfile::TempDir,
}

impl EventLogRun {
    fn dir(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn command(&self) -> Command {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .map(|path| match path.is_absolute() {
                true => path,
                false => repository.join(path),
            })
            .unwrap_or_else(|| repository.join("target"));
        let mut command = Command::new(target.join("debug/quecto"));
        command
            .current_dir(self.dir("work"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir("home"))
            .env("XDG_CONFIG_HOME", self.dir("home/.config"))
            .env("XDG_DATA_HOME", self.dir("home/.local/share"))
            .env("XDG_STATE_HOME", self.dir("home/.local/state"))
            .env("XDG_RUNTIME_DIR", self.root.path())
            .env("QUECTO_BASE_DIR", self.dir("base"))
            .env("RUST_LOG", "warn")
            .stdin(Stdio::null());
        command
    }

    /// Every record the base directory's audit logs hold.
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
}

#[given(regex = r"^an event log workspace with the event log switched (on|off)$")]
fn workspace(world: &mut QuectoWorld, switch: String) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["home", "base", "work"] {
        std::fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    std::fs::write(root.path().join("work/a.txt"), "hello world\n").unwrap();
    // The mock provider lives for the rest of the process: a runtime dropped
    // inside the step executor aborts it (as the web search steps do).
    let runtime: &'static tokio::runtime::Runtime =
        Box::leak(Box::new(tokio::runtime::Runtime::new().unwrap()));
    let server: &'static wiremock::MockServer =
        Box::leak(Box::new(runtime.block_on(wiremock::MockServer::start())));
    let calls = Arc::new(AtomicUsize::new(0));
    runtime.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = request.body_json().unwrap();
                let (message, finish) = match calls.fetch_add(1, Ordering::SeqCst) {
                    0 => (
                        json!({"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"grep","arguments":"{\"pattern\":\"hello\"}"}}]}),
                        "tool_calls",
                    ),
                    _ => (json!({"role":"assistant","content":"found it"}), "stop"),
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
    let config = json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": server.uri()}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": root.path().join("work")}},
        "telemetry": {"event_log": {"enabled": switch == "on"}}
    });
    std::fs::write(root.path().join("base/config.json"), config.to_string()).unwrap();
    world.event_log = Some(EventLogRun { root });
}

#[when("a UDS agent answers a prompt that runs a tool")]
fn uds_agent(world: &mut QuectoWorld) {
    let run = world.event_log.as_ref().unwrap();
    let socket = run.dir("agent.sock");
    let mut child = run
        .command()
        .args(["agent", "--mode", "uds", "--socket"])
        .arg(&socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !socket.exists() {
        assert!(child.try_wait().unwrap().is_none(), "the agent exited");
        assert!(started.elapsed() < Duration::from_secs(30), "no socket");
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    writeln!(
        stream,
        "{}",
        json!({"type":"prompt","message":"Find hello.","id":"p1"})
    )
    .unwrap();
    for line in BufReader::new(stream.try_clone().unwrap()).lines() {
        if line.unwrap().contains("\"agent_end\"") {
            break;
        }
    }
    let _ = Command::new("kill").arg(child.id().to_string()).status();
    let _ = child.wait();
}

fn one_shot(world: &mut QuectoWorld, extra: &[&str]) {
    let run = world.event_log.as_ref().unwrap();
    let status = run
        .command()
        .args(["agent", "-m", "Find hello."])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "{status}");
}

#[when("a one-shot agent answers a prompt that runs a tool")]
fn one_shot_agent(world: &mut QuectoWorld) {
    one_shot(world, &[]);
}

#[when("an ephemeral one-shot agent answers a prompt that runs a tool")]
fn ephemeral_one_shot_agent(world: &mut QuectoWorld) {
    one_shot(world, &["-s", "-"]);
}

#[then("the event log holds the tool result with its duration and sizes")]
fn logged(world: &mut QuectoWorld) {
    use std::os::unix::fs::PermissionsExt;
    let run = world.event_log.as_ref().unwrap();
    let records = run.records();
    let result = records
        .iter()
        .find(|record| record["event"] == "tool_result" && record["tool"] == "grep")
        .unwrap_or_else(|| panic!("no grep tool result in {records:?}"));
    assert!(result["duration_ms"].is_u64(), "{result}");
    assert_eq!(
        result["argument_bytes"],
        "{\"pattern\":\"hello\"}".len(),
        "{result}"
    );
    assert!(result["content_bytes"].as_u64().unwrap() > 0, "{result}");
    assert!(result["unix_ms"].as_u64().unwrap() > 0, "{result}");
    assert!(
        records
            .iter()
            .any(|record| record["event"] == "request_observed")
    );
    let mode = |path: PathBuf| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(run.dir("base/audit")), 0o700);
    for entry in std::fs::read_dir(run.dir("base/audit")).unwrap() {
        assert_eq!(mode(entry.unwrap().path()), 0o600);
    }
}

#[then("no event log was written")]
fn not_logged(world: &mut QuectoWorld) {
    let records = world.event_log.as_ref().unwrap().records();
    assert!(records.is_empty(), "{records:?}");
}
