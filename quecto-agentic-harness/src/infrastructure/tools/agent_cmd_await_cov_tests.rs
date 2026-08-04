use super::*;

fn poison_registry(registry: &SubagentRegistry) {
    let cloned = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = cloned.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err(), "registry should be poisoned");
}
use crate::infrastructure::test_support::read_framed_command_async;
use crate::infrastructure::tools::subagent_lifecycle::{
    SubagentLifecycleEvent, SubagentLifecycleState,
};
use crate::infrastructure::tools::subagent_registry::{ExitSignal, new_exit_signal_channel};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct AwaitJson {
    status: String,
    reason: Option<String>,
    agent_id: String,
    workflow: Option<WorkflowSnapshot>,
    error: Option<String>,
}

fn parse_result(result: ToolResult) -> AwaitJson {
    assert!(
        !result.is_error,
        "await result should be structured success"
    );
    serde_json::from_str(&result.content).expect("await json")
}

fn tool_with_entry(
    agent_id: &str,
    socket_path: PathBuf,
    status: SubagentStatus,
    exit_tx: Option<tokio::sync::watch::Sender<Option<ExitSignal>>>,
) -> (AgentCmdTool, SubagentRegistry) {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(socket_path, 123);
    entry.lifecycle = SubagentLifecycleState::from_status(&status);
    entry.status = status;
    entry.exit_signal_tx = exit_tx;
    registry.lock().unwrap().insert(agent_id.to_string(), entry);
    (AgentCmdTool::new(registry.clone()), registry)
}

async fn serve_one_get_state(socket: &Path, mut response: serde_json::Value) {
    let listener = tokio::net::UnixListener::bind(socket).expect("bind uds");
    loop {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut reader = tokio::io::BufReader::new(stream);
        let Some(request) = read_framed_command_async(&mut reader).await else {
            // `execute_await` first performs a synchronous connectability probe.
            continue;
        };
        assert!(request.contains("get_state"), "request: {request}");
        if let (Some(request_id), Some(obj)) = (
            serde_json::from_str::<serde_json::Value>(&request)
                .ok()
                .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_owned)),
            response.as_object_mut(),
        ) {
            obj.insert("id".to_string(), serde_json::Value::String(request_id));
        }
        quecto_line_io::write_frame(
            reader.get_mut(),
            response.to_string().as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .expect("write response");
        break;
    }
}

#[tokio::test]
async fn execute_await_rejects_bad_json_missing_agent_id_and_bad_agent_id() {
    let tool = AgentCmdTool::new(new_registry());

    let bad = tool.execute_await("not-json").await.unwrap();
    assert!(bad.is_error);
    assert!(bad.content.contains("invalid JSON arguments"));

    let missing = tool.execute_await(r#"{"timeout":1}"#).await.unwrap();
    assert!(missing.is_error);
    assert!(missing.content.contains("missing required field: agent_id"));

    let invalid = tool
        .execute_await(r#"{"agent_id":"../bad"}"#)
        .await
        .unwrap();
    assert!(invalid.is_error);
    assert!(invalid.content.contains("agent_cmd error"));
}

#[tokio::test]
async fn execute_await_reports_agent_not_found_and_connection_failed() {
    let tool = AgentCmdTool::new(new_registry());
    let missing = parse_result(
        tool.execute_await(r#"{"agent_id":"ghost","timeout":0}"#)
            .await
            .unwrap(),
    );
    assert_eq!(missing.status, "error");
    assert_eq!(missing.reason.as_deref(), Some("agent_not_found"));
    assert_eq!(missing.agent_id, "ghost");

    let tmp = tempfile::TempDir::new().unwrap();
    let dead_socket = tmp.path().join("dead.sock");
    std::fs::write(&dead_socket, b"not a socket").unwrap();
    let (tool, _) = tool_with_entry("stale", dead_socket, SubagentStatus::Idle, None);
    let failed = parse_result(
        tool.execute_await(r#"{"agent_id":"stale","timeout":0}"#)
            .await
            .unwrap(),
    );
    assert_eq!(failed.status, "error");
    assert_eq!(failed.reason.as_deref(), Some("connection_failed"));
}

#[tokio::test]
async fn execute_await_idle_fetches_workflow_snapshot_and_marks_consumed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("agent.sock");
    let server_socket = socket.clone();
    let server = tokio::spawn(async move {
        serve_one_get_state(
            &server_socket,
            serde_json::json!({
                "type": "response",
                "data": {"workflow": {"mode": "active", "progress": {"done": 2, "total": 5}}}
            }),
        )
        .await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let (tool, registry) = tool_with_entry("worker", socket, SubagentStatus::Idle, None);
    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"worker","timeout":2,"idle_timeout":0}"#)
            .await
            .unwrap(),
    );
    server.await.unwrap();

    assert_eq!(got.status, "idle");
    assert_eq!(got.reason.as_deref(), Some("idle"));
    let wf = got.workflow.expect("workflow snapshot");
    assert_eq!(wf.mode, "active");
    assert_eq!(wf.steps_completed, 2);
    assert_eq!(wf.steps_total, 5);
    assert!(registry.lock().unwrap()["worker"].completion_consumed_by_await);
}

#[tokio::test]
async fn execute_await_exit_signal_wakes_immediately_with_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("exit.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (exit_tx, _exit_rx) = new_exit_signal_channel();
    let (tool, registry) = tool_with_entry(
        "exiter",
        socket.clone(),
        SubagentStatus::Running,
        Some(exit_tx.clone()),
    );
    let accept = tokio::spawn(async move {
        let _ = listener.accept().await;
    });

    let notify = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        exit_tx
            .send(Some(ExitSignal {
                exit_code: Some(7),
                signal: None,
            }))
            .unwrap();
    });

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"exiter","timeout":2}"#)
            .await
            .unwrap(),
    );
    notify.await.unwrap();
    accept.abort();

    assert_eq!(got.status, "exited");
    assert_eq!(got.reason.as_deref(), Some("exit_code_7"));
    assert!(registry.lock().unwrap()["exiter"].completion_consumed_by_await);
}

#[tokio::test]
async fn execute_await_error_status_with_run_error_returns_structured_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("err.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let accept = tokio::spawn(async move {
        let _ = listener.accept().await;
    });
    let (tool, registry) = tool_with_entry("bad", socket, SubagentStatus::Error, None);
    registry.lock().unwrap().get_mut("bad").unwrap().run_error = Some("provider exploded".into());

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"bad","timeout":2,"idle_timeout":0}"#)
            .await
            .unwrap(),
    );
    accept.abort();

    assert_eq!(got.status, "error");
    assert_eq!(got.reason.as_deref(), Some("agent_error"));
    assert_eq!(got.error.as_deref(), Some("provider exploded"));
}

#[tokio::test]
async fn execute_await_duplicate_active_awaiter_is_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("dup.sock");
    std::fs::write(&socket, b"fake").unwrap();
    let active = new_active_awaits();
    active.lock().unwrap().insert("dup".to_string());
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("dup".into(), SubagentEntry::new(socket, 1));
    let tool = AgentCmdTool::with_active_awaits(registry, active);

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"dup","timeout":0}"#)
            .await
            .unwrap(),
    );
    assert_eq!(got.status, "error");
    assert_eq!(got.reason.as_deref(), Some("another_await_active"));
}

#[tokio::test]
async fn await_timeout_preserves_busy_child_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("busy.sock");
    let _listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (tool, registry) = tool_with_entry("busy", socket, SubagentStatus::Running, None);

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"busy","timeout":0}"#)
            .await
            .unwrap(),
    );

    assert_eq!(got.status, "timeout");
    let entry = registry.lock().unwrap().get("busy").unwrap().clone();
    assert_eq!(
        entry.last_lifecycle_event,
        Some(SubagentLifecycleEvent::AwaitTimedOut)
    );
    assert_eq!(entry.lifecycle, SubagentLifecycleState::Busy);
    assert_eq!(entry.status, SubagentStatus::Running);
}

/// #1378: wall-timeout lifecycle bookkeeping must use the resolved UUID key
/// even when the caller awaits by display label (production registry shape).
#[tokio::test]
async fn await_timeout_applies_lifecycle_on_uuid_keyed_entry_via_display_label() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("uuid-busy.sock");
    let _listener = tokio::net::UnixListener::bind(&socket).unwrap();

    let registry = new_registry();
    let uuid = crate::domain::ids::AgentUuid::new("44444444-4444-4444-8444-444444444444");
    let mut entry = SubagentEntry::with_identity(uuid.clone(), "busy".into(), socket, 123);
    entry.lifecycle = SubagentLifecycleState::from_status(&SubagentStatus::Running);
    entry.status = SubagentStatus::Running;
    registry.lock().unwrap().insert(uuid.to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"busy","timeout":0}"#)
            .await
            .unwrap(),
    );
    assert_eq!(got.status, "timeout");
    assert_eq!(got.agent_id, "busy");

    let entry = registry.lock().unwrap().get(uuid.as_str()).unwrap().clone();
    assert_eq!(
        entry.last_lifecycle_event,
        Some(SubagentLifecycleEvent::AwaitTimedOut),
        "timeout path must get_mut the UUID-keyed entry, not the display label"
    );
    assert_eq!(entry.lifecycle, SubagentLifecycleState::Busy);
    assert_eq!(entry.status, SubagentStatus::Running);
}

#[tokio::test]
async fn execute_await_exited_status_uses_signal_reason_from_registry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("signal.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let accept = tokio::spawn(async move {
        let _ = listener.accept().await;
    });
    let (exit_tx, _exit_rx) = new_exit_signal_channel();
    exit_tx
        .send(Some(ExitSignal {
            exit_code: None,
            signal: Some(15),
        }))
        .unwrap();
    let (tool, registry) = tool_with_entry("sig", socket, SubagentStatus::Exited, Some(exit_tx));

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"sig","timeout":1,"idle_timeout":0}"#)
            .await
            .unwrap(),
    );
    accept.abort();

    assert_eq!(got.status, "exited");
    assert_eq!(got.reason.as_deref(), Some("signal_15"));
    assert!(registry.lock().unwrap()["sig"].completion_consumed_by_await);
}

#[tokio::test]
async fn execute_await_recoverable_error_without_run_error_returns_idle() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("recover.sock");
    let server_socket = socket.clone();
    let server = tokio::spawn(async move {
        serve_one_get_state(
            &server_socket,
            serde_json::json!({"type":"response","data":{"workflow":{"mode":"active"}}}),
        )
        .await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let (tool, _) = tool_with_entry("recover", socket, SubagentStatus::Error, None);

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"recover","timeout":1,"idle_timeout":0}"#)
            .await
            .unwrap(),
    );
    server.await.unwrap();

    assert_eq!(got.status, "idle");
    assert_eq!(got.reason.as_deref(), Some("idle"));
    assert!(got.error.is_none());
    let wf = got.workflow.unwrap();
    assert_eq!(wf.mode, "active");
    assert_eq!(wf.steps_completed, 0);
    assert_eq!(wf.steps_total, 0);
}

/// #1378: await must resolve a live display label onto the UUID-keyed registry
/// entry (and arm/consume dedupe on that durable key).
#[tokio::test]
async fn execute_await_resolves_display_name_to_uuid_keyed_entry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("display-await.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let accept = tokio::spawn(async move {
        let _ = listener.accept().await;
    });

    let uuid = crate::domain::ids::AgentUuid::new("22222222-2222-4222-8222-222222222222");
    let active = new_active_awaits();
    let registry = new_registry();
    let mut entry = SubagentEntry::with_identity(uuid.clone(), "worker".into(), socket, 9);
    entry.status = SubagentStatus::Idle;
    registry.lock().unwrap().insert(uuid.to_string(), entry);

    let tool = AgentCmdTool::with_active_awaits(registry.clone(), active.clone());
    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"worker","timeout":1,"idle_timeout":0}"#)
            .await
            .unwrap(),
    );
    accept.abort();

    assert_eq!(got.status, "idle");
    assert_eq!(got.agent_id, "worker", "user-facing agent_id stays display");
    // Await must have armed dedupe on the UUID-keyed entry (not a display key).
    assert!(
        registry.lock().unwrap()[uuid.as_str()].completion_consumed_by_await,
        "dedupe flag must land on the UUID registry key"
    );
    assert!(
        !registry.lock().unwrap().contains_key("worker"),
        "must not create a display-keyed registry entry"
    );
    // Cross-token consume: arm by display, take by UUID.
    crate::infrastructure::tools::subagent_registry::mark_completion_consumed_by_await(
        &registry, "worker",
    );
    assert!(
        crate::infrastructure::tools::subagent_registry::take_completion_consumed_by_await(
            &registry,
            uuid.as_str()
        )
    );
    let _ = active;
}

#[tokio::test]
async fn execute_await_poisoned_locks_recover_for_lookup_duplicate_and_removed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("poison.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let accept = tokio::spawn(async move {
        let _ = listener.accept().await;
    });
    let active = new_active_awaits();
    let registry = new_registry();
    let mut entry = SubagentEntry::new(socket, 1);
    entry.status = SubagentStatus::Exited;
    registry.lock().unwrap().insert("poison".into(), entry);
    poison_registry(&registry);
    let tool = AgentCmdTool::with_active_awaits(registry.clone(), active);

    let got = parse_result(
        tool.execute_await(r#"{"agent_id":"poison","timeout":1,"idle_timeout":0}"#)
            .await
            .unwrap(),
    );
    accept.abort();
    assert_eq!(got.status, "exited");
    assert_eq!(got.reason.as_deref(), Some("exit_code_0"));
    assert!(
        registry.lock().unwrap_or_else(|e| e.into_inner())["poison"].completion_consumed_by_await
    );
}
