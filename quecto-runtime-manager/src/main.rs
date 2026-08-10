use quecto_runtime_manager::{
    application::{ManagerConfig, RuntimeRegistry},
    infrastructure::{AppState, ProductionRuntimeLifecycle, serve},
};
use reqwest::{Certificate, Client};
use std::{collections::HashSet, net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let host = std::env::var("RUNTIME_MANAGER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env_u16("RUNTIME_MANAGER_PORT", 8080);
    let addr: SocketAddr = format!("{host}:{port}").parse()?;

    let manager_token = std::env::var("RUNTIME_MANAGER_TOKEN")
        .ok()
        .map(|token| token.trim().replace(['\r', '\n'], ""))
        .filter(|token| !token.is_empty());

    let state = AppState {
        config: Arc::new(ManagerConfig {
            runtime_root: env_path("QUECTO_RUNTIME_ROOT", "/data/runtimes"),
            socket_root: env_path("QUECTO_SOCKET_ROOT", "/data/sockets"),
            api_port_base: env_u16("QUECTO_API_PORT_BASE", 21000),
            api_port_span: env_u16("QUECTO_API_PORT_SPAN", 2000),
            max_runtimes: env_usize("QUECTO_MAX_RUNTIMES", 24),
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
            credentials_secret_name: std::env::var("QUECTO_CREDENTIALS_SECRET_NAME")
                .unwrap_or_else(|_| "quecto-secrets".to_string()),
            manager_self_url: std::env::var("RUNTIME_MANAGER_SELF_URL")
                .unwrap_or_else(|_| "http://quecto-runtime-manager:8080".to_string()),
            manager_token: manager_token.clone(),
        }),
        registry: Arc::new(Mutex::new(RuntimeRegistry::default())),
        token: manager_token,
        http: http_client(),
        lifecycle: Arc::new(ProductionRuntimeLifecycle),
        pending_starts: Arc::new(Mutex::new(HashSet::new())),
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

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
