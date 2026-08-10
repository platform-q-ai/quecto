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
    ToolPolicyOperationPayload, ToolPolicyScopePayload,
};
use crate::application::use_cases;
use crate::domain::event::AgentEvent;
use std::collections::HashSet;

/// Shared application state, injected into every handler.
pub struct AppState<G: AgentGateway> {
    pub gateway: G,
}

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
    Sync {
        id: Option<String>,
        epoch: u64,
        #[serde(rename = "sinceRev")]
        since_rev: u64,
        agent_id: Option<String>,
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

fn direct_response_id(event: &AgentEvent) -> Option<&str> {
    match event {
        AgentEvent::Response { id: Some(id), .. } => Some(id.as_str()),
        _ => None,
    }
}

async fn send_ws_event(socket: &mut WebSocket, event: &AgentEvent) -> bool {
    match serde_json::to_string(event) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => true,
    }
}

fn command_string_from_text(text: &str, key: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get(key)?.as_str().map(ToOwned::to_owned))
}

fn command_type_from_text(text: &str) -> Option<String> {
    command_string_from_text(text, "type")
}

fn command_id_from_text(text: &str) -> Option<String> {
    command_string_from_text(text, "id")
}

fn event_response(
    result: Result<AgentEvent, crate::domain::error::ApiError>,
) -> axum::response::Response {
    match result {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
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

#[derive(Deserialize)]
struct MessageRequest {
    message: String,
}

async fn steer_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<MessageRequest>,
) -> impl IntoResponse {
    event_response(use_cases::steer::execute(&state.gateway, body.message).await)
}

async fn follow_up_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<MessageRequest>,
) -> impl IntoResponse {
    event_response(use_cases::follow_up::execute(&state.gateway, body.message).await)
}

async fn abort_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    event_response(use_cases::abort::execute(&state.gateway).await)
}

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

#[derive(Deserialize)]
struct SetEffortRequest {
    effort: String,
}

async fn set_effort_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<SetEffortRequest>,
) -> impl IntoResponse {
    event_response(use_cases::set_effort::execute(&state.gateway, body.effort).await)
}

async fn clear_history_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    event_response(use_cases::clear_history::execute(&state.gateway).await)
}

async fn subagents_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    event_response(use_cases::get_subagents::execute(&state.gateway).await)
}

async fn tools_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    event_response(use_cases::tools::catalogue(&state.gateway).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetToolPolicyRequest {
    #[serde(default)]
    mutations: Vec<ToolPolicyMutationPayload>,
    #[serde(default)]
    mode: ToolPolicyApplyModePayload,
    #[serde(default)]
    operation: ToolPolicyOperationPayload,
    #[serde(default)]
    unlisted_scope: Option<ToolPolicyScopePayload>,
}

async fn set_tool_policy_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
    Json(body): Json<SetToolPolicyRequest>,
) -> impl IntoResponse {
    match use_cases::set_tool_policy::execute(
        &state.gateway,
        body.mutations,
        body.mode,
        body.operation,
        body.unlisted_scope,
    )
    .await
    {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

async fn state_handler<G: AgentGateway>(
    State(state): State<Arc<AppState<G>>>,
) -> impl IntoResponse {
    match use_cases::get_state::execute(&state.gateway).await {
        Ok(event) => (StatusCode::OK, Json(serde_json::to_value(event).unwrap())).into_response(),
        Err(e) => api_error_response(e).into_response(),
    }
}

#[derive(Deserialize)]
struct MessagesQuery {
    before: Option<String>,
}

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
    agent_id: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
}

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

async fn ws_handler<G: AgentGateway + Clone + 'static>(
    State(state): State<Arc<AppState<G>>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(state, socket))
}

async fn handle_ws<G: AgentGateway + Clone>(state: Arc<AppState<G>>, mut socket: WebSocket) {
    let mut subscriber = match state.gateway.subscribe().await {
        Ok(sub) => sub,
        Err(_) => {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel::<String>(32);
    let (command_event_tx, mut command_event_rx) =
        tokio::sync::mpsc::channel::<(AgentEvent, Vec<String>)>(32);

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
                    let (event, mut suppress): (AgentEvent, Vec<String>) = match gateway
                        .send(AgentCommand::GetMessage {
                            message_id,
                            agent_id,
                            tool_call_id,
                            offset,
                            limit,
                        })
                        .await
                    {
                        Ok(event) => {
                            let suppress = direct_response_id(&event)
                                .into_iter()
                                .map(ToOwned::to_owned)
                                .collect();
                            (with_response_id(event, id), suppress)
                        }
                        Err(err) => (
                            ws_error_response(id, "get_message", err.to_string()),
                            Vec::new(),
                        ),
                    };
                    suppress.extend(direct_response_id(&event).map(ToOwned::to_owned));
                    let _ = command_event_tx.send((event, suppress)).await;
                    continue;
                }
                Ok(WsCommandRequest::Sync {
                    id,
                    epoch,
                    since_rev,
                    agent_id,
                }) => {
                    let (event, mut suppress): (AgentEvent, Vec<String>) =
                        match use_cases::sync_ledger::execute(
                            &gateway,
                            use_cases::sync_ledger::SyncInput {
                                epoch,
                                since_rev,
                                agent_id,
                            },
                        )
                        .await
                        {
                            Ok(event) => {
                                let suppress = direct_response_id(&event)
                                    .into_iter()
                                    .map(ToOwned::to_owned)
                                    .collect();
                                (with_response_id(event, id), suppress)
                            }
                            Err(err) => {
                                (ws_error_response(id, "sync", err.to_string()), Vec::new())
                            }
                        };
                    suppress.extend(direct_response_id(&event).map(ToOwned::to_owned));
                    let _ = command_event_tx.send((event, suppress)).await;
                    continue;
                }
                Err(err) => {
                    // Only malformed commands owned by this direct-command
                    // parser are errors here. Other typed payloads (notably
                    // `type: prompt`) retain the legacy PromptRequest fallback.
                    if matches!(
                        command_type_from_text(&text).as_deref(),
                        Some("get_message" | "sync")
                    ) {
                        let command =
                            command_type_from_text(&text).unwrap_or_else(|| "command".into());
                        let event = ws_error_response(
                            command_id_from_text(&text),
                            &command,
                            format!("invalid request: {err}"),
                        );
                        let suppress = direct_response_id(&event)
                            .into_iter()
                            .map(ToOwned::to_owned)
                            .collect();
                        let _ = command_event_tx.send((event, suppress)).await;
                        continue;
                    }
                }
            }
            if let Ok(req) = serde_json::from_str::<PromptRequest>(&text) {
                if !req.message.is_empty() {
                    let result = gateway
                        .send(AgentCommand::Prompt {
                            message: req.message,
                            streaming_behavior: req.streaming_behavior,
                        })
                        .await;
                    let (event, suppress) = match result {
                        Ok(event) => {
                            let suppress = direct_response_id(&event)
                                .map(str::to_owned)
                                .into_iter()
                                .collect();
                            (event, suppress)
                        }
                        Err(err) => (
                            ws_error_response(
                                command_id_from_text(&text),
                                "prompt",
                                err.to_string(),
                            ),
                            Vec::new(),
                        ),
                    };
                    let _ = command_event_tx.send((event, suppress)).await;
                }
            }
        }
    });

    let mut direct_response_ids = HashSet::<String>::new();

    loop {
        tokio::select! {
            event = command_event_rx.recv() => {
                match event {
                    Some((ev, suppress_ids)) => {
                        direct_response_ids.extend(suppress_ids);
                        if !send_ws_event(&mut socket, &ev).await {
                            break;
                        }
                    }
                    None => break,
                }
            }
            event = subscriber.recv() => {
                match event {
                    Some(ev) => {
                        // Direct command responses are returned by
                        // `gateway.send` above. The real UDS gateway also
                        // broadcasts them, so suppress only the correlated
                        // duplicate already returned on this WebSocket.
                        if let Some(id) = direct_response_id(&ev) {
                            if direct_response_ids.remove(id) {
                                continue;
                            }

                            // UDS resolves send() and broadcasts back-to-back; if this
                            // broadcast wins first, let the direct channel catch up.
                            match tokio::time::timeout(
                                std::time::Duration::from_millis(10),
                                command_event_rx.recv(),
                            )
                            .await
                            {
                                Ok(Some((direct_ev, suppress_ids))) => {
                                    direct_response_ids.extend(suppress_ids);
                                    if direct_response_ids.remove(id) {
                                        if !send_ws_event(&mut socket, &direct_ev).await {
                                            break;
                                        }
                                        continue;
                                    }
                                    if !send_ws_event(&mut socket, &direct_ev).await {
                                        break;
                                    }
                                }
                                Ok(None) => break,
                                Err(_) => {}
                            }
                        }
                        if !send_ws_event(&mut socket, &ev).await {
                            break;
                        }
                    }
                    None => {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let text = text.to_string();
                        let _ = incoming_tx.send(text).await;
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
