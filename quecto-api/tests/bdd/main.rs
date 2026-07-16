#![allow(unused_imports)]

use cucumber::{World, given, then, when};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::{SinkExt, StreamExt};

use quecto_api::application::ports::agent_gateway::{AgentCommand, AgentGateway, EventSubscriber};
use quecto_api::domain::error::ApiError;
use quecto_api::domain::event::AgentEvent;
use quecto_api::infrastructure::http::router::build_router;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

// ── Mock gateway ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct MockGateway {
    connected: Arc<AtomicBool>,
}

const OVERSIZED_REF: &str = "oversized-message-ref";

fn oversized_body() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..quecto_line_io::PROTOCOL_LINE_CAP_BYTES + 1024)
        .map(|idx| ALPHABET[idx % ALPHABET.len()] as char)
        .collect()
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
            // Return a successful response for any command. GetMessages echoes
            // its paging cursor so scenarios can pin that the router actually
            // propagates `?before=` to the gateway (#1061 review follow-up) —
            // without the echo, dropping the cursor would still return 200.
            let mut data = serde_json::json!({"mock": true});
            let command_name = match cmd {
                AgentCommand::Prompt { .. } => "prompt",
                AgentCommand::Abort => "abort",
                AgentCommand::GetState => "get_state",
                AgentCommand::GetMessages { before } => {
                    if let Some(before) = before {
                        data["cursorEcho"] = serde_json::Value::String(before);
                    }
                    "get_messages"
                }
                AgentCommand::GetMessagesTail { .. } => "get_messages_tail",
                AgentCommand::GetMessage {
                    message_id,
                    tool_call_id: _,
                    offset,
                    limit,
                    ..
                } => {
                    let body = oversized_body();
                    let start = offset.unwrap_or(0).min(body.len());
                    let requested = limit.unwrap_or(body.len() - start);
                    let mut end = (start + requested).min(body.len());
                    while !body.is_char_boundary(end) {
                        end -= 1;
                    }
                    data = serde_json::json!({
                        "id": message_id,
                        "role": "assistant",
                        "content": &body[start..end],
                        "offset": start,
                        "nextOffset": end,
                        "contentLength": body.len(),
                        "hasMoreContent": end < body.len(),
                    });
                    "get_message"
                }
                AgentCommand::GetSessionStats => "get_session_stats",
                AgentCommand::SetModel { .. } => "set_model",
                AgentCommand::ClearHistory => "clear_history",
            };
            Ok(AgentEvent::Response {
                id: Some("mock-id".to_string()),
                command: command_name.to_string(),
                success: true,
                data: Some(data),
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
                AgentCommand::GetMessages { .. } => "get_messages",
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
    pub ws: Option<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>,
    pub ws_fragments: Vec<serde_json::Value>,
    pub ws_reassembled: String,
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

#[given("the agent is connected with prior history containing an oversized message")]
fn agent_connected_with_oversized_history(world: &mut ApiWorld) {
    world.agent_connected = true;
}

#[given("the agent is not connected")]
fn agent_not_connected(world: &mut ApiWorld) {
    world.agent_connected = false;
}

// ── When steps (HTTP) ────────────────────────────────────────────────────────

#[when("I connect a WebSocket to /ws")]
async fn connect_websocket(world: &mut ApiWorld) {
    let base = world.ensure_server().await.replace("http://", "ws://");
    let (ws, _) = connect_async(format!("{base}/ws"))
        .await
        .expect("websocket connects");
    world.ws = Some(ws);
}

#[when("I request the oversized message by its stable reference via the WebSocket")]
async fn request_oversized_message_via_websocket(world: &mut ApiWorld) {
    let mut offset = 0usize;
    loop {
        let request_id = format!("oversized-page-{offset}");
        let request = serde_json::json!({
            "type": "get_message",
            "id": request_id,
            "messageId": OVERSIZED_REF,
            "offset": offset,
            "limit": quecto_line_io::PROTOCOL_LINE_CAP_BYTES / 2,
        });
        let ws = world.ws.as_mut().expect("websocket connected");
        ws.send(WsMessage::Text(request.to_string().into()))
            .await
            .expect("send get_message request");
        let msg = ws
            .next()
            .await
            .expect("websocket yields response")
            .expect("websocket frame ok");
        let text = msg.into_text().expect("text response");
        let response: serde_json::Value = serde_json::from_str(&text).expect("json response");
        assert_eq!(
            response.get("id").and_then(|value| value.as_str()),
            Some(request_id.as_str()),
            "each WebSocket page must preserve its caller correlation id"
        );
        let data = response.get("data").expect("response data");
        world.ws_reassembled.push_str(
            data.get("content")
                .and_then(|v| v.as_str())
                .expect("response content"),
        );
        let next = data
            .get("nextOffset")
            .and_then(|v| v.as_u64())
            .expect("nextOffset") as usize;
        let more = data
            .get("hasMoreContent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        world.ws_fragments.push(response);
        if !more {
            break;
        }
        assert!(
            next > offset,
            "WebSocket get_message pagination must progress"
        );
        offset = next;
    }
}

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

#[then(
    "each oversized-message response fragment delivered to the WebSocket should stay within the protocol frame cap"
)]
fn ws_fragments_bounded(world: &mut ApiWorld) {
    assert!(
        !world.ws_fragments.is_empty(),
        "no WebSocket response fragments"
    );
    for fragment in &world.ws_fragments {
        let line = serde_json::to_string(fragment).expect("fragment serializes");
        assert!(
            line.len() <= quecto_line_io::PROTOCOL_LINE_CAP_BYTES,
            "WebSocket fragment exceeded protocol cap: {} > {}",
            line.len(),
            quecto_line_io::PROTOCOL_LINE_CAP_BYTES
        );
    }
}

#[then("the WebSocket client should receive the complete reassembled message body")]
fn ws_complete_body(world: &mut ApiWorld) {
    assert_eq!(world.ws_reassembled, oversized_body());
    assert!(
        world.ws_fragments.len() >= 3,
        "body larger than one protocol page should require multiple fragments"
    );
}

#[then("the WebSocket remains open")]
async fn ws_remains_open(world: &mut ApiWorld) {
    let ws = world.ws.as_mut().expect("websocket connected");
    ws.send(WsMessage::Ping(Vec::new().into()))
        .await
        .expect("ping succeeds on open WebSocket");
}

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
            if sc.tags.contains(&String::from("wip")) {
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
