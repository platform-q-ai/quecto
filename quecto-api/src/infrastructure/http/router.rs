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

use crate::application::ports::agent_gateway::{
    AgentCommand, AgentGateway, ToolPolicyApplyModePayload, ToolPolicyMutationPayload,
};
use crate::application::use_cases;
use crate::domain::event::AgentEvent;

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
        .route("/steer", post(steer_handler::<G>))
        .route("/follow_up", post(follow_up_handler::<G>))
        .route("/abort", post(abort_handler::<G>))
        .route("/model", post(set_model_handler::<G>))
        .route("/effort", post(set_effort_handler::<G>))
        .route("/clear_history", post(clear_history_handler::<G>))
        .route("/subagents", get(subagents_handler::<G>))
        .route("/tools", get(tools_handler::<G>))
        .route("/tools/catalogue", get(tools_handler::<G>))
        .route("/tools/policy", post(set_tool_policy_handler::<G>))
        .route("/state", get(state_handler::<G>))
        .route("/messages", get(messages_handler::<G>))
        .route("/messages/tail", get(messages_tail_handler::<G>))
        .route("/messages/{id}", get(message_handler::<G>))
        .route("/audit/events", get(audit_events_handler))
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

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsCommandRequest {
    GetMessage {
        id: Option<String>,
        #[serde(rename = "messageId")]
        message_id: String,
        agent_id: Option<String>,
        #[serde(rename = "toolCallId")]
        tool_call_id: Option<String>,
        offset: Option<usize>,
        limit: Option<usize>,
    },
}

fn default_wait_for_completion() -> bool {
    true
}

fn ws_error_response(id: Option<String>, command: &str, error: impl Into<String>) -> AgentEvent {
    AgentEvent::Response {
        id,
        command: command.into(),
        success: false,
        data: None,
        error: Some(error.into()),
    }
}

fn with_response_id(event: AgentEvent, id: Option<String>) -> AgentEvent {
    match event {
        AgentEvent::Response {
            command,
            success,
            data,
            error,
            ..
        } => AgentEvent::Response {
            id,
            command,
            success,
            data,
            error,
        },
        other => other,
    }
}

fn is_direct_ws_command_response(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::Response { command, .. } if command == "get_message"
    )
}

fn command_type_from_text(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("type")?.as_str().map(ToOwned::to_owned))
}

fn command_id_from_text(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("id")?.as_str().map(ToOwned::to_owned))
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

// ── Steer / Follow-up / Abort ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct MessageRequest {
    message: String,
}

async fn steer_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<MessageRequest>,
) -> impl IntoResponse {
    match use_cases::steer::execute(&state.gateway, body.message).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

async fn follow_up_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<MessageRequest>,
) -> impl IntoResponse {
    match use_cases::follow_up::execute(&state.gateway, body.message).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

async fn abort_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    match use_cases::abort::execute(&state.gateway).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

// ── Set model ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SetModelRequest {
    model: Option<String>,
    provider: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
}

async fn set_model_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<SetModelRequest>,
) -> impl IntoResponse {
    let input = use_cases::set_model::SetModelInput {
        model: body.model,
        provider: body.provider,
        model_id: body.model_id,
    };
    match use_cases::set_model::execute(&state.gateway, input).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

// ── Set effort / Clear history ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct SetEffortRequest {
    effort: String,
}

async fn set_effort_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<SetEffortRequest>,
) -> impl IntoResponse {
    match use_cases::set_effort::execute(&state.gateway, body.effort).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

async fn clear_history_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    match use_cases::clear_history::execute(&state.gateway).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

// ── Subagents / Tools ──────────────────────────────────────────────────────────

async fn subagents_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    match use_cases::get_subagents::execute(&state.gateway).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

async fn tools_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    match use_cases::tools::catalogue(&state.gateway).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetToolPolicyRequest {
    mutations: Vec<ToolPolicyMutationPayload>,
    mode: ToolPolicyApplyModePayload,
}

async fn set_tool_policy_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<SetToolPolicyRequest>,
) -> impl IntoResponse {
    match use_cases::set_tool_policy::execute(&state.gateway, body.mutations, body.mode).await {
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

#[derive(Deserialize)]
struct MessagesQuery {
    /// #1061: page backward — a stable message id from a prior page's `before`.
    before: Option<String>,
}

/// #1061: history is paged. Returns the newest bounded page (or the page before
/// `?before=<id>`); the response's `data` carries `before`/`hasMoreBefore` so
/// clients walk older history explicitly instead of receiving one monolithic
/// (and previously silently trimmed) snapshot.
async fn messages_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Query(params): Query<MessagesQuery>,
) -> impl IntoResponse {
    if !state.gateway.is_connected() {
        return api_error_response(crate::domain::error::ApiError::AgentNotConnected)
            .into_response();
    }
    match state
        .gateway
        .send(AgentCommand::GetMessages {
            before: params.before,
        })
        .await
    {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

#[derive(Deserialize)]
struct MessageQuery {
    /// Forward the lookup to a spawned child agent by id (#1060).
    agent_id: Option<String>,
    /// Byte offset into message content for bounded recovery (#1094).
    offset: Option<usize>,
    /// Maximum content bytes to return for bounded recovery (#1094).
    limit: Option<usize>,
}

/// #1060: resolve a single message by its stable id — the on-demand lookup for
/// refs carried on `agent_end` / `turn_end`, so a WS/REST client that only holds
/// refs can fetch the full content.
async fn message_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Query(params): Query<MessageQuery>,
) -> impl IntoResponse {
    if !state.gateway.is_connected() {
        return api_error_response(crate::domain::error::ApiError::AgentNotConnected)
            .into_response();
    }
    match state
        .gateway
        .send(AgentCommand::GetMessage {
            message_id: id,
            agent_id: params.agent_id,
            tool_call_id: None,
            offset: params.offset,
            limit: params.limit,
        })
        .await
    {
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

async fn audit_events_handler(Query(params): Query<AuditEventsQuery>) -> impl IntoResponse {
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
    let (command_event_tx, mut command_event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(32);

    let gateway = state.gateway.clone();
    let cmd_task = tokio::spawn(async move {
        while let Some(text) = incoming_rx.recv().await {
            match serde_json::from_str::<WsCommandRequest>(&text) {
                Ok(WsCommandRequest::GetMessage {
                    id,
                    message_id,
                    agent_id,
                    tool_call_id,
                    offset,
                    limit,
                }) => {
                    let event = match gateway
                        .send(AgentCommand::GetMessage {
                            message_id,
                            agent_id,
                            tool_call_id,
                            offset,
                            limit,
                        })
                        .await
                    {
                        Ok(event) => with_response_id(event, id),
                        Err(err) => ws_error_response(id, "get_message", err.to_string()),
                    };
                    let _ = command_event_tx.send(event).await;
                    continue;
                }
                Err(err) => {
                    // Only malformed commands owned by this direct-command
                    // parser are errors here. Other typed payloads (notably
                    // `type: prompt`) retain the legacy PromptRequest fallback.
                    if command_type_from_text(&text).as_deref() == Some("get_message") {
                        let event = ws_error_response(
                            command_id_from_text(&text),
                            "get_message",
                            format!("invalid request: {err}"),
                        );
                        let _ = command_event_tx.send(event).await;
                        continue;
                    }
                }
            }
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
            // Direct command response → WS
            event = command_event_rx.recv() => {
                match event {
                    Some(ev) => {
                        if let Ok(json) = serde_json::to_string(&ev) {
                            if socket.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }
            // Agent event → WS
            event = subscriber.recv() => {
                match event {
                    Some(ev) => {
                        // Direct get_message responses are returned by
                        // `gateway.send` above. The real UDS gateway also
                        // broadcasts them, so suppress that duplicate here.
                        if is_direct_ws_command_response(&ev) {
                            continue;
                        }
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

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
