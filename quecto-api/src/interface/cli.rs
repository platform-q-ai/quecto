//! Interface layer — CLI configuration parsing.
//!
//! Clean Architecture: the interface layer adapts the outside world (process
//! arguments, environment) into a validated [`Config`] value object, then hands
//! control to the composition root. All parsing lives here (not in `main`) so it
//! is unit-testable without spawning a process.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Validated startup configuration for the API gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Path to the quecto agent's Unix domain socket.
    pub socket: PathBuf,
    /// Host interface to bind the HTTP server on.
    pub host: String,
    /// TCP port to bind the HTTP server on.
    pub port: u16,
}

/// Errors that can occur while parsing CLI configuration.
///
/// Transport-agnostic and free of `process::exit` so callers (and tests) decide
/// how to react.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("unknown argument: {0}")]
    UnknownArgument(String),

    #[error("missing value for {0}")]
    MissingValue(&'static str),

    #[error("invalid port: {0}")]
    InvalidPort(String),

    #[error("missing --socket / QUECTO_SOCKET")]
    MissingSocket,
}

impl Config {
    /// Parse configuration from an argument iterator (excluding argv[0]) and an
    /// environment lookup function.
    ///
    /// The env lookup is injected so tests are hermetic — they never touch the
    /// real process environment.
    pub fn parse<I, F>(args: I, env: F) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = String>,
        F: Fn(&str) -> Option<String>,
    {
        let mut socket: Option<PathBuf> = None;
        let mut host = "127.0.0.1".to_string();
        let mut port: u16 = 8080;

        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--socket" => {
                    socket = Some(PathBuf::from(
                        args.next().ok_or(ConfigError::MissingValue("--socket"))?,
                    ));
                }
                "--host" => {
                    host = args.next().ok_or(ConfigError::MissingValue("--host"))?;
                }
                "--port" => {
                    let raw = args.next().ok_or(ConfigError::MissingValue("--port"))?;
                    port = raw.parse().map_err(|_| ConfigError::InvalidPort(raw))?;
                }
                other => return Err(ConfigError::UnknownArgument(other.to_string())),
            }
        }

        let socket = socket
            .or_else(|| env("QUECTO_SOCKET").map(PathBuf::from))
            .ok_or(ConfigError::MissingSocket)?;

        Ok(Config { socket, host, port })
    }

    /// Resolve the bound socket address (`host:port`).
    pub fn socket_addr(&self) -> Result<SocketAddr, ConfigError> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|_| ConfigError::InvalidPort(format!("{}:{}", self.host, self.port)))
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
