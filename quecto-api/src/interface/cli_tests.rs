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
    let err = Config::parse(args(&["--socket", "/s", "--port", "notaport"]), no_env).unwrap_err();
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
