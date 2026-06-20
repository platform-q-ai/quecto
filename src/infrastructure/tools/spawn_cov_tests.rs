//! Coverage-focused unit tests for `spawn.rs`.
//!
//! These exercise pure helpers and the local UDS IPC helpers (`wait_for_socket`,
//! `send_initial_prompt`) without ever spawning a real `quecto` subprocess. The
//! full `launch_uds_agent` path requires a real child process and is covered by
//! BDD/integration tests instead.

use super::*;

// --- registry() accessor ---

#[test]
fn registry_accessor_reflects_shared_state() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let tool = SpawnTool::new(vec![], true).with_registry(registry.clone());
    registry.lock().unwrap().insert(
        "probe".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/probe.sock"), 7),
    );
    assert!(tool.registry().lock().unwrap().contains_key("probe"));
}

// --- inherited_runtime_config_path() ---

#[test]
fn inherited_runtime_config_path_is_callable() {
    // With the env var unset (the usual test environment) this returns None;
    // either way it must not panic and yields an Option<PathBuf>.
    let result = inherited_runtime_config_path();
    if std::env::var("QUECTO_RUNTIME_CONFIG_PATH").is_err() {
        assert!(result.is_none());
    }
}

// --- write_private_new error branch (non-AlreadyExists) ---

#[test]
fn write_private_new_propagates_non_already_exists_error() {
    let dir = tempfile::TempDir::new().unwrap();
    // Parent directory does not exist -> NotFound, which is neither a success
    // nor AlreadyExists, so the error is returned directly.
    let path = dir.path().join("missing-subdir").join("wf.json");
    let err = write_private_new(&path, b"data").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "got: {err}");
}

// --- wait_for_socket: success once a listener is bound ---

#[tokio::test]
async fn wait_for_socket_returns_ok_when_socket_is_connectable() {
    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("ready.sock");
    let _listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    let tool = SpawnTool::new(vec![], true);
    tool.wait_for_socket(&socket_path)
        .await
        .expect("listener is bound, so the socket should be connectable");
}

// --- send_initial_prompt: error when nothing is listening ---

#[tokio::test]
async fn send_initial_prompt_errors_when_socket_absent() {
    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("nope.sock");
    let tool = SpawnTool::new(vec![], true);
    let err = tool
        .send_initial_prompt(&socket_path, "hi")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("failed to connect to subagent"),
        "got: {err}"
    );
}

// --- send_initial_prompt: success path writes and shuts down ---

#[tokio::test]
async fn send_initial_prompt_writes_to_listening_socket() {
    use tokio::io::AsyncReadExt;
    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("live.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    let accept = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).await.unwrap();
        buf
    });

    let tool = SpawnTool::new(vec![], true);
    tool.send_initial_prompt(&socket_path, "do-the-thing")
        .await
        .expect("send to a bound listener should succeed");

    let received = accept.await.unwrap();
    assert!(received.contains("do-the-thing"), "got: {received}");
    assert!(received.contains("\"type\":\"prompt\""), "got: {received}");
}
