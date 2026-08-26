use super::*;
use crate::interface::cli::ConfigError;
use tokio::io::AsyncWriteExt;

async fn stub_agent(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let socket_path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Keep the connection open briefly.
            let _ = stream.write_all(b"").await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });
    socket_path
}

#[tokio::test]
async fn bind_connects_agent_and_binds_port() {
    let dir = tempfile::tempdir().unwrap();
    let socket = stub_agent(&dir).await;
    let config = Config {
        socket,
        host: "127.0.0.1".into(),
        port: 0, // ephemeral
    };
    let (listener, _app) = bind(&config).await.expect("bind succeeds");
    // An ephemeral port was assigned.
    assert_ne!(listener.local_addr().unwrap().port(), 0);
}

#[tokio::test]
async fn bind_fails_when_agent_socket_missing() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        socket: dir.path().join("missing.sock"),
        host: "127.0.0.1".into(),
        port: 0,
    };
    let err = bind(&config).await.unwrap_err();
    assert!(matches!(err, ServerError::Connect(_)));
}

#[tokio::test]
async fn serve_runs_until_shutdown_signal() {
    let dir = tempfile::tempdir().unwrap();
    let socket = stub_agent(&dir).await;
    let config = Config {
        socket,
        host: "127.0.0.1".into(),
        port: 0,
    };
    let (listener, app) = bind(&config).await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        serve(listener, app, async {
            let _ = rx.await;
        })
        .await
    });

    // The server is live: /health responds.
    let body = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health request");
    assert!(body.status().is_success() || body.status().as_u16() == 503);

    // Trigger graceful shutdown and confirm the task returns Ok.
    tx.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("server shuts down")
        .expect("join");
    assert!(result.is_ok());
}

#[test]
fn server_error_display_is_informative() {
    assert!(
        ServerError::Connect("x".into())
            .to_string()
            .contains("quecto agent")
    );
    assert!(ServerError::Bind("y".into()).to_string().contains("bind"));
    assert!(
        ServerError::Config(ConfigError::MissingSocket)
            .to_string()
            .contains("configuration error")
    );
}

#[tokio::test]
async fn bind_reports_invalid_socket_address_as_config_error() {
    let dir = tempfile::tempdir().unwrap();
    let socket = stub_agent(&dir).await;
    let config = Config {
        socket,
        host: "not a host".into(),
        port: 8080,
    };

    let err = bind(&config).await.unwrap_err();
    assert!(matches!(
        err,
        ServerError::Config(ConfigError::InvalidPort(_))
    ));
}

#[tokio::test]
async fn bind_reports_tcp_bind_failure() {
    let dir = tempfile::tempdir().unwrap();
    let socket = stub_agent(&dir).await;
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    let config = Config {
        socket,
        host: "127.0.0.1".into(),
        port,
    };

    let err = bind(&config).await.unwrap_err();
    assert!(matches!(err, ServerError::Bind(_)));
}
