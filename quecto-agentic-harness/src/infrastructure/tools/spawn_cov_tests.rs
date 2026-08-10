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
#[serial_test::serial]
fn inherited_runtime_config_path_reads_non_empty_and_ignores_empty() {
    let old = std::env::var_os("QUECTO_RUNTIME_CONFIG_PATH");
    // Serialized by #[serial_test::serial]: no other test in this binary runs
    // concurrently while the process-wide env is mutated.
    // SAFETY: no concurrent env readers; the value is restored before return.
    unsafe {
        std::env::set_var("QUECTO_RUNTIME_CONFIG_PATH", "  ");
    }
    assert!(inherited_runtime_config_path().is_none());

    // SAFETY: See note above; serialized by #[serial_test::serial].
    unsafe {
        std::env::set_var("QUECTO_RUNTIME_CONFIG_PATH", "runtime.toml");
    }
    assert_eq!(
        inherited_runtime_config_path().as_deref(),
        Some(std::path::Path::new("runtime.toml"))
    );

    match old {
        Some(value) => {
            // SAFETY: Restores the process environment value saved at test entry.
            unsafe {
                std::env::set_var("QUECTO_RUNTIME_CONFIG_PATH", value);
            }
        }
        None => {
            // SAFETY: Restores the absence of the process environment value saved at test entry.
            unsafe {
                std::env::remove_var("QUECTO_RUNTIME_CONFIG_PATH");
            }
        }
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

#[test]
fn parse_args_accepts_all_optional_spawn_fields() {
    let tool = SpawnTool::new(vec!["allowed".to_string()], false);
    let cfg = tool
        .parse_args(
            r#"{
                "agent_id":"allowed",
                "task":"do work",
                "system":"be useful",
                "config":"configs/child.toml",
                "workflow":true,
                "workflow_guards":true,
                "model":"openai/gpt-5",
                "effort":"high",
                "read_only":true,
                "disable_tools":["grep"]
            }"#,
        )
        .unwrap();

    assert_eq!(cfg.agent_id.as_deref(), Some("allowed"));
    assert_eq!(cfg.task.as_deref(), Some("do work"));
    assert_eq!(cfg.system.as_deref(), Some("be useful"));
    assert_eq!(
        cfg.config_path.as_deref(),
        Some(std::path::Path::new("configs/child.toml"))
    );
    assert!(cfg.workflow);
    assert!(cfg.workflow_guards);
    assert_eq!(cfg.model.as_deref(), Some("openai/gpt-5"));
    assert_eq!(cfg.effort.as_deref(), Some("high"));
    assert_eq!(cfg.disable_tools, vec!["write", "edit", "grep"]);
    assert!(cfg.read_only);
    assert!(!cfg.restrict_to_workspace);
}

#[test]
fn parse_args_rejects_bad_json_config_path_and_disable_tool_shapes() {
    let tool = SpawnTool::new(vec![], true);

    let err = tool.parse_args("{").unwrap_err();
    assert!(err.contains("invalid JSON"), "{err}");

    let err = tool
        .parse_args(r#"{"config":"../secret.toml"}"#)
        .unwrap_err();
    assert!(err.contains("contains '..'"), "{err}");

    let err = tool.parse_args(r#"{"disable_tools":"write"}"#).unwrap_err();
    assert!(err.contains("disable_tools must be an array"), "{err}");

    let err = tool
        .parse_args(r#"{"disable_tools":["write",7]}"#)
        .unwrap_err();
    assert!(err.contains("entries must be strings"), "{err}");
}

#[test]
fn parse_args_rejects_specific_invalid_fields() {
    let tool = SpawnTool::new(vec!["ok".to_string()], true);
    let err = tool.parse_args(r#"{"agent_id":"bad space"}"#).unwrap_err();
    assert!(err.contains("invalid") || err.contains("agent_id"), "{err}");

    let err = tool
        .parse_args(r#"{"agent_id":"not-allowed"}"#)
        .unwrap_err();
    assert!(
        err.contains("not allowed") || err.contains("not-allowed"),
        "{err}"
    );

    let err = tool.parse_args(r#"{"workflow_guards":true}"#).unwrap_err();
    assert_eq!(err, "workflow_guards requires workflow to also be true");

    let err = tool.parse_args(r#"{"provider":"openai"}"#).unwrap_err();
    assert!(err.contains("invalid model"), "{err}");

    let err = tool.parse_args(r#"{"effort":"extreme"}"#).unwrap_err();
    assert!(
        err.contains("invalid effort") || err.contains("effort"),
        "{err}"
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
async fn stub_spawn_duplicate_id_rejects_live_duplicate_without_panic() {
    let tool = SpawnTool::new(vec![], true);
    tool.execute(r#"{"agent_id":"dup"}"#).await.unwrap();
    let duplicate = tool
        .execute(r#"{"agent_id":"dup","task":"now busy"}"#)
        .await
        .unwrap();
    let registry = tool.registry.lock().unwrap();
    assert_eq!(registry.len(), 1);
    assert!(duplicate.is_error);
    assert!(
        duplicate
            .content
            .contains("duplicate live subagent display label 'dup'")
    );
}

#[tokio::test]
async fn register_and_broadcast_sends_state_changed_event() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (tx, mut rx) = tokio::sync::broadcast::channel(4);
    let cfg = SpawnTool::new(vec![], true)
        .parse_args(r#"{"agent_id":"child","read_only":true}"#)
        .unwrap();
    let entry = initial_registry_entry(InitialRegistryEntrySpec {
        agent_uuid: crate::domain::ids::AgentUuid::new("00000000-0000-4000-8000-000000000104"),
        display_name: "child".to_string(),
        socket_path: PathBuf::from("/tmp/child.sock"),
        pid: 123,
        parent_id: Some("parent".to_string()),
        config: &cfg,
        exit_signal_tx: None,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environment_registry: None,
        environment_ref: None,
        process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
    });

    register_and_broadcast(&registry, Some(&tx), "child", entry).unwrap();

    assert!(
        registry
            .lock()
            .unwrap()
            .values()
            .any(|e| e.display_name == "child")
    );
    let event = rx.recv().await.unwrap();
    assert!(event.contains("state_changed"), "{event}");
    assert!(event.contains("child"), "{event}");
}

#[test]
fn shutdown_all_clears_the_registry() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.spawn(async {
        std::future::pending::<()>().await;
    });
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/no-pid.sock"), 0);
    entry.monitor_handle = Some(Arc::new(handle));
    registry.lock().unwrap().insert("idle".to_string(), entry);

    shutdown_all(&registry);

    assert!(registry.lock().unwrap().is_empty());
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
    assert!(
        result
            .content
            .contains("duplicate live subagent display label")
    );
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

#[tokio::test]
async fn wait_for_socket_or_child_exit_reports_pre_ready_exit() {
    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("never-ready.sock");
    let mut child = tokio::process::Command::new("/usr/bin/false")
        .spawn()
        .expect("test helper process should spawn");
    let tool = SpawnTool::new(vec![], true);

    let err = tool
        .wait_for_socket_or_child_exit(&socket_path, &mut child)
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("subagent exited before socket ready"),
        "got: {err}"
    );
}

// --- send_initial_prompt: error when nothing is listening ---

#[tokio::test]
async fn send_initial_prompt_errors_when_socket_absent() {
    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("nope.sock");
    let tool = SpawnTool::new(vec![], true);
    let err = tool
        .send_initial_prompt_for_test(&socket_path, "hi")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("connect to subagent"),
        "got: {err}"
    );
}

// --- send_initial_prompt: success path sends an accepted framed command ---

#[tokio::test]
async fn send_initial_prompt_writes_to_listening_socket() {
    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("live.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = tokio::io::BufReader::new(stream);
        let payload =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .unwrap()
                .unwrap();
        let text = String::from_utf8_lossy(&payload).to_string();
        let sent: serde_json::Value = serde_json::from_str(&text).unwrap();
        let ack = serde_json::json!({
            "type":"response",
            "command":"prompt",
            "id": sent["id"].clone(),
            "success":true
        })
        .to_string();
        quecto_line_io::write_frame(
            reader.get_mut(),
            ack.as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
        text
    });

    let tool = SpawnTool::new(vec![], true);
    tool.send_initial_prompt_for_test(&socket_path, "do-the-thing")
        .await
        .expect("send to a bound listener should succeed");

    let received = accept.await.unwrap();
    assert!(received.contains("do-the-thing"), "got: {received}");
    assert!(received.contains("\"type\":\"prompt\""), "got: {received}");
}

#[test]
fn parse_args_workflow_spec_null_and_scalar_paths() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool.parse_args(r#"{"workflow_spec":null}"#).unwrap();
    assert!(cfg.workflow_spec.is_none());

    let err = tool.parse_args(r#"{"workflow_spec":42}"#).unwrap_err();
    assert!(err.contains("invalid workflow_spec"), "{err}");
}

fn poison_registry(registry: &SubagentRegistry) {
    let cloned = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = cloned.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err(), "registry should be poisoned");
}

#[tokio::test]
async fn register_and_broadcast_closed_receiver_still_inserts_entry() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (tx, rx) = tokio::sync::broadcast::channel(1);
    drop(rx);
    let cfg = SpawnTool::new(vec![], true)
        .parse_args(r#"{"agent_id":"closed"}"#)
        .unwrap();
    let entry = initial_registry_entry(InitialRegistryEntrySpec {
        agent_uuid: crate::domain::ids::AgentUuid::new("00000000-0000-4000-8000-000000000105"),
        display_name: "closed".to_string(),
        socket_path: PathBuf::from("/tmp/closed.sock"),
        pid: 0,
        parent_id: None,
        config: &cfg,
        exit_signal_tx: None,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environment_registry: None,
        environment_ref: None,
        process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
    });

    register_and_broadcast(&registry, Some(&tx), "closed", entry).unwrap();

    assert!(
        registry
            .lock()
            .unwrap()
            .values()
            .any(|e| e.display_name == "closed")
    );
}

#[tokio::test]
async fn spawn_registry_poison_recovery_paths_do_not_drop_entries() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let cfg = SpawnTool::new(vec![], true)
        .parse_args(r#"{"agent_id":"poison","read_only":true}"#)
        .unwrap();
    poison_registry(&registry);

    register_and_broadcast(
        &registry,
        None,
        "poison",
        initial_registry_entry(InitialRegistryEntrySpec {
            agent_uuid: crate::domain::ids::AgentUuid::new("00000000-0000-4000-8000-000000000106"),
            display_name: "poison".to_string(),
            socket_path: PathBuf::from("/tmp/poison.sock"),
            pid: 0,
            parent_id: None,
            config: &cfg,
            exit_signal_tx: None,
            cleanup_environment_id: None,
            cleanup_argv: Vec::new(),
            environment_registry: None,
            environment_ref: None,
            process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
        }),
    )
    .unwrap();
    assert!(
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|e| e.display_name == "poison")
    );

    shutdown_all(&registry);
    assert!(
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    );
}

#[tokio::test]
async fn launch_uds_agent_rejects_an_oversized_workflow_spec_before_spawning() {
    use crate::domain::workflow::{WorkflowSpec, WorkflowTemplate, WorkflowTemplateStep};

    // The spec is forwarded to the child as a file; an unbounded one would let a
    // caller write an arbitrarily large attacker-controlled file into socket_dir.
    // The size check must therefore reject before any file is created or any
    // child process is launched.
    let dir = tempfile::tempdir().expect("tempdir");
    let tool = SpawnTool::new(vec![], true).with_socket_dir(dir.path().to_path_buf());

    let huge = "g".repeat(crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES + 1);
    let spec = WorkflowSpec {
        template: WorkflowTemplate {
            id: "oversized".into(),
            label: "Oversized".into(),
            description: "spec exceeding the byte cap".into(),
            when_to_use: None,
            steps: vec![WorkflowTemplateStep {
                key: "step".into(),
                label: "Step".into(),
                phase: "phase".into(),
                guidance: Some(huge),
            }],
            guards: vec![],
        },
    };

    let mut config = tool
        .parse_args(r#"{"task":"go"}"#)
        .expect("baseline args parse");
    config.workflow_spec = Some(spec);

    let err = tool
        .launch_uds_agent(&config)
        .await
        .expect_err("an oversized workflow spec must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("workflow spec too large"),
        "expected the size-cap error, got: {msg}"
    );

    // Nothing was written to the socket dir before the rejection.
    let leaked: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read socket dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        leaked.is_empty(),
        "spec file leaked before rejection: {leaked:?}"
    );
}

#[test]
fn parse_args_provider_model_id_form_sets_model() {
    let tool = SpawnTool::new(vec![], true);

    let cfg = tool
        .parse_args(r#"{"provider":"openai","model_id":"gpt-5"}"#)
        .expect("provider/model_id form should parse");

    assert_eq!(cfg.model.as_deref(), Some("openai/gpt-5"));
}

#[tokio::test]
async fn launch_uds_agent_duplicate_with_poisoned_registry_recovers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    registry.lock().unwrap().insert(
        "taken".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/taken.sock"), 0),
    );
    poison_registry(&registry);

    let tool = SpawnTool::with_base_dir(vec![], true, dir.path().to_path_buf())
        .with_socket_dir(dir.path().to_path_buf())
        .with_registry(registry);
    let cfg = tool.parse_args(r#"{"agent_id":"taken"}"#).unwrap();

    let result = tool.launch_uds_agent(&cfg).await.unwrap();

    assert!(result.is_error);
    assert!(
        result
            .content
            .contains("duplicate live subagent display label"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn launch_uds_agent_maps_workflow_spec_write_failure() {
    use crate::domain::workflow::{WorkflowSpec, WorkflowTemplate, WorkflowTemplateStep};

    let dir = tempfile::tempdir().expect("tempdir");
    // Missing socket directory: writing the by-value workflow spec fails before
    // resolving or spawning any child binary.
    let missing_socket_dir = dir.path().join("missing-socket-dir");
    let tool = SpawnTool::with_base_dir(vec![], true, dir.path().to_path_buf())
        .with_socket_dir(missing_socket_dir);

    let mut cfg = tool.parse_args(r#"{"agent_id":"wf-child"}"#).unwrap();
    cfg.workflow_spec = Some(WorkflowSpec {
        template: WorkflowTemplate {
            id: "wf".into(),
            label: "Workflow".into(),
            description: "small valid spec".into(),
            when_to_use: None,
            steps: vec![WorkflowTemplateStep {
                key: "step".into(),
                label: "Step".into(),
                phase: "phase".into(),
                guidance: None,
            }],
            guards: vec![],
        },
    });

    let err = tool
        .launch_uds_agent(&cfg)
        .await
        .expect_err("missing socket dir must map spec write failure");
    let msg = err.to_string();
    assert!(msg.contains("failed to write workflow spec"), "got: {msg}");
}

#[tokio::test]
async fn launch_uds_agent_uses_uuid_not_display_label_for_socket_and_session_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let child = dir.path().join("fake-child.py");
    let args_file = dir.path().join("args.json");
    std::fs::write(
        &child,
        format!(
            r#"#!/usr/bin/env python3
import json, os, socket, sys, time
with open({args_file:?}, "w") as f:
    json.dump(sys.argv[1:], f)
sock_path = sys.argv[sys.argv.index("--socket") + 1]
try:
    os.unlink(sock_path)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sock_path)
s.listen(1)
time.sleep(0.2)
"#,
            args_file = args_file.to_string_lossy().to_string()
        ),
    )
    .expect("write fake child");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&child).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&child, perms).unwrap();
    }

    // SAFETY: this test runs in-process and restores QUECTO_CHILD_BINARY before returning.
    unsafe { std::env::set_var("QUECTO_CHILD_BINARY", &child) };
    let tool = SpawnTool::with_base_dir(vec![], true, dir.path().to_path_buf())
        .with_socket_dir(dir.path().to_path_buf());
    let mut cfg = tool.parse_args(r#"{"agent_id":"worker"}"#).unwrap();
    cfg.workflow_spec = Some(crate::domain::workflow::WorkflowSpec {
        template: crate::domain::workflow::WorkflowTemplate {
            id: "wf".into(),
            label: "Workflow".into(),
            description: "small valid spec".into(),
            when_to_use: None,
            steps: vec![],
            guards: vec![],
        },
    });

    let result = tool.launch_uds_agent(&cfg).await.expect("spawn succeeds");
    // SAFETY: paired cleanup for the test-scoped environment override above.
    unsafe { std::env::remove_var("QUECTO_CHILD_BINARY") };
    assert!(!result.is_error, "{}", result.content);

    let args: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&args_file).unwrap()).unwrap();
    let session = args[args.iter().position(|arg| arg == "-s").unwrap() + 1].clone();
    let socket = args[args.iter().position(|arg| arg == "--socket").unwrap() + 1].clone();
    assert_ne!(
        session, "worker",
        "child persist session key must be UUID, not display label"
    );
    assert!(
        uuid::Uuid::parse_str(&session).is_ok(),
        "session should be UUID: {session}"
    );
    assert!(
        socket.contains(&session),
        "socket should be derived from UUID session {session}: {socket}"
    );
    assert!(
        !socket.contains("worker"),
        "socket path must not contain display label: {socket}"
    );

    super::shutdown_all(tool.registry());
}
