use crate::{
    application::{
        ManagedRuntime, ManagerConfig, ManagerError, RuntimeRegistry, ensure_request_envelope,
    },
    domain::{EnsureRuntimeRequest, StopRuntimeResponse},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        OriginalUri, Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post, put},
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, process::Command, sync::Mutex, time::sleep};

#[path = "infrastructure_lifecycle.rs"]
mod infrastructure_lifecycle;
#[cfg(test)]
pub(super) use infrastructure_lifecycle::BoxFutureResult;
use infrastructure_lifecycle::PendingStartGuard;
pub use infrastructure_lifecycle::{ProductionRuntimeLifecycle, RuntimeLifecycle};
use tokio_tungstenite::{connect_async, tungstenite};
use tracing::{info, warn};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ManagerConfig>,
    pub registry: Arc<Mutex<RuntimeRegistry>>,
    pub token: Option<String>,
    pub http: Client,
    pub lifecycle: Arc<dyn RuntimeLifecycle>,
    pub pending_starts: Arc<Mutex<HashSet<String>>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/runtimes/ensure", post(ensure_runtime))
        .route("/credentials", put(sync_credentials))
        .route("/runtimes/:runtime_ref", delete(stop_runtime))
        .route("/runtimes/:runtime_ref/status", get(runtime_status))
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

type ManagerResponseResult<T> = Result<T, Box<Response>>;

fn boxed_json_error(status: StatusCode, message: String) -> Box<Response> {
    Box::new(json_error(status, message))
}

enum EnsureStartClaim {
    Existing,
    Claimed(PendingStartGuard),
}

async fn claim_ensure_start(
    state: &AppState,
    runtime_ref: &str,
) -> ManagerResponseResult<EnsureStartClaim> {
    loop {
        {
            let mut registry = state.registry.lock().await;
            if let Some(runtime) = registry.get_mut(runtime_ref) {
                runtime.touch();
                return Ok(EnsureStartClaim::Existing);
            }
        }

        let should_start = {
            let registry = state.registry.lock().await;
            let mut pending = state.pending_starts.lock().await;
            let active_count = registry.active_count();
            if !pending.contains(runtime_ref)
                && active_count == 0
                && pending.len() >= state.config.max_runtimes
            {
                return Err(boxed_json_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ManagerError::RuntimeLimitReached.to_string(),
                ));
            }
            pending.insert(runtime_ref.to_string())
        };
        if should_start {
            return Ok(EnsureStartClaim::Claimed(PendingStartGuard::new(
                state.pending_starts.clone(),
                runtime_ref.to_string(),
            )));
        }

        sleep(Duration::from_millis(25)).await;
    }
}

async fn reap_for_pending_capacity(
    state: &AppState,
    pending_guard: &PendingStartGuard,
) -> ManagerResponseResult<Option<String>> {
    let mut registry = state.registry.lock().await;
    let pending_count = state.pending_starts.lock().await.len();
    if registry.active_count() + pending_count <= state.config.max_runtimes {
        return Ok(None);
    }

    let reaped_pod_name = registry.reap_one_oldest_pod();
    if reaped_pod_name.is_none()
        && registry.active_count() + pending_count > state.config.max_runtimes
    {
        pending_guard.release().await;
        return Err(boxed_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            ManagerError::RuntimeLimitReached.to_string(),
        ));
    }
    Ok(reaped_pod_name.flatten())
}

async fn allocate_runtime_port(
    state: &AppState,
    runtime_ref: &str,
    pending_guard: &PendingStartGuard,
) -> ManagerResponseResult<u16> {
    let mut registry = state.registry.lock().await;
    match registry.allocate_port(&state.config, runtime_ref) {
        Ok(port) => Ok(port),
        Err(error) => {
            pending_guard.release().await;
            Err(boxed_json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                error.to_string(),
            ))
        }
    }
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

    // Reject early if the socket path would exceed the OS UDS path limit —
    // otherwise the later bind() fails with a cryptic truncation error. Only the
    // process model binds a local `socket_root/{ref}.sock`; the pod model uses a
    // fixed `kubernetes://` socket, so the length guard doesn't apply there.
    if body.execution_model.as_deref() != Some("pod")
        && !crate::domain::socket_path_within_uds_limit(&state.config.socket_root, &runtime_ref)
    {
        return json_error(
            StatusCode::BAD_REQUEST,
            format!("socket path too long for runtime '{runtime_ref}' (exceeds the OS UDS limit)"),
        );
    }

    let pending_guard = match claim_ensure_start(&state, &runtime_ref).await {
        Ok(EnsureStartClaim::Existing) => return (StatusCode::OK, Json(envelope)).into_response(),
        Ok(EnsureStartClaim::Claimed(guard)) => guard,
        Err(response) => return *response,
    };

    let reaped_pod_name = match reap_for_pending_capacity(&state, &pending_guard).await {
        Ok(pod_name) => pod_name,
        Err(response) => return *response,
    };

    if let Some(pod_name) = reaped_pod_name {
        if let Err(error) = state
            .lifecycle
            .delete_runtime_pod(state.clone(), pod_name.clone())
            .await
        {
            warn!(%error, %pod_name, "failed to delete reaped runtime pod");
        }
    }

    let port = match allocate_runtime_port(&state, &runtime_ref, &pending_guard).await {
        Ok(port) => port,
        Err(response) => return *response,
    };

    match state
        .lifecycle
        .start_runtime(state.clone(), body, runtime_ref.clone(), port)
        .await
    {
        Ok(runtime) => {
            if runtime.runtime_ref != runtime_ref {
                state.registry.lock().await.release_port(port);
                pending_guard.release().await;
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "runtime lifecycle returned mismatched ref '{}'",
                        runtime.runtime_ref
                    ),
                );
            }
            state.registry.lock().await.insert(runtime);
            pending_guard.release().await;
            info!(%runtime_ref, "runtime started");
            (StatusCode::CREATED, Json(envelope)).into_response()
        }
        Err(error) => {
            state.registry.lock().await.release_port(port);
            pending_guard.release().await;
            json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    }
}

/// Sync a refreshed `credentials.json` from a runtime pod back into the shared
/// Kubernetes Secret.
///
/// A runtime pod refreshes its OAuth access token in-process and persists it to
/// its (now writable) local `credentials.json`, but that copy is lost when the
/// pod exits. Without writing the fresh token back to the Secret, every newly
/// spawned pod would start from the stale (expired) token baked into the Secret
/// and report "token expired" until the Secret is manually rotated. This endpoint
/// lets the pod push the refreshed credentials back so the next pod starts fresh.
async fn sync_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let Some(credentials_json) = body.get("credentials_json").and_then(Value::as_str) else {
        return json_error(StatusCode::BAD_REQUEST, "missing credentials_json");
    };

    // Validate it parses as JSON before persisting — never write a malformed
    // blob into the shared Secret.
    if serde_json::from_str::<Value>(credentials_json).is_err() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "credentials_json is not valid JSON",
        );
    }

    match state
        .lifecycle
        .sync_credentials(state.clone(), credentials_json.to_string())
        .await
    {
        Ok(()) => {
            info!("synced refreshed credentials into secret");
            (StatusCode::NO_CONTENT, ()).into_response()
        }
        Err(error) => {
            warn!(%error, "failed to sync credentials into secret");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    }
}

/// PATCH the `credentials.json` key of the shared credentials Secret with a
/// `application/merge-patch+json` request, leaving other keys untouched.
async fn patch_credentials_secret(
    state: &AppState,
    credentials_json: &str,
) -> Result<(), ManagerError> {
    use base64::Engine;

    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials_json.as_bytes());
    let secret = &state.config.credentials_secret_name;
    let url = kubernetes_url(&state.config, &format!("/secrets/{secret}"));
    let patch = json!({ "data": { "credentials.json": encoded } });

    let response = state
        .http
        .patch(url)
        .bearer_auth(kubernetes_token().await?)
        .header("content-type", "application/merge-patch+json")
        .json(&patch)
        .send()
        .await?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        warn!(%status, %body, "kubernetes secret patch rejected");
        Err(ManagerError::KubernetesApi(status.as_u16()))
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

    let pod_name = state
        .registry
        .lock()
        .await
        .get(&runtime_ref)
        .and_then(|runtime| runtime.pod_name.clone());
    let stopped = state.registry.lock().await.stop(&runtime_ref);

    if stopped {
        if let Some(pod_name) = pod_name {
            if let Err(error) = state
                .lifecycle
                .delete_runtime_pod(state.clone(), pod_name.clone())
                .await
            {
                warn!(%error, %pod_name, "failed to delete runtime pod");
            }
        }
    }

    Json(StopRuntimeResponse {
        runtime_ref,
        status: "stopped".to_string(),
        stopped,
    })
    .into_response()
}

async fn runtime_status(
    State(state): State<AppState>,
    Path(runtime_ref): Path<String>,
) -> Response {
    let pod_name = {
        let registry = state.registry.lock().await;
        registry
            .get(&runtime_ref)
            .and_then(|runtime| runtime.pod_name.clone())
    };

    let Some(pod_name) = pod_name else {
        return json_error(StatusCode::NOT_FOUND, "runtime_not_found");
    };

    match state
        .lifecycle
        .runtime_pod_status(state.clone(), pod_name)
        .await
    {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(error) => json_error(StatusCode::BAD_GATEWAY, error),
    }
}

async fn proxy_http(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((runtime_ref, path)): Path<(String, String)>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    body: Body,
) -> Response {
    if !authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let target_base = {
        let mut registry = state.registry.lock().await;
        let Some(runtime) = registry.get_mut(&runtime_ref) else {
            return json_error(StatusCode::NOT_FOUND, "runtime_not_found");
        };
        runtime.touch();
        runtime_target_base(runtime)
    };

    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(error) => return json_error(StatusCode::BAD_GATEWAY, error.to_string()),
    };

    let url = runtime_target_url(&target_base, &path, original_uri.query());
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

    let target_ws_url = {
        let mut registry = state.registry.lock().await;
        let Some(runtime) = registry.get_mut(&runtime_ref) else {
            return json_error(StatusCode::NOT_FOUND, "runtime_not_found");
        };
        runtime.touch();
        runtime_target_ws(runtime)
    };

    ws.on_upgrade(move |socket| async move {
        if let Err(error) = bridge_ws(socket, target_ws_url).await {
            warn!(%error, "websocket proxy failed");
        }
    })
    .into_response()
}

async fn bridge_ws(
    client: WebSocket,
    backend_url: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (backend, _) = connect_async(backend_url).await?;
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
    state: &AppState,
    body: &EnsureRuntimeRequest,
    runtime_ref: &str,
    port: u16,
) -> Result<ManagedRuntime, ManagerError> {
    if body.execution_model.as_deref() == Some("pod") {
        return start_pod_runtime(state, body, runtime_ref, port).await;
    }

    let config = &state.config;
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
        .current_dir(&workspace)
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
        .env("QUECTO_BASE_DIR", &base_dir)
        .env("QUECTO_SESSION_KEY", agent_session_key(&body.session_name))
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
                    "--tool-prefix",
                    "boards.",
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
        pod_name: None,
        pod_ip: None,
        last_used_at: Instant::now(),
    })
}

async fn start_pod_runtime(
    state: &AppState,
    body: &EnsureRuntimeRequest,
    runtime_ref: &str,
    port: u16,
) -> Result<ManagedRuntime, ManagerError> {
    let config = &state.config;
    let pod_name = runtime_pod_name(runtime_ref);
    let manifest = runtime_pod_manifest(config, body, runtime_ref, &pod_name);

    create_runtime_pod(state, &manifest).await?;
    let pod_ip = match wait_for_runtime_pod_ready(state, &pod_name, Duration::from_secs(90)).await {
        Ok(pod_ip) => pod_ip,
        Err(error) => {
            if let Err(cleanup_error) = delete_runtime_pod(state, &pod_name).await {
                warn!(%cleanup_error, %pod_name, "failed to delete unhealthy runtime pod");
            }
            return Err(error);
        }
    };

    Ok(ManagedRuntime {
        runtime_ref: runtime_ref.to_string(),
        session_name: body.session_name.clone(),
        session_key: body.session_key.clone(),
        base_dir: PathBuf::from(format!(
            "kubernetes://{}/{}",
            config.kubernetes_namespace, pod_name
        )),
        socket_path: PathBuf::from(format!("kubernetes://{pod_name}/shared/quecto.sock")),
        port,
        agent: None,
        api: None,
        mcp: None,
        pod_name: Some(pod_name),
        pod_ip: Some(pod_ip),
        last_used_at: Instant::now(),
    })
}

#[path = "infrastructure_k8s.rs"]
mod infrastructure_k8s;
use infrastructure_k8s::*;

fn kubernetes_url(config: &ManagerConfig, path: &str) -> String {
    let host = std::env::var("KUBERNETES_SERVICE_HOST")
        .unwrap_or_else(|_| "kubernetes.default.svc".to_string());
    let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".to_string());
    format!(
        "https://{host}:{port}/api/v1/namespaces/{}{}",
        config.kubernetes_namespace, path
    )
}

async fn kubernetes_token() -> Result<String, ManagerError> {
    Ok(
        tokio::fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/token")
            .await?
            .trim()
            .to_string(),
    )
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
#[path = "infrastructure_tests.rs"]
mod tests;
