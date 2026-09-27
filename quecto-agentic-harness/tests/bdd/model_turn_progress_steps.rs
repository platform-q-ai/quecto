//! #2210: supervising a model turn in flight. A real `quecto` UDS agent
//! with an isolated HOME and base directory and its event log on, and a
//! mock OpenAI provider on a raw socket that streams two deltas and then
//! stalls, or streams deltas without end.
use super::QuectoWorld;
use cucumber::{given, then, when};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The output each delta carries.
const DELTA: &str = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijkl";
/// How long a step waits for the agent before failing.
const PATIENCE: Duration = Duration::from_secs(60);

/// How the mock provider replies.
#[derive(Debug, Clone, Copy)]
enum Reply {
    /// Two deltas, then nothing, the connection held open.
    Stalls,
    /// Deltas without end, until the agent hangs up.
    Endless,
}

/// One scenario's run: its directories, its provider, what it saw.
#[derive(Debug)]
pub struct ModelTurnRun {
    root: tempfile::TempDir,
    requests: Arc<AtomicUsize>,
    /// Every line the prompting client received.
    lines: Vec<Value>,
    /// The `get_state` a second client received while the reply streamed.
    state: Option<Value>,
}

impl ModelTurnRun {
    fn dir(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// The UDS agent, started and listening.
    fn start_agent(&self) -> (Child, PathBuf) {
        let socket = self.dir("agent.sock");
        let mut child = Command::new(env!("CARGO_BIN_EXE_quecto"))
            .args(["agent", "--mode", "uds", "--socket"])
            .arg(&socket)
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !socket.exists() {
            assert!(child.try_wait().unwrap().is_none(), "the agent exited");
            assert!(started.elapsed() < PATIENCE, "no socket");
            std::thread::sleep(Duration::from_millis(25));
        }
        (child, socket)
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

    /// The observations of the event log's `request_observed` records.
    fn observations(&self) -> Vec<Value> {
        self.records()
            .into_iter()
            .filter(|record| record["event"] == "request_observed")
            .map(|record| record["observation"].clone())
            .collect()
    }
}

/// A client of the agent's socket, reading its lines.
struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn connect(socket: &PathBuf) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        Self { stream, reader }
    }

    fn send(&mut self, command: Value) {
        writeln!(self.stream, "{command}").unwrap();
    }

    /// Read lines, keeping each, until one satisfies `last`.
    fn read_until(&mut self, kept: &mut Vec<Value>, last: impl Fn(&Value) -> bool) -> Value {
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line).unwrap();
            assert!(read > 0, "the agent closed the socket; lines: {kept:?}");
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            kept.push(value.clone());
            if last(&value) {
                return value;
            }
        }
    }
}

/// Read a request's head and body, returning whether it asked for a chat
/// completion.
fn read_request(stream: &mut TcpStream) -> bool {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return false;
    }
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    first.starts_with("POST ") && first.contains("/chat/completions")
}

fn chunk(data: &str) -> String {
    format!("{:x}\r\n{data}\r\n", data.len())
}

fn delta_event() -> String {
    let event = json!({"choices": [{"index": 0, "delta": {"content": DELTA}}]});
    chunk(&format!("data: {event}\n\n"))
}

/// Answer one connection as `reply` says.
fn respond(mut stream: TcpStream, reply: Reply, requests: &AtomicUsize) {
    if !read_request(&mut stream) {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
        return;
    }
    requests.fetch_add(1, Ordering::SeqCst);
    let role = json!({"choices": [{"index": 0, "delta": {"role": "assistant"}}]});
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{}",
        chunk(&format!("data: {role}\n\n"))
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    match reply {
        Reply::Stalls => {
            let _ = stream.write_all(format!("{}{}", delta_event(), delta_event()).as_bytes());
            let _ = stream.flush();
            // Held open, silent, until the agent hangs up (or long after
            // the scenario ended).
            let mut sink = [0u8; 64];
            let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
            let _ = stream.read(&mut sink);
        }
        Reply::Endless => {
            let delta = delta_event();
            while stream.write_all(delta.as_bytes()).is_ok() {}
        }
    }
}

/// A mock OpenAI provider answering every chat completion as `reply` says.
fn provider(reply: Reply) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let counted = counted.clone();
            std::thread::spawn(move || respond(stream, reply, &counted));
        }
    });
    (url, requests)
}

#[given(
    regex = r"^a model turn workspace whose provider (streams two deltas and then stalls|streams without end)$"
)]
fn workspace(world: &mut QuectoWorld, reply: String) {
    let reply = match reply.as_str() {
        "streams two deltas and then stalls" => Reply::Stalls,
        _ => Reply::Endless,
    };
    let root = tempfile::tempdir().unwrap();
    for dir in ["home", "base", "work"] {
        std::fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    let (url, requests) = provider(reply);
    let config = json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": url}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": root.path().join("work")}},
        "telemetry": {"event_log": {"enabled": true}}
    });
    std::fs::write(root.path().join("base/config.json"), config.to_string()).unwrap();
    world.model_turn = Some(ModelTurnRun {
        root,
        requests,
        lines: Vec::new(),
        state: None,
    });
}

/// Run a step's blocking work — a real agent process, its socket, its
/// exit — on a thread of its own, awaiting it without blocking the shared
/// scenario executor (#2210 review): other scenarios run meanwhile.
async fn off_executor(
    world: &mut QuectoWorld,
    work: impl FnOnce(&mut ModelTurnRun) + Send + 'static,
) {
    let mut run = world.model_turn.take().expect("a model turn workspace");
    let (done, finished) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&mut run)));
        let _ = done.send(outcome.map(|()| run));
    });
    match finished.await.expect("the step's thread reports back") {
        Ok(run) => world.model_turn = Some(run),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn is_token(line: &Value) -> bool {
    line["type"] == "token"
}

/// Prompt a fresh agent and wait for its first streamed token.
fn prompt(run: &mut ModelTurnRun) -> (Child, PathBuf, Client) {
    let (child, socket) = run.start_agent();
    let mut client = Client::connect(&socket);
    client.send(json!({"type": "prompt", "message": "Write a lot.", "id": "p1"}));
    client.read_until(&mut run.lines, is_token);
    (child, socket, client)
}

/// Stop the agent with SIGTERM and wait for it to exit.
fn terminate(mut child: Child) {
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > PATIENCE {
            let _ = child.kill();
            panic!("the agent did not exit after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[when("a UDS agent is prompted and its state is requested while the reply streams")]
async fn state_while_streaming(world: &mut QuectoWorld) {
    off_executor(world, state_while_streaming_blocking).await;
}

fn state_while_streaming_blocking(run: &mut ModelTurnRun) {
    let (child, socket, _client) = prompt(run);
    // A supervising parent connects as a second client: a busy agent
    // answers with its point-in-time state at once.
    let mut parent = Client::connect(&socket);
    let mut seen = Vec::new();
    let state = parent.read_until(&mut seen, |line| line["command"] == "get_state");
    run.state = Some(state);
    terminate(child);
}

#[when("a UDS agent is prompted and stopped by a termination signal while the reply streams")]
async fn signalled_while_streaming(world: &mut QuectoWorld) {
    off_executor(world, signalled_while_streaming_blocking).await;
}

fn signalled_while_streaming_blocking(run: &mut ModelTurnRun) {
    let (child, _socket, _client) = prompt(run);
    terminate(child);
}

#[when(regex = r"^a one-shot agent is prompted with a (\d+) second run deadline$")]
async fn one_shot_with_deadline(world: &mut QuectoWorld, seconds: String) {
    off_executor(world, move |run| {
        one_shot_with_deadline_blocking(run, &seconds)
    })
    .await;
}

fn one_shot_with_deadline_blocking(run: &mut ModelTurnRun, seconds: &str) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_quecto"))
        .args(["agent", "-m", "Write a lot.", "--max-time", seconds])
        .current_dir(run.dir("work"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", run.dir("home"))
        .env("XDG_CONFIG_HOME", run.dir("home/.config"))
        .env("XDG_DATA_HOME", run.dir("home/.local/share"))
        .env("XDG_STATE_HOME", run.dir("home/.local/state"))
        .env("XDG_RUNTIME_DIR", run.root.path())
        .env("QUECTO_BASE_DIR", run.dir("base"))
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > PATIENCE {
            let _ = child.kill();
            panic!("the one-shot agent outlived its deadline");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[then("the event log holds the request as cancelled with its attempt interrupted")]
fn interrupted_request_recorded_without_output(world: &mut QuectoWorld) {
    let run = world.model_turn.as_ref().unwrap();
    let observations = run.observations();
    let cancelled: Vec<_> = observations
        .iter()
        .filter(|observation| observation["outcome"] == "cancelled")
        .collect();
    assert_eq!(cancelled.len(), 1, "{observations:?}");
    let attempts = cancelled[0]["attempt_diagnostics"].as_array().unwrap();
    assert_eq!(attempts.len(), 1, "{attempts:?}");
    assert_eq!(attempts[0]["termination"], "Interrupted", "{attempts:?}");
}

#[when("a UDS agent is prompted until its turn ends")]
async fn prompted_until_the_turn_ends(world: &mut QuectoWorld) {
    off_executor(world, prompted_until_the_turn_ends_blocking).await;
}

fn prompted_until_the_turn_ends_blocking(run: &mut ModelTurnRun) {
    let (child, socket) = run.start_agent();
    let mut client = Client::connect(&socket);
    client.send(json!({"type": "prompt", "message": "Write a lot.", "id": "p1"}));
    client.read_until(&mut run.lines, |line| line["command"] == "agent_error");
    terminate(child);
}

#[then("the state shows the model turn with its attempt's events, output and last event")]
fn state_shows_the_model_turn(world: &mut QuectoWorld) {
    let run = world.model_turn.as_ref().unwrap();
    let state = run.state.as_ref().expect("a get_state line");
    let turn = &state["data"]["modelTurn"];
    assert!(turn["elapsedMs"].is_u64(), "{state}");
    assert!(turn["outputCapBytes"].as_u64().unwrap() > 0, "{state}");
    let attempt = &turn["attempt"];
    assert_eq!(attempt["number"], 1, "{state}");
    assert_eq!(attempt["outputBytes"], 2 * DELTA.len(), "{state}");
    assert_eq!(
        attempt["events"], 3,
        "the role chunk and two deltas: {state}"
    );
    assert!(attempt["sinceLastEventMs"].is_u64(), "{state}");
    assert!(attempt["firstTokenMs"].is_u64(), "{state}");
    // The parent's own check of a busy child's snapshot accepts it.
    use quecto::domain::state_snapshot::{StateSnapshot, UnchangedSnapshot};
    let typed = StateSnapshot::read_forward_compatible(&state["data"]).expect("accepted");
    assert!(typed.model_turn.is_some());
    // Polled at the generation it saw, a parent relays the small unchanged
    // marker, which still carries the model turn.
    let marker = serde_json::to_value(UnchangedSnapshot::at(typed.generation, &typed)).unwrap();
    assert_eq!(marker["modelTurn"], state["data"]["modelTurn"]);
    assert_eq!(marker.as_object().unwrap().len(), 3, "{marker}");
}

#[then(
    "the event log holds the request as cancelled with its attempt interrupted after its output"
)]
fn interrupted_request_recorded(world: &mut QuectoWorld) {
    let run = world.model_turn.as_ref().unwrap();
    let observations = run.observations();
    let cancelled: Vec<_> = observations
        .iter()
        .filter(|observation| observation["outcome"] == "cancelled")
        .collect();
    assert_eq!(cancelled.len(), 1, "{observations:?}");
    let attempt = cancelled[0]["attempt_diagnostics"]
        .as_array()
        .and_then(|attempts| attempts.last())
        .unwrap_or_else(|| panic!("an attempt record: {observations:?}"));
    assert_eq!(attempt["termination"], "Interrupted", "{attempt}");
    assert_eq!(attempt["output_bytes"], 2 * DELTA.len(), "{attempt}");
    assert_eq!(attempt["event_count"], 3, "{attempt}");
    assert!(attempt["first_token_ms"].is_u64(), "{attempt}");
}

#[then("the turn ends with the output cap error")]
fn turn_ends_with_the_cap_error(world: &mut QuectoWorld) {
    let run = world.model_turn.as_ref().unwrap();
    assert!(
        run.lines
            .iter()
            .any(|line| line.to_string().contains("output cap exceeded: ")),
        "{:?}",
        run.lines
    );
}

#[then("the provider was asked once")]
fn provider_asked_once(world: &mut QuectoWorld) {
    let run = world.model_turn.as_ref().unwrap();
    assert_eq!(run.requests.load(Ordering::SeqCst), 1);
}

#[then("the event log holds the attempt stopped at its output cap")]
fn capped_attempt_recorded(world: &mut QuectoWorld) {
    let run = world.model_turn.as_ref().unwrap();
    let observations = run.observations();
    let failed: Vec<_> = observations
        .iter()
        .filter(|observation| observation["error_class"] == "output_capped")
        .collect();
    assert_eq!(failed.len(), 1, "{observations:?}");
    let attempts = failed[0]["attempt_diagnostics"].as_array().unwrap();
    assert_eq!(attempts.len(), 1, "{attempts:?}");
    assert_eq!(attempts[0]["termination"], "OutputCapped");
    assert!(attempts[0]["output_bytes"].as_u64().unwrap() > 0);
    assert!(
        run.records().iter().any(|record| record["event"] == "provider_error"
            && record["class"] == "output_capped"),
        "the terminal failure is in the event log"
    );
}
