// HTTP router — maps HTTP/WebSocket endpoints to application use cases.

use std::{path::PathBuf, sync::Arc};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::application::use_cases;

/// Shared application state, injected into every handler.
pub struct AppState<G: AgentGateway> {
    pub gateway: G,
}

/// Build the axum router.
pub fn build_router<G: AgentGateway + Clone + 'static>(gateway: G) -> Router {
    let state = Arc::new(AppState { gateway });
    Router::new()
        .route("/health", get(health_handler::<G>))
        .route("/prompt", post(prompt_handler::<G>))
        .route("/state", get(state_handler::<G>))
        .route("/messages", get(messages_handler::<G>))
        .route("/messages/tail", get(messages_tail_handler::<G>))
        .route("/audit/events", get(audit_events_handler::<G>))
        .route("/stats", get(stats_handler::<G>))
        .route("/ws", get(ws_handler::<G>))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    healthy: bool,
    agent_connected: bool,
}

async fn health_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    let result = use_cases::health_check::execute(&state.gateway);
    let status = if result.healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(HealthResponse {
            healthy: result.healthy,
            agent_connected: result.agent_connected,
        }),
    )
}

// ── Prompt ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PromptRequest {
    message: String,
    #[serde(rename = "streamingBehavior")]
    streaming_behavior: Option<String>,
    #[serde(rename = "waitForCompletion", default = "default_wait_for_completion")]
    wait_for_completion: bool,
}

fn default_wait_for_completion() -> bool {
    true
}

async fn prompt_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<PromptRequest>,
) -> impl IntoResponse {
    if body.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid request: message must not be empty"})),
        )
            .into_response();
    }

    let input = use_cases::send_prompt::SendPromptInput {
        message: body.message,
        streaming_behavior: body.streaming_behavior,
        wait_for_completion: body.wait_for_completion,
    };

    match use_cases::send_prompt::execute(&state.gateway, input).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

async fn state_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    match use_cases::get_state::execute(&state.gateway).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

// ── Messages ──────────────────────────────────────────────────────────────────

async fn messages_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    if !state.gateway.is_connected() {
        return api_error_response(crate::domain::error::ApiError::AgentNotConnected)
            .into_response();
    }
    match state.gateway.send(AgentCommand::GetMessages).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

#[derive(Deserialize)]
struct TailQuery {
    n: Option<usize>,
}

async fn messages_tail_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Query(params): Query<TailQuery>,
) -> impl IntoResponse {
    if !state.gateway.is_connected() {
        return api_error_response(crate::domain::error::ApiError::AgentNotConnected)
            .into_response();
    }
    let count = params.n.unwrap_or(10);
    match state
        .gateway
        .send(AgentCommand::GetMessagesTail { count })
        .await
    {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

// ── Audit events ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AuditEventsQuery {
    after: Option<usize>,
    limit: Option<usize>,
}

async fn audit_events_handler<G: AgentGateway>(
    Query(params): Query<AuditEventsQuery>,
) -> impl IntoResponse {
    let after = params.after.unwrap_or(0);
    let limit = params.limit.unwrap_or(500).min(2_000);

    match read_audit_events(after, limit).await {
        Ok((events, next_offset)) => (
            StatusCode::OK,
            Json(serde_json::json!({"data": {"events": events, "next_offset": next_offset}})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}

async fn read_audit_events(
    after: usize,
    limit: usize,
) -> Result<(Vec<serde_json::Value>, usize), std::io::Error> {
    let path = audit_log_path();
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };

    let mut next_offset = 0;
    let events = content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            next_offset = index + 1;
            if index < after || index >= after.saturating_add(limit) {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(line).ok()
        })
        .collect();

    Ok((events, next_offset))
}

fn audit_log_path() -> PathBuf {
    let base_dir =
        std::env::var("QUECTO_BASE_DIR").unwrap_or_else(|_| "/home/appuser/.quecto".to_string());
    let session_key = std::env::var("QUECTO_SESSION_KEY").unwrap_or_else(|_| "default".to_string());
    PathBuf::from(base_dir)
        .join("audit")
        .join(format!("{}.jsonl", sanitize_session_key(&session_key)))
}

fn sanitize_session_key(key: &str) -> String {
    if key.is_empty() || key.starts_with('.') || key.chars().all(|c| c == '.') {
        return hex_encode(key);
    }
    if key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.'))
    {
        return key.replace(':', "_");
    }
    hex_encode(key)
}

fn hex_encode(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len() * 2 + 4);
    encoded.push_str("key_");
    for byte in key.as_bytes() {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

// ── Stats ─────────────────────────────────────────────────────────────────────

async fn stats_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    if !state.gateway.is_connected() {
        return api_error_response(crate::domain::error::ApiError::AgentNotConnected)
            .into_response();
    }
    match state.gateway.send(AgentCommand::GetSessionStats).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

// ── WebSocket ─────────────────────────────────────────────────────────────────

async fn ws_handler<G: AgentGateway + Clone + 'static>(
    State(state): State<Arc<AppState<G>>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(state, socket))
}

async fn handle_ws<G: AgentGateway + Clone>(state: Arc<AppState<G>>, mut socket: WebSocket) {
    // Subscribe to agent events.
    let mut subscriber = match state.gateway.subscribe().await {
        Ok(sub) => sub,
        Err(_) => {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    // Spawn a task that reads from the WebSocket and sends commands to the agent.
    // We use a channel to coordinate: the reader task sends incoming messages,
    // and the main loop forwards events back to the client.
    let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel::<String>(32);

    let gateway = state.gateway.clone();
    let cmd_task = tokio::spawn(async move {
        while let Some(text) = incoming_rx.recv().await {
            if let Ok(req) = serde_json::from_str::<PromptRequest>(&text) {
                if !req.message.is_empty() {
                    let _ = gateway
                        .send(AgentCommand::Prompt {
                            message: req.message,
                            streaming_behavior: req.streaming_behavior,
                        })
                        .await;
                }
            }
        }
    });

    // Main loop: concurrently read from WS and write agent events.
    loop {
        tokio::select! {
            // Agent event → WS
            event = subscriber.recv() => {
                match event {
                    Some(ev) => {
                        if let Ok(json) = serde_json::to_string(&ev) {
                            if socket.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
            // WS message → agent
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let _ = incoming_tx.send(text.to_string()).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    drop(incoming_tx);
    cmd_task.abort();
}

// ── Error mapping ─────────────────────────────────────────────────────────────

fn api_error_response(
    err: crate::domain::error::ApiError,
) -> (StatusCode, Json<serde_json::Value>) {
    let (status, message) = match &err {
        crate::domain::error::ApiError::AgentNotConnected => {
            (StatusCode::SERVICE_UNAVAILABLE, err.to_string())
        }
        crate::domain::error::ApiError::AgentBusy => (StatusCode::CONFLICT, err.to_string()),
        crate::domain::error::ApiError::Timeout(_) => {
            (StatusCode::GATEWAY_TIMEOUT, err.to_string())
        }
        crate::domain::error::ApiError::InvalidRequest(_) => {
            (StatusCode::BAD_REQUEST, err.to_string())
        }
        crate::domain::error::ApiError::Internal(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
        }
    };
    (status, Json(serde_json::json!({"error": message})))
}
