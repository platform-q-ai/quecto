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

#[test]
fn effective_config_path_prefers_explicit_over_inherited() {
    let explicit = PathBuf::from("explicit.toml");
    let inherited = PathBuf::from("inherited.toml");
    assert_eq!(
        effective_config_path(Some(&explicit), Some(inherited.clone())).as_deref(),
        Some(explicit.as_path())
    );
    assert_eq!(
        effective_config_path(None, Some(inherited.clone())).as_deref(),
        Some(inherited.as_path())
    );
    assert!(effective_config_path(None, None).is_none());
}

#[test]
fn parse_disable_tools_rejects_malformed_values() {
    let err = parse_disable_tools(&serde_json::json!({"read_only":"yes"})).unwrap_err();
    assert_eq!(err, "read_only must be a boolean");
    let err = parse_disable_tools(&serde_json::json!({"disable_tools":"write"})).unwrap_err();
    assert_eq!(err, "disable_tools must be an array of tool names");
    let err = parse_disable_tools(&serde_json::json!({"disable_tools":["write",7]})).unwrap_err();
    assert_eq!(err, "disable_tools entries must be strings (tool names)");
}

#[test]
fn parse_disable_tools_read_only_first_and_deduped() {
    let tools = parse_disable_tools(
        &serde_json::json!({"read_only":true,"disable_tools":["edit","grep","write"]}),
    )
    .unwrap();
    assert_eq!(tools, vec!["write", "edit", "grep"]);
}

#[test]
fn validate_config_path_rejects_parent_dir_component() {
    let err = validate_config_path("configs/../secret.toml").unwrap_err();
    assert!(err.contains("contains '..'"));
    assert_eq!(
        validate_config_path("configs/child.toml").unwrap(),
        PathBuf::from("configs/child.toml")
    );
}

#[tokio::test]
async fn execute_parse_error_returns_llm_addressable_tool_error() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool.execute(r#"{"read_only":"yes"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("read_only must be a boolean"));
}

#[tokio::test]
async fn stub_spawn_duplicate_id_replaces_existing_entry_without_panic() {
    let tool = SpawnTool::new(vec![], true);
    tool.execute(r#"{"agent_id":"dup"}"#).await.unwrap();
    tool.execute(r#"{"agent_id":"dup","task":"now busy"}"#)
        .await
        .unwrap();
    let registry = tool.registry.lock().unwrap();
    assert_eq!(registry.len(), 1);
    assert_eq!(
        registry.get("dup").unwrap().status,
        SubagentStatus::Starting
    );
}

#[tokio::test]
async fn launch_uds_agent_duplicate_id_fails_before_spawning() {
    let dir = tempfile::tempdir().unwrap();
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    registry.lock().unwrap().insert(
        "taken".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/taken.sock"), 0),
    );
    let tool = SpawnTool::with_base_dir(vec![], true, dir.path().to_path_buf())
        .with_socket_dir(dir.path().to_path_buf())
        .with_registry(registry);
    let cfg = tool.parse_args(r#"{"agent_id":"taken"}"#).unwrap();
    let result = tool.launch_uds_agent(&cfg).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("already running"));
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
