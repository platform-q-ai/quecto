//! #2218 a finished child's report, end to end: a real `quecto agent --mode
//! uds --spawned` child over a mock provider is handed its task the way a
//! spawn hands it (`follow_up`), finishes the turn and goes idle; the
//! parent-side `AgentCmdTool` then reads its default report through the
//! child's real socket. The report is complete (durable ordinals, no
//! `reportIncomplete`) and, once delivered, a second read is `unchanged`.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use cucumber::given;
use quecto::infrastructure::tools::agent_cmd::AgentCmdTool;
use quecto::infrastructure::tools::subagent_registry::SubagentEntry;

use super::QuectoWorld;

const LIMIT: Duration = Duration::from_secs(60);
/// The child's session key: a spawn names a child's session by its uuid.
const CHILD_SESSION: &str = "0b5e2218-0000-4000-8000-000000002218";

/// The live child process and everything its scenario borrows; the child is
/// killed when the scenario's world is dropped.
#[derive(Default)]
pub struct FinishedChildState {
    _temp: Option<tempfile::TempDir>,
    child: Option<Child>,
    /// The launcher's connection: it keeps the client-bound child alive.
    _holder: Option<UnixStream>,
    /// Serves the mock provider for the child's lifetime.
    _runtime: Option<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for FinishedChildState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FinishedChildState")
            .field("child", &self.child.as_ref().map(Child::id))
            .finish_non_exhaustive()
    }
}

impl Drop for FinishedChildState {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A mock OpenAI-compatible provider whose every reply is `report`.
fn start_mock(report: &str) -> (String, tokio::runtime::Runtime) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let report = report.to_string();
    let uri = rt.block_on(async move {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                let streaming = request
                    .body_json::<serde_json::Value>()
                    .ok()
                    .and_then(|body| body.get("stream").and_then(serde_json::Value::as_bool))
                    == Some(true);
                let usage = serde_json::json!({"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6});
                if streaming {
                    let chunk = serde_json::json!({"choices": [{"index": 0,
                        "delta": {"content": report}, "finish_reason": "stop"}], "usage": usage});
                    wiremock::ResponseTemplate::new(200).set_body_raw(
                        format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                        "text/event-stream",
                    )
                } else {
                    wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "chatcmpl-2218", "object": "chat.completion", "usage": usage,
                        "choices": [{"index": 0, "finish_reason": "stop",
                            "message": {"role": "assistant", "content": report}}]
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

fn write_config(base: &Path, mock: &str) {
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = serde_json::json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": mock}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": workspace}},
    });
    std::fs::write(base.join("config.json"), config.to_string()).unwrap();
}

fn launch_child(base: &Path, socket: &Path) -> (Child, UnixStream) {
    let stderr_path = base.join("child.stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_quecto"))
        .args(["agent", "--mode", "uds", "--spawned", "-s", CHILD_SESSION])
        .arg("--socket")
        .arg(socket)
        .env("QUECTO_BASE_DIR", base)
        .env("HOME", base)
        .current_dir(base.join("workspace"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&stderr_path).unwrap())
        .spawn()
        .expect("launch the child agent");
    let deadline = Instant::now() + LIMIT;
    let stream = loop {
        if let Ok(stream) = UnixStream::connect(socket) {
            break stream;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "the child exited before its socket was ready: {status}\n{}",
                std::fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        assert!(Instant::now() < deadline, "the child's socket never opened");
        std::thread::sleep(Duration::from_millis(20));
    };
    (child, stream)
}

/// Hand the child its task as a spawn does and wait on the same connection
/// for the turn's `agent_end`, the event a real parent's completion note
/// fires on: the turn is saved before it is sent (#2218), so a read right
/// after it already sees the finished report.
fn run_task_to_idle(stream: &UnixStream, task: &str) {
    stream.set_read_timeout(Some(LIMIT)).unwrap();
    let request = serde_json::json!({"type": "follow_up", "id": "task-2218", "message": task});
    (&*stream)
        .write_all(format!("{request}\n").as_bytes())
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let deadline = Instant::now() + LIMIT;
    loop {
        assert!(
            Instant::now() < deadline,
            "the child never finished its task"
        );
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .expect("the child's event stream");
        assert!(read > 0, "the child closed its socket before finishing");
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if event["type"] == "agent_end" {
            return;
        }
    }
}

#[given(
    expr = "a real spawned child agent {string} that finished its task with the report {string}"
)]
fn given_finished_child(world: &mut QuectoWorld, agent_id: String, report: String) {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().to_path_buf();
    let (mock, runtime) = start_mock(&report);
    write_config(&base, &mock);
    let socket = base.join("c.sock");
    let (child, holder) = launch_child(&base, &socket);
    world.finished_child = FinishedChildState {
        _temp: Some(temp),
        child: Some(child),
        _holder: None,
        _runtime: Some(runtime),
    };
    run_task_to_idle(&holder, "reply ok");
    world.finished_child._holder = Some(holder);

    let registry = AgentCmdTool::new_registry();
    registry
        .lock()
        .unwrap()
        .insert(agent_id, SubagentEntry::new(socket, 0));
    world.agent_cmd_tool = Some(AgentCmdTool::new(registry.clone()));
    world.agent_cmd_registry = Some(registry);
}
