//! Interface layer — composition root / server bootstrap.
//!
//! Clean Architecture: this is where the concrete infrastructure adapters are
//! wired to the HTTP interface. The logic is kept out of `fn main` so the
//! bind/serve wiring is unit-testable (only the process-level `main` shim and
//! OS signal handling remain untested).

use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;

use crate::infrastructure::http::router::build_router;
use crate::infrastructure::uds::client::UdsGateway;
use crate::interface::cli::Config;

/// Errors raised while bringing the server up.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("configuration error: {0}")]
    Config(#[from] crate::interface::cli::ConfigError),

    #[error("failed to connect to quecto agent: {0}")]
    Connect(String),

    #[error("bind failed: {0}")]
    Bind(String),
}

/// Connect to the agent named by `config` and build the HTTP application,
/// returning a bound TCP listener plus the router ready to serve.
///
/// Separated from [`serve`] so tests can assert the wiring (address binding,
/// gateway connection) without blocking on `axum::serve`.
pub async fn bind(config: &Config) -> Result<(TcpListener, Router), ServerError> {
    let gateway = UdsGateway::connect(&config.socket)
        .await
        .map_err(|e| ServerError::Connect(e.to_string()))?;

    let addr: SocketAddr = config.socket_addr()?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| ServerError::Bind(e.to_string()))?;

    Ok((listener, build_router(gateway)))
}

/// Serve until the provided shutdown future resolves.
pub async fn serve<F>(listener: TcpListener, app: Router, shutdown: F) -> Result<(), std::io::Error>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
