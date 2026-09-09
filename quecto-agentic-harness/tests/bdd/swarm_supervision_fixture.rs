//! Real UDS process with a synthetic, private container launch contract.
//! This exercises production composition without claiming PID namespace isolation.
use quecto::infrastructure::tools::subagent_registry::send_subagent_uds_command_with_timeout;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub struct Runtime {
    child: Child,
    socket: PathBuf,
    pub requests: Arc<AtomicUsize>,
    _server: wiremock::MockServer,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
/// What the fake provider answers for request `index` after the run was
/// created: assistant text, or a terminal provider failure.
#[derive(Clone, Copy)]
pub enum Reply {
    Text(&'static str),
    /// HTTP 401 with an invalid-key body: terminal, never retried, so the
    /// agent suspends its automatic turns.
    TerminalFailure,
}

pub type Script = fn(usize) -> Reply;

fn default_script(index: usize) -> Reply {
    if index == 1 {
        Reply::Text("READY")
    } else {
        Reply::Text("APPROVED")
    }
}

impl Runtime {
    pub async fn start(workspace: &Path) -> Self {
        Self::start_with(workspace, default_script).await
    }

    /// Request 0 always creates the run; `script` answers the rest.
    pub async fn start_with(workspace: &Path, script: Script) -> Self {
        let server = wiremock::MockServer::start().await;
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                let index = seen.fetch_add(1, Ordering::SeqCst);
                if index > 0 && let Reply::TerminalFailure = script(index) {
                    return wiremock::ResponseTemplate::new(401).set_body_json(json!({
                        "error": {"message": "Incorrect API key provided", "type": "invalid_request_error", "code": "invalid_api_key"}
                    }));
                }
                let message = if index == 0 {
                    json!({"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"create-run","type":"function","function":{"name":"swarm","arguments":json!({"op":"create","goal":"ship approved feature","constraints":[],"criteria":[{"id":"tests","kind":"command","description":"pass"}],"member_limit":2,"deadline":deadline}).to_string()}}]})
                } else {
                    let Reply::Text(text) = script(index) else { unreachable!("failures answered above") };
                    json!({"role":"assistant","content":text})
                };
                let finish = if index == 0 {"tool_calls"} else {"stop"};
                let usage = json!({"prompt_tokens":10,"completion_tokens":2,"total_tokens":12});
                let input: Value = request.body_json().unwrap();
                let response = if input["stream"] == true {
                    let chunk = json!({"choices":[{"index":0,"delta":message,"finish_reason":finish}],"usage":usage});
                    wiremock::ResponseTemplate::new(200).set_body_raw(format!("data: {chunk}\n\ndata: [DONE]\n\n"), "text/event-stream")
                } else {
                    wiremock::ResponseTemplate::new(200).set_body_json(json!({"id":"swarm-bdd","object":"chat.completion","choices":[{"index":0,"message":message,"finish_reason":finish}],"usage":usage}))
                };
                if index == 2 { response.set_delay(Duration::from_secs(4)) } else {response}
            }).mount(&server).await;
        let config = workspace.join("supervision-config.json");
        std::fs::write(&config, json!({"providers":{"openai":{"api_key":"sk-test","api_base":server.uri()}},"agents":{"defaults":{"model":"openai-api/gpt-4o-mini","workspace":workspace}}}).to_string()).unwrap();
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    repository.join(path)
                }
            })
            .unwrap_or_else(|| repository.join("target"));
        let socket = workspace.join("supervisor.sock");
        let child = Command::new(target.join("debug/quecto"))
            .args([
                "agent",
                "--mode",
                "uds",
                "--persist",
                "--session",
                "swarm-supervision",
            ])
            .arg("--socket")
            .arg(&socket)
            .arg("--config")
            .arg(config)
            .env("QUECTO_BASE_DIR", workspace.join("runtime"))
            .env("QUECTO_SWARM_CHECKOUT", workspace)
            .env("QUECTO_SWARM_CONTAINER", "isolated-pid-v1")
            .env("QUECTO_SWARM_HOST_PID_NS", "pid:[0]")
            .env("QUECTO_SWARM_MEMBER", "coordinator")
            .env("QUECTO_SWARM_BOOTSTRAP", "1")
            .env_remove("QUECTO_SWARM_RESERVATION")
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(workspace.join("supervisor.stderr")).unwrap())
            .spawn()
            .unwrap();
        let mut runtime = Self {
            child,
            socket,
            requests,
            _server: server,
        };
        let until = tokio::time::Instant::now() + Duration::from_secs(15);
        while !runtime.socket.exists() {
            assert!(
                runtime.child.try_wait().unwrap().is_none(),
                "swarm runtime exited during startup"
            );
            assert!(
                tokio::time::Instant::now() < until,
                "swarm socket startup timed out"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        runtime
    }
    pub async fn command(&self, input: Value) -> Value {
        let reply = send_subagent_uds_command_with_timeout(
            &self.socket,
            &input.to_string(),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["success"], true, "{reply}");
        reply
    }
    pub async fn wait_report(&self, expected: &str) -> Value {
        let until = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let report = self.command(json!({"type":"get_report"})).await;
            if report["data"]["report"]["content"] == expected {
                return report;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "report did not become {expected}: {report}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    pub async fn wait_receipt(&self, id: &Value, expected: &str) -> Value {
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = self.command(json!({"type":"get_state"})).await;
            if let Some(receipt) = state["data"]["controlReceipts"]
                .as_array()
                .and_then(|rows| {
                    rows.iter()
                        .find(|row| &row["id"] == id && row["status"] == expected)
                })
            {
                return receipt.clone();
            }
            assert!(
                tokio::time::Instant::now() < until,
                "receipt did not become {expected}: {state}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    /// The agent's stderr so far (diagnostics for a failed expectation).
    pub fn stderr_tail(&self) -> String {
        let path = self.socket.with_file_name("supervisor.stderr");
        let text = std::fs::read_to_string(path).unwrap_or_default();
        text.lines()
            .rev()
            .take(25)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Poll `get_state` until `accept` holds or the deadline passes.
    pub async fn wait_state(&self, what: &str, accept: impl Fn(&Value) -> bool) -> Value {
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = self.command(json!({"type":"get_state"})).await;
            if accept(&state["data"]) {
                return state;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "state never became {what}: {state}\n--- agent stderr ---\n{}",
                self.stderr_tail()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub async fn wait_idle(&self) {
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = self.command(json!({"type":"get_state"})).await;
            if state["data"]["state"] == "idle" {
                return;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "agent did not become idle: {state}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    pub async fn finish(mut self) {
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "swarm container runtime must remain alive for inspection"
        );
        // SIGTERM is handled by the real UDS server, allowing orderly cleanup and
        // coverage/profile flushing. Drop has a bounded fallback on assertion failure.
        assert!(
            Command::new("kill")
                .args(["-TERM", &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if self.child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "graceful shutdown timed out"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}
