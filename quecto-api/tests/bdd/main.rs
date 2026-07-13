#![allow(unused_imports)]

use cucumber::{World, given, then, when};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use quecto_api::application::ports::agent_gateway::{AgentCommand, AgentGateway, EventSubscriber};
use quecto_api::domain::error::ApiError;
use quecto_api::domain::event::AgentEvent;
use quecto_api::infrastructure::http::router::build_router;

// ── Mock gateway ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct MockGateway {
    connected: Arc<AtomicBool>,
}

impl MockGateway {
    fn new(connected: bool) -> Self {
        Self {
            connected: Arc::new(AtomicBool::new(connected)),
        }
    }
}

struct MockSubscriber;

impl EventSubscriber for MockSubscriber {
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = Option<AgentEvent>> + Send + '_>> {
        // Never yields — tests don't exercise long-lived streams here.
        Box::pin(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            None
        })
    }
}

impl AgentGateway for MockGateway {
    fn send(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        let connected = self.connected.load(Ordering::Relaxed);
        Box::pin(async move {
            if !connected {
                return Err(ApiError::AgentNotConnected);
            }
            // Return a successful response for any command.
            let command_name = match cmd {
                AgentCommand::Prompt { .. } => "prompt",
                AgentCommand::Abort => "abort",
                AgentCommand::GetState => "get_state",
                AgentCommand::GetMessages => "get_messages",
                AgentCommand::GetMessagesTail { .. } => "get_messages_tail",
                AgentCommand::GetMessage { .. } => "get_message",
                AgentCommand::GetSessionStats => "get_session_stats",
                AgentCommand::SetModel { .. } => "set_model",
                AgentCommand::ClearHistory => "clear_history",
            };
            Ok(AgentEvent::Response {
                id: Some("mock-id".to_string()),
                command: command_name.to_string(),
                success: true,
                data: Some(serde_json::json!({"mock": true})),
                error: None,
            })
        })
    }

    fn enqueue(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        let connected = self.connected.load(Ordering::Relaxed);
        Box::pin(async move {
            if !connected {
                return Err(ApiError::AgentNotConnected);
            }
            let command_name = match cmd {
                AgentCommand::Prompt { .. } => "prompt",
                AgentCommand::Abort => "abort",
                AgentCommand::GetState => "get_state",
                AgentCommand::GetMessages => "get_messages",
                AgentCommand::GetMessagesTail { .. } => "get_messages_tail",
                AgentCommand::GetMessage { .. } => "get_message",
                AgentCommand::GetSessionStats => "get_session_stats",
                AgentCommand::SetModel { .. } => "set_model",
                AgentCommand::ClearHistory => "clear_history",
            };
            Ok(AgentEvent::Response {
                id: Some("mock-enqueued-id".to_string()),
                command: command_name.to_string(),
                success: true,
                data: Some(serde_json::json!({"accepted": true})),
                error: None,
            })
        })
    }

    fn subscribe(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn EventSubscriber>, ApiError>> + Send + '_>> {
        let connected = self.connected.load(Ordering::Relaxed);
        Box::pin(async move {
            if !connected {
                return Err(ApiError::AgentNotConnected);
            }
            Ok(Box::new(MockSubscriber) as Box<dyn EventSubscriber>)
        })
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

// ── World ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, World)]
pub struct ApiWorld {
    pub agent_connected: bool,
    pub response_status: Option<u16>,
    pub response_body: Option<String>,
    pub server_addr: Option<String>,
}

impl ApiWorld {
    /// Start an axum test server with a mock gateway and return its base URL.
    async fn ensure_server(&mut self) -> String {
        if let Some(ref addr) = self.server_addr {
            return addr.clone();
        }
        let gateway = MockGateway::new(self.agent_connected);
        let app = build_router(gateway);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Small delay to let the server bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.server_addr = Some(base.clone());
        base
    }
}

// ── Given steps ──────────────────────────────────────────────────────────────

#[given("the agent is connected")]
fn agent_connected(world: &mut ApiWorld) {
    world.agent_connected = true;
}

#[given("the agent is not connected")]
fn agent_not_connected(world: &mut ApiWorld) {
    world.agent_connected = false;
}

// ── When steps (HTTP) ────────────────────────────────────────────────────────

#[when(regex = r"^I request GET (.+)$")]
async fn request_get(world: &mut ApiWorld, path: String) {
    let base = world.ensure_server().await;
    let url = format!("{base}{path}");
    let resp = reqwest::get(&url).await.expect("HTTP request failed");
    world.response_status = Some(resp.status().as_u16());
    world.response_body = Some(resp.text().await.unwrap_or_default());
}

#[when("I POST /prompt with body:")]
async fn request_post_prompt(world: &mut ApiWorld, step: &cucumber::gherkin::Step) {
    let body = step
        .docstring
        .as_ref()
        .expect("missing docstring")
        .trim()
        .to_string();
    let base = world.ensure_server().await;
    let url = format!("{base}/prompt");
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("HTTP request failed");
    world.response_status = Some(resp.status().as_u16());
    world.response_body = Some(resp.text().await.unwrap_or_default());
}

// ── Then steps ───────────────────────────────────────────────────────────────

#[then(regex = r"^the response status is (\d+)$")]
fn check_status(world: &mut ApiWorld, expected: u16) {
    let actual = world.response_status.expect("no response received");
    assert_eq!(actual, expected, "expected status {expected}, got {actual}");
}

#[then(regex = r#"^the response body contains (.+)$"#)]
fn check_body_contains(world: &mut ApiWorld, fragment: String) {
    let body = world.response_body.as_deref().expect("no response body");
    // The fragment comes from Gherkin as e.g. "healthy":true — strip surrounding quotes if any.
    let needle = fragment.trim().trim_matches('"');
    assert!(
        body.contains(needle),
        "response body does not contain '{needle}'.\nBody: {body}"
    );
}

// ── Architecture steps ───────────────────────────────────────────────────────

#[then(
    regex = r"^the (domain|application) source should not import from (infrastructure|application|domain)$"
)]
fn layer_should_not_import(_world: &mut ApiWorld, source_layer: String, forbidden_layer: String) {
    let source_dir = format!("src/{}", source_layer);
    let forbidden_pattern = format!("crate::{}", forbidden_layer);
    let violations = check_imports(&source_dir, &forbidden_pattern);
    assert!(
        violations.is_empty(),
        "{source_layer} layer imports {forbidden_layer}: {violations:?}"
    );
}

#[then(regex = r#"^the (domain|application) source should not contain "([^"]+)"$"#)]
fn layer_should_not_contain(_world: &mut ApiWorld, layer: String, pattern: String) {
    let source_dir = format!("src/{}", layer);
    let violations = check_contains(&source_dir, &pattern);
    assert!(
        violations.is_empty(),
        "{layer} layer contains '{pattern}': {violations:?}"
    );
}

fn check_imports(dir: &str, pattern: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    if !base.exists() {
        return violations;
    }
    for entry in walkdir(&base) {
        if entry.extension().is_some_and(|e| e == "rs") {
            if let Ok(content) = std::fs::read_to_string(&entry) {
                for (i, line) in content.lines().enumerate() {
                    if line.contains("use ") && line.contains(pattern) {
                        violations.push(format!("{}:{}: {}", entry.display(), i + 1, line.trim()));
                    }
                }
            }
        }
    }
    violations
}

fn check_contains(dir: &str, pattern: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    if !base.exists() {
        return violations;
    }
    for entry in walkdir(&base) {
        if entry.extension().is_some_and(|e| e == "rs") {
            if let Ok(content) = std::fs::read_to_string(&entry) {
                for (i, line) in content.lines().enumerate() {
                    if line.contains(pattern) && !line.trim_start().starts_with("//") {
                        violations.push(format!("{}:{}: {}", entry.display(), i + 1, line.trim()));
                    }
                }
            }
        }
    }
    violations
}

fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir(&path));
            } else {
                files.push(path);
            }
        }
    }
    files
}

// ── Runner ───────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let tag_filter = std::env::var("QUECTO_TAG").ok();

    ApiWorld::cucumber()
        .max_concurrent_scenarios(1) // serial — each scenario starts its own server
        .filter_run("tests/features", move |feat, _, sc| {
            // Skip websocket tests for now — they need a real agent
            if feat.name.contains("WebSocket") {
                return false;
            }
            if let Some(ref tag) = tag_filter {
                let matches_feature = feat.tags.iter().any(|t| t == tag.as_str());
                let matches_scenario = sc.tags.iter().any(|t| t == tag.as_str());
                if !matches_feature && !matches_scenario {
                    return false;
                }
                return true;
            }
            feat.tags.iter().any(|t| t == "done") || sc.tags.iter().any(|t| t == "done")
        })
        .await;
}
