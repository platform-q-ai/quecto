use crate::{
    application::{
        ManagedRuntime, ManagerConfig, ManagerError, RuntimeRegistry, ensure_capacity,
        ensure_request_envelope,
    },
    domain::{EnsureRuntimeRequest, StopRuntimeResponse},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post},
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::json;
use std::{
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, process::Command, sync::Mutex, time::sleep};
use tokio_tungstenite::{connect_async, tungstenite};
use tracing::{info, warn};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ManagerConfig>,
    pub registry: Arc<Mutex<RuntimeRegistry>>,
    pub token: Option<String>,
    pub http: Client,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/runtimes/ensure", post(ensure_runtime))
        .route("/runtimes/:runtime_ref", delete(stop_runtime))
        .route("/runtimes/:runtime_ref/ws", get(proxy_ws))
        .route("/runtimes/:runtime_ref/*path", any(proxy_http))
        .with_state(state)
}

pub async fn serve(state: AppState, addr: SocketAddr) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router(state)).await
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let runtimes = state.registry.lock().await.active_count();
    Json(json!({ "healthy": true, "runtimes": runtimes }))
}

async fn ensure_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EnsureRuntimeRequest>,
) -> Response {
    if !authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let envelope = match ensure_request_envelope(&body) {
        Ok(envelope) => envelope,
        Err(ManagerError::InvalidRequest(reason)) => {
            return json_error(StatusCode::BAD_REQUEST, reason);
        }
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };

    let runtime_ref = envelope.runtime_ref.clone();
    {
        let mut registry = state.registry.lock().await;
        if let Some(runtime) = registry.get_mut(&runtime_ref) {
            runtime.touch();
            return (StatusCode::OK, Json(envelope)).into_response();
        }
        if let Err(error) = ensure_capacity(&mut registry, &state.config, &runtime_ref) {
            return json_error(StatusCode::SERVICE_UNAVAILABLE, error.to_string());
        }
    }

    let port = {
        let mut registry = state.registry.lock().await;
        match registry.allocate_port(&state.config, &runtime_ref) {
            Ok(port) => port,
            Err(error) => return json_error(StatusCode::SERVICE_UNAVAILABLE, error.to_string()),
        }
    };

    match start_runtime(&state.config, &body, &runtime_ref, port).await {
        Ok(runtime) => {
            state.registry.lock().await.insert(runtime);
            info!(%runtime_ref, "runtime started");
            (StatusCode::CREATED, Json(envelope)).into_response()
        }
        Err(error) => {
            state.registry.lock().await.release_port(port);
            json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    }
}

async fn stop_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(runtime_ref): Path<String>,
) -> Response {
    if !authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let stopped = state.registry.lock().await.stop(&runtime_ref);
    Json(StopRuntimeResponse {
        runtime_ref,
        status: "stopped".to_string(),
        stopped,
    })
    .into_response()
}

#[allow(clippy::too_many_arguments)]
async fn proxy_http(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((runtime_ref, path)): Path<(String, String)>,
    method: Method,
    body: Body,
) -> Response {
    if !authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let port = {
        let mut registry = state.registry.lock().await;
        let Some(runtime) = registry.get_mut(&runtime_ref) else {
            return json_error(StatusCode::NOT_FOUND, "runtime_not_found");
        };
        runtime.touch();
        runtime.port
    };

    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(error) => return json_error(StatusCode::BAD_GATEWAY, error.to_string()),
    };

    let url = format!("http://127.0.0.1:{port}/{path}");
    let mut request = state.http.request(method, url).body(body_bytes);
    for (key, value) in headers.iter() {
        if !is_hop_header(key.as_str()) {
            request = request.header(key, value);
        }
    }

    match request.send().await {
        Ok(response) => {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let bytes = response.bytes().await.unwrap_or_default();
            (status, bytes).into_response()
        }
        Err(error) => json_error(StatusCode::BAD_GATEWAY, error.to_string()),
    }
}

async fn proxy_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(runtime_ref): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    if !authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let port = {
        let mut registry = state.registry.lock().await;
        let Some(runtime) = registry.get_mut(&runtime_ref) else {
            return json_error(StatusCode::NOT_FOUND, "runtime_not_found");
        };
        runtime.touch();
        runtime.port
    };

    ws.on_upgrade(move |socket| async move {
        if let Err(error) = bridge_ws(socket, port).await {
            warn!(%error, "websocket proxy failed");
        }
    })
    .into_response()
}

async fn bridge_ws(
    client: WebSocket,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (backend, _) = connect_async(format!("ws://127.0.0.1:{port}/ws")).await?;
    let (mut client_tx, mut client_rx) = client.split();
    let (mut backend_tx, mut backend_rx) = backend.split();

    let client_to_backend = async {
        while let Some(Ok(message)) = client_rx.next().await {
            let message = match message {
                Message::Text(text) => tungstenite::Message::Text(text),
                Message::Binary(bytes) => tungstenite::Message::Binary(bytes),
                Message::Ping(bytes) => tungstenite::Message::Ping(bytes),
                Message::Pong(bytes) => tungstenite::Message::Pong(bytes),
                Message::Close(_) => break,
            };
            backend_tx.send(message).await?;
        }
        Ok::<(), tungstenite::Error>(())
    };

    let backend_to_client = async {
        while let Some(Ok(message)) = backend_rx.next().await {
            let message = match message {
                tungstenite::Message::Text(text) => Message::Text(text),
                tungstenite::Message::Binary(bytes) => Message::Binary(bytes),
                tungstenite::Message::Ping(bytes) => Message::Ping(bytes),
                tungstenite::Message::Pong(bytes) => Message::Pong(bytes),
                tungstenite::Message::Close(_) => break,
                tungstenite::Message::Frame(_) => continue,
            };
            client_tx.send(message).await?;
        }
        Ok::<(), axum::Error>(())
    };

    tokio::select! {
        _ = client_to_backend => {},
        _ = backend_to_client => {},
    }

    Ok(())
}

pub async fn start_runtime(
    config: &ManagerConfig,
    body: &EnsureRuntimeRequest,
    runtime_ref: &str,
    port: u16,
) -> Result<ManagedRuntime, ManagerError> {
    tokio::fs::create_dir_all(&config.runtime_root).await?;
    tokio::fs::create_dir_all(&config.socket_root).await?;
    let base_dir = config.runtime_root.join(runtime_ref);
    let workspace = base_dir.join("workspace");
    let socket_path = config.socket_root.join(format!("{runtime_ref}.sock"));
    tokio::fs::create_dir_all(&workspace).await?;
    seed_file(&config.seed_config_path, &base_dir.join("config.json")).await?;
    seed_file(
        &config.seed_credentials_path,
        &base_dir.join("credentials.json"),
    )
    .await?;
    let _ = tokio::fs::remove_file(&socket_path).await;

    let mut agent_args = vec![
        "agent".to_string(),
        "--mode".to_string(),
        "uds".to_string(),
        "--no-sandbox".to_string(),
        "--network".to_string(),
        "--socket".to_string(),
        socket_path.to_string_lossy().to_string(),
        "--session".to_string(),
        body.session_name.clone(),
        "--persist".to_string(),
    ];

    if let Ok(system) = tokio::fs::read_to_string(&config.system_prompt_path).await {
        agent_args.extend(["--system".to_string(), system]);
    }

    let agent = Command::new("quecto")
        .args(agent_args)
        .env("QUECTO_BASE_DIR", &base_dir)
        .env("QUECTO_AGENTS_DEFAULTS_WORKSPACE", &workspace)
        .spawn()?;

    wait_for_socket(&socket_path, Duration::from_secs(15)).await?;

    let api = Command::new("quecto-api")
        .args([
            "--socket",
            &socket_path.to_string_lossy(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .spawn()?;

    let mcp = if config.mcp_url.as_deref().is_some_and(|url| !url.is_empty())
        && config.mcp_token_path.exists()
    {
        Some(
            Command::new("quecto-mcp")
                .args([
                    "--socket",
                    &socket_path.to_string_lossy(),
                    "--mcp-url",
                    config.mcp_url.as_deref().unwrap_or_default(),
                    "--mcp-token-file",
                    &config.mcp_token_path.to_string_lossy(),
                    "--tool-prefix",
                    "community.",
                    "--tool-allowlist",
                    &config.mcp_allowlist,
                    "--register-timeout",
                    "10",
                    "--timeout",
                    "30",
                ])
                .spawn()?,
        )
    } else {
        None
    };

    Ok(ManagedRuntime {
        runtime_ref: runtime_ref.to_string(),
        session_name: body.session_name.clone(),
        session_key: body.session_key.clone(),
        base_dir,
        socket_path,
        port,
        agent: Some(agent),
        api: Some(api),
        mcp,
        last_used_at: Instant::now(),
    })
}

async fn seed_file(source: &PathBuf, target: &PathBuf) -> Result<(), ManagerError> {
    if !source.exists() || target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(source, target).await?;
    Ok(())
}

async fn wait_for_socket(path: &FsPath, timeout: Duration) -> Result<(), ManagerError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        sleep(Duration::from_millis(250)).await;
    }
    Err(ManagerError::RuntimeUnhealthy)
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = state.token.as_deref() else {
        return true;
    };
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {token}"))
        || headers
            .get("x-quecto-token")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == token)
}

fn is_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection" | "upgrade" | "proxy-connection" | "keep-alive" | "transfer-encoding"
    )
}

fn json_error(status: StatusCode, error: impl ToString) -> Response {
    (status, Json(json!({ "error": error.to_string() }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    fn test_state(tmp: &tempfile::TempDir, token: Option<String>) -> AppState {
        AppState {
            config: Arc::new(ManagerConfig {
                runtime_root: tmp.path().join("runtimes"),
                socket_root: tmp.path().join("sockets"),
                api_port_base: 21000,
                api_port_span: 10,
                idle: Duration::from_secs(60),
                max_runtimes: 50,
                system_prompt_path: tmp.path().join("system-prompt.txt"),
                seed_config_path: tmp.path().join("config.json"),
                seed_credentials_path: tmp.path().join("credentials.json"),
                mcp_url: None,
                mcp_allowlist: String::new(),
                mcp_token_path: tmp.path().join("mcp-token"),
            }),
            registry: Arc::new(Mutex::new(RuntimeRegistry::default())),
            token,
            http: Client::new(),
        }
    }

    fn fake_runtime(runtime_ref: &str, socket_path: PathBuf) -> ManagedRuntime {
        ManagedRuntime {
            runtime_ref: runtime_ref.to_string(),
            session_name: "session".to_string(),
            session_key: "key".to_string(),
            base_dir: PathBuf::from("/tmp/runtime"),
            socket_path,
            port: 21000,
            agent: None,
            api: None,
            mcp: None,
            last_used_at: Instant::now(),
        }
    }

    #[tokio::test]
    async fn health_reports_active_runtime_count() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp, None);

        let response = health(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn stop_runtime_requires_auth() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp, Some("secret".to_string()));
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/runtimes/cc-test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn stop_runtime_is_idempotent_and_removes_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("runtime.sock");
        tokio::fs::write(&socket, "stale").await.unwrap();
        let state = test_state(&tmp, Some("secret".to_string()));
        state
            .registry
            .lock()
            .await
            .insert(fake_runtime("cc-test", socket.clone()));
        let app = router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/runtimes/cc-test")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!socket.exists());
        assert_eq!(state.registry.lock().await.active_count(), 0);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["stopped"], true);

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/runtimes/cc-test")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["stopped"], false);
    }
}
