use quecto_runtime_manager::{
    application::{ManagerConfig, RuntimeRegistry},
    infrastructure::{AppState, serve},
};
use reqwest::{Certificate, Client};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let host = std::env::var("RUNTIME_MANAGER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env_u16("RUNTIME_MANAGER_PORT", 8080);
    let addr: SocketAddr = format!("{host}:{port}").parse()?;

    let state = AppState {
        config: Arc::new(ManagerConfig {
            runtime_root: env_path("QUECTO_RUNTIME_ROOT", "/data/runtimes"),
            socket_root: env_path("QUECTO_SOCKET_ROOT", "/data/sockets"),
            api_port_base: env_u16("QUECTO_API_PORT_BASE", 21000),
            api_port_span: env_u16("QUECTO_API_PORT_SPAN", 2000),
            idle: Duration::from_millis(env_u64("QUECTO_RUNTIME_IDLE_MS", 1_800_000)),
            max_runtimes: env_usize("QUECTO_MAX_RUNTIMES", 24),
            max_context_tokens: env_usize("QUECTO_MAX_CONTEXT_TOKENS", 250_000),
            disable_default_workflows: env_bool("QUECTO_DISABLE_DEFAULT_WORKFLOWS", false),
            system_prompt_path: env_path(
                "QUECTO_SYSTEM_PROMPT_PATH",
                "/etc/quecto/system-prompt.txt",
            ),
            seed_config_path: env_path("QUECTO_CONFIG_PATH", "/etc/quecto/config.json"),
            seed_credentials_path: env_path(
                "QUECTO_CREDENTIALS_PATH",
                "/etc/quecto/credentials.json",
            ),
            mcp_url: std::env::var("MCP_URL").ok(),
            mcp_allowlist: std::env::var("MCP_ALLOWLIST").unwrap_or_default(),
            mcp_token_path: env_path("MCP_TOKEN_PATH", "/etc/quecto/mcp-token"),
            kubernetes_namespace: std::env::var("KUBERNETES_NAMESPACE")
                .unwrap_or_else(|_| "apps".to_string()),
            pod_image: std::env::var("QUECTO_RUNTIME_POD_IMAGE")
                .unwrap_or_else(|_| "ghcr.io/platform-q-ai/quecto:latest".to_string()),
            pod_pull_secret: std::env::var("QUECTO_RUNTIME_POD_PULL_SECRET")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        }),
        registry: Arc::new(Mutex::new(RuntimeRegistry::default())),
        token: std::env::var("RUNTIME_MANAGER_TOKEN")
            .ok()
            .map(|token| token.trim().replace(['\r', '\n'], ""))
            .filter(|token| !token.is_empty()),
        http: http_client(),
    };

    serve(state, addr).await?;
    Ok(())
}

fn http_client() -> Client {
    let mut builder = Client::builder();

    if let Ok(ca_pem) = std::fs::read("/var/run/secrets/kubernetes.io/serviceaccount/ca.crt") {
        if let Ok(cert) = Certificate::from_pem(&ca_pem) {
            builder = builder.add_root_certificate(cert);
        }
    }

    builder.build().unwrap_or_else(|_| Client::new())
}

fn env_path(key: &str, default: &str) -> PathBuf {
    PathBuf::from(std::env::var(key).unwrap_or_else(|_| default.to_string()))
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}
