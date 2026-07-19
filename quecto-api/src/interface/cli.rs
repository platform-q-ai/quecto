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
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_all_flags() {
        let cfg = Config::parse(
            args(&[
                "--socket",
                "/tmp/a.sock",
                "--host",
                "0.0.0.0",
                "--port",
                "9000",
            ]),
            no_env,
        )
        .unwrap();
        assert_eq!(cfg.socket, PathBuf::from("/tmp/a.sock"));
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 9000);
    }

    #[test]
    fn defaults_host_and_port() {
        let cfg = Config::parse(args(&["--socket", "/tmp/a.sock"]), no_env).unwrap();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 8080);
    }

    #[test]
    fn socket_falls_back_to_env() {
        let cfg = Config::parse(args(&[]), |k| {
            (k == "QUECTO_SOCKET").then(|| "/env/s.sock".to_string())
        })
        .unwrap();
        assert_eq!(cfg.socket, PathBuf::from("/env/s.sock"));
    }

    #[test]
    fn explicit_socket_overrides_env() {
        let cfg = Config::parse(args(&["--socket", "/flag.sock"]), |_| {
            Some("/env.sock".to_string())
        })
        .unwrap();
        assert_eq!(cfg.socket, PathBuf::from("/flag.sock"));
    }

    #[test]
    fn missing_socket_is_an_error() {
        let err = Config::parse(args(&[]), no_env).unwrap_err();
        assert_eq!(err, ConfigError::MissingSocket);
    }

    #[test]
    fn unknown_argument_is_an_error() {
        let err = Config::parse(args(&["--bogus"]), no_env).unwrap_err();
        assert_eq!(err, ConfigError::UnknownArgument("--bogus".to_string()));
    }

    #[test]
    fn invalid_port_is_an_error() {
        let err =
            Config::parse(args(&["--socket", "/s", "--port", "notaport"]), no_env).unwrap_err();
        assert_eq!(err, ConfigError::InvalidPort("notaport".to_string()));
    }

    #[test]
    fn missing_value_for_flag_is_an_error() {
        assert_eq!(
            Config::parse(args(&["--socket"]), no_env).unwrap_err(),
            ConfigError::MissingValue("--socket")
        );
        assert_eq!(
            Config::parse(args(&["--host"]), no_env).unwrap_err(),
            ConfigError::MissingValue("--host")
        );
        assert_eq!(
            Config::parse(args(&["--port"]), no_env).unwrap_err(),
            ConfigError::MissingValue("--port")
        );
    }

    #[test]
    fn resolves_socket_addr() {
        let cfg = Config::parse(args(&["--socket", "/s", "--port", "8081"]), no_env).unwrap();
        let addr = cfg.socket_addr().unwrap();
        assert_eq!(addr.port(), 8081);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn invalid_host_yields_addr_error() {
        let cfg = Config {
            socket: PathBuf::from("/s"),
            host: "not a host".to_string(),
            port: 80,
        };
        assert!(cfg.socket_addr().is_err());
    }

    #[test]
    fn error_messages_are_stable() {
        assert_eq!(
            ConfigError::MissingSocket.to_string(),
            "missing --socket / QUECTO_SOCKET"
        );
        assert_eq!(
            ConfigError::UnknownArgument("x".into()).to_string(),
            "unknown argument: x"
        );
        assert_eq!(
            ConfigError::MissingValue("--port").to_string(),
            "missing value for --port"
        );
        assert_eq!(
            ConfigError::InvalidPort("z".into()).to_string(),
            "invalid port: z"
        );
    }
}
