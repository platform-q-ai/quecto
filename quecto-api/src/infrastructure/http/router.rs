// HTTP router — maps HTTP/WebSocket endpoints to application use cases.

use std::sync::Arc;

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
