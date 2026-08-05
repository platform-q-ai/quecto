use crate::domain::agent_launch_backend::ParentEndpoint;
use crate::domain::error::DomainError;
use std::path::Path;
use std::time::Duration;

pub async fn connect(endpoint: &ParentEndpoint) -> Result<tokio::net::UnixStream, DomainError> {
    match endpoint {
        ParentEndpoint::DirectUds(path) => connect_path(path).await,
        // #1310 proxy mechanics expose a host-side Unix socket endpoint for the
        // parent. Keep it typed as a proxy endpoint for registry/UI semantics,
        // but connect to the proxy socket itself rather than falling back to the
        // requested child UDS path.
        ParentEndpoint::Proxy(proxy) => connect_path(Path::new(proxy)).await,
    }
}

async fn connect_path(path: &Path) -> Result<tokio::net::UnixStream, DomainError> {
    tokio::net::UnixStream::connect(path).await.map_err(|e| {
        DomainError::Tool(format!(
            "connect to subagent at {} failed: {e}",
            path.display()
        ))
    })
}

pub async fn wait_ready(endpoint: &ParentEndpoint, timeout: Duration) -> Result<(), DomainError> {
    use tokio::time::Instant;
    let deadline = Instant::now() + timeout;
    let mut interval = tokio::time::interval_at(
        Instant::now() + Duration::from_millis(100),
        Duration::from_millis(100),
    );
    loop {
        interval.tick().await;
        if connect(endpoint).await.is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(DomainError::Tool(format!(
                "subagent {} endpoint did not become ready within {}s",
                endpoint.mode(),
                timeout.as_secs()
            )));
        }
    }
}

pub async fn send_command_with_timeout(
    endpoint: &ParentEndpoint,
    command: &str,
    response_timeout: Duration,
) -> Result<String, DomainError> {
    let stream = connect(endpoint).await?;
    super::subagent_registry::send_subagent_stream_command_with_timeout(
        stream,
        command,
        response_timeout,
    )
    .await
}
