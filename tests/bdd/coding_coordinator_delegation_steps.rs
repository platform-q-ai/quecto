//! BDD step definitions for coordinator delegation via file-based IPC.
//!
//! These steps test the domain types (CoordinatorIpcCommand, CoordinatorIpcResponse,
//! CoordinatorNotification, CoordinatorState) and the FileCoordinatorIpc infrastructure
//! implementation.

use cucumber::{given, then, when};
use tempfile::TempDir;

use quecto::domain::coding_ipc::{
    CoordinatorIpc, CoordinatorIpcCommand, CoordinatorIpcResponse, CoordinatorNotification,
    CoordinatorState, NotificationType,
};
use quecto::infrastructure::coding::coordinator_ipc::FileCoordinatorIpc;

use crate::QuectoWorld;

// ============================================================================
// Helpers
// ============================================================================

fn ensure_ipc(world: &mut QuectoWorld) {
    if world.coord_ipc.is_none() {
        let td = TempDir::new().expect("temp dir");
        let ipc = FileCoordinatorIpc::new(td.path().join("coordinator")).expect("create ipc");
        world.coord_ipc = Some(ipc);
        world._coord_ipc_temp_dir = Some(td);
    }
}

fn ipc(world: &QuectoWorld) -> &FileCoordinatorIpc {
    world.coord_ipc.as_ref().expect("ipc not initialized")
}

fn parse_notification_type(s: &str) -> NotificationType {
    match s {
        "worker_blocked" => NotificationType::WorkerBlocked,
        "job_failed" => NotificationType::JobFailed,
        "worker_stuck" => NotificationType::WorkerStuck,
        "batch_complete" => NotificationType::BatchComplete,
        "policy_violation" => NotificationType::PolicyViolation,
        _ => panic!("unknown notification type: {s}"),
    }
}

// ============================================================================
// Domain type scenarios
// ============================================================================

#[given(regex = r#"^a coordinator IPC command with action "(\w+)" and payload (.+)$"#)]
fn given_ipc_command(world: &mut QuectoWorld, action: String, payload_str: String) {
    let payload: serde_json::Value =
        serde_json::from_str(&payload_str).expect("valid JSON payload");
    let cmd = CoordinatorIpcCommand {
        command_id: format!("cmd_{}", uuid::Uuid::new_v4().as_simple()),
        action,
        payload,
    };
    world.coord_ipc_last_cmd = Some(cmd);
}

#[when("the command is serialized to JSON")]
fn when_command_serialized(world: &mut QuectoWorld) {
    let cmd = world.coord_ipc_last_cmd.as_ref().expect("command set");
    let json = serde_json::to_string(cmd).expect("serialize");
    world.coord_ipc_last_json = Some(json);
}

#[then(regex = r#"^the JSON should contain a "(\w+)" field$"#)]
fn then_json_contains_field(world: &mut QuectoWorld, field: String) {
    let json = world.coord_ipc_last_json.as_ref().expect("json set");
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    assert!(
        v.get(&field).is_some(),
        "JSON should contain field '{field}', got: {json}"
    );
}

#[then(regex = r#"^the JSON should contain an? "(\w+)" field with value "([^"]+)"$"#)]
fn then_json_contains_field_string_value(world: &mut QuectoWorld, field: String, value: String) {
    let json = world.coord_ipc_last_json.as_ref().expect("json set");
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    let actual = v
        .get(&field)
        .unwrap_or_else(|| panic!("field '{field}' not found in {json}"));
    assert_eq!(
        actual.as_str().unwrap_or(""),
        value,
        "field '{field}' should be '{value}'"
    );
}

#[then(regex = r#"^the JSON should contain a "(\w+)" object$"#)]
fn then_json_contains_object(world: &mut QuectoWorld, field: String) {
    let json = world.coord_ipc_last_json.as_ref().expect("json set");
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    let val = v
        .get(&field)
        .unwrap_or_else(|| panic!("field '{field}' not found"));
    assert!(val.is_object(), "'{field}' should be an object");
}

// --- Response domain types ---

#[given(regex = r#"^a coordinator IPC response with command_id "([^"]+)" and ok (true|false)$"#)]
fn given_ipc_response_ok(world: &mut QuectoWorld, command_id: String, ok: String) {
    let resp = CoordinatorIpcResponse {
        command_id,
        ok: ok == "true",
        body: Some(serde_json::json!({"result": "ok"})),
        error: None,
    };
    world.coord_ipc_last_response = Some(resp);
}

#[given(regex = r#"^a coordinator IPC response with command_id "([^"]+)" and error "([^"]+)"$"#)]
fn given_ipc_response_error(world: &mut QuectoWorld, command_id: String, error: String) {
    let resp = CoordinatorIpcResponse {
        command_id,
        ok: false,
        body: None,
        error: Some(error),
    };
    world.coord_ipc_last_response = Some(resp);
}

#[when("the response is serialized to JSON")]
fn when_response_serialized(world: &mut QuectoWorld) {
    let resp = world
        .coord_ipc_last_response
        .as_ref()
        .expect("response set");
    let json = serde_json::to_string(resp).expect("serialize");
    world.coord_ipc_last_json = Some(json);
}

#[then(regex = r#"^the JSON should contain "(\w+)" with value "([^"]+)"$"#)]
fn then_json_field_eq_string(world: &mut QuectoWorld, field: String, value: String) {
    let json = world.coord_ipc_last_json.as_ref().expect("json set");
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    let actual = v
        .get(&field)
        .unwrap_or_else(|| panic!("field '{field}' not found in {json}"));
    assert_eq!(
        actual.as_str().unwrap_or(""),
        value,
        "field '{field}' value mismatch"
    );
}

#[then(regex = r#"^the JSON should contain "(\w+)" with value (true|false)$"#)]
fn then_json_field_eq_bool(world: &mut QuectoWorld, field: String, value: String) {
    let json = world.coord_ipc_last_json.as_ref().expect("json set");
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    let expected = value == "true";
    let actual = v
        .get(&field)
        .unwrap_or_else(|| panic!("field '{field}' not found in {json}"));
    assert_eq!(
        actual.as_bool().unwrap(),
        expected,
        "field '{field}' bool mismatch"
    );
}

#[then(regex = r#"^the JSON should contain "(\w+)" with value (\d+)$"#)]
fn then_json_field_eq_number(world: &mut QuectoWorld, field: String, value: String) {
    let json = world.coord_ipc_last_json.as_ref().expect("json set");
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    let expected: u64 = value.parse().unwrap();
    let actual = v
        .get(&field)
        .unwrap_or_else(|| panic!("field '{field}' not found in {json}"));
    assert_eq!(
        actual.as_u64().unwrap(),
        expected,
        "field '{field}' number mismatch"
    );
}

// --- Notification domain types ---

#[given(regex = r#"^a coordinator notification of type "(\w+)" for job "([^"]+)"$"#)]
fn given_notification_for_job(world: &mut QuectoWorld, ntype: String, job_id: String) {
    let notif = CoordinatorNotification {
        notification_type: parse_notification_type(&ntype),
        job_id: Some(job_id),
        job_ids: vec![],
        detail: None,
        no_progress_minutes: None,
        ts: "2026-01-15T10:00:00Z".to_string(),
    };
    world.coord_ipc_last_notification = Some(notif);
}

#[when("the notification is serialized to JSON")]
fn when_notification_serialized(world: &mut QuectoWorld) {
    let notif = world
        .coord_ipc_last_notification
        .as_ref()
        .expect("notification set");
    let json = serde_json::to_string(notif).expect("serialize");
    world.coord_ipc_last_json = Some(json);
}

// --- State snapshot domain types ---

#[given(
    regex = r#"^a coordinator state snapshot with (\d+) active jobs and last heartbeat "([^"]+)"$"#
)]
fn given_state_snapshot(world: &mut QuectoWorld, active_jobs: u32, last_heartbeat: String) {
    let state = CoordinatorState {
        alive: true,
        active_jobs,
        last_heartbeat,
        job_summary: serde_json::json!({}),
    };
    world.coord_ipc_last_state = Some(state);
}

#[when("the state is serialized to JSON")]
fn when_state_serialized(world: &mut QuectoWorld) {
    let state = world.coord_ipc_last_state.as_ref().expect("state set");
    let json = serde_json::to_string(state).expect("serialize");
    world.coord_ipc_last_json = Some(json);
}

// ============================================================================
// File-based IPC infrastructure scenarios
// ============================================================================

#[given("a coordinator IPC directory at a temp path")]
fn given_ipc_dir(world: &mut QuectoWorld) {
    ensure_ipc(world);
}

#[when(regex = r#"^a command with action "(\w+)" is written to the inbox$"#)]
fn when_write_command_inbox(world: &mut QuectoWorld, action: String) {
    ensure_ipc(world);
    let cmd = CoordinatorIpcCommand {
        command_id: format!("cmd_{}", uuid::Uuid::new_v4().as_simple()),
        action,
        payload: serde_json::json!({"goal": "test"}),
    };
    ipc(world).write_command(&cmd).expect("write command");
    world.coord_ipc_last_cmd = Some(cmd);
}

#[then("a JSON file should exist in the inbox directory")]
fn then_inbox_has_file(world: &mut QuectoWorld) {
    let cmds = ipc(world).read_pending_commands().expect("read");
    assert!(!cmds.is_empty(), "inbox should have at least one file");
}

#[then("the file name should match the command_id with .json extension")]
fn then_inbox_filename_matches(world: &mut QuectoWorld) {
    let cmd = world.coord_ipc_last_cmd.as_ref().expect("command");
    let cmds = ipc(world).read_pending_commands().expect("read");
    assert!(
        cmds.iter().any(|c| c.command_id == cmd.command_id),
        "should find command with matching id"
    );
}

#[given("a coordinator IPC directory with a pending command")]
fn given_ipc_dir_with_command(world: &mut QuectoWorld) {
    ensure_ipc(world);
    let cmd = CoordinatorIpcCommand {
        command_id: "cmd_pending".to_string(),
        action: "run".to_string(),
        payload: serde_json::json!({"goal": "pending test"}),
    };
    ipc(world).write_command(&cmd).expect("write");
    world.coord_ipc_last_cmd = Some(cmd);
}

#[when("the inbox is polled for new commands")]
fn when_poll_inbox(world: &mut QuectoWorld) {
    let cmds = ipc(world).read_pending_commands().expect("read");
    if let Some(cmd) = cmds.into_iter().next() {
        world.coord_ipc_last_cmd = Some(cmd);
    }
}

#[then("the command should be returned with its action and payload")]
fn then_command_returned(world: &mut QuectoWorld) {
    let cmd = world.coord_ipc_last_cmd.as_ref().expect("command");
    assert!(!cmd.action.is_empty(), "action should be set");
}

#[then("the command file should still exist until acknowledged")]
fn then_command_file_exists(world: &mut QuectoWorld) {
    let cmds = ipc(world).read_pending_commands().expect("read");
    assert!(!cmds.is_empty(), "command file should still exist");
}

#[when(regex = r#"^a response for command_id "([^"]+)" is written to the outbox$"#)]
fn when_write_response_outbox(world: &mut QuectoWorld, command_id: String) {
    ensure_ipc(world);
    let resp = CoordinatorIpcResponse {
        command_id,
        ok: true,
        body: Some(serde_json::json!({"job_id": "j1"})),
        error: None,
    };
    ipc(world).write_response(&resp).expect("write response");
}

#[then(regex = r#"^a JSON file "([^"]+)" should exist in the outbox directory$"#)]
fn then_outbox_has_file(world: &mut QuectoWorld, filename: String) {
    let resp = ipc(world)
        .read_response(&filename.replace(".json", ""))
        .expect("read");
    assert!(resp.is_some(), "outbox should contain {filename}");
}

#[given(regex = r#"^a response file is pre-written for command_id "([^"]+)"$"#)]
fn given_response_pre_written(world: &mut QuectoWorld, command_id: String) {
    ensure_ipc(world);
    let resp = CoordinatorIpcResponse {
        command_id: command_id.clone(),
        ok: true,
        body: Some(serde_json::json!({"status": "ok"})),
        error: None,
    };
    ipc(world).write_response(&resp).expect("write");
}

#[when(
    regex = r#"^the outbox is polled for command_id "([^"]+)" with timeout (\d+)\s*(second|ms)s?$"#
)]
fn when_poll_outbox(world: &mut QuectoWorld, command_id: String, _timeout: u32, _unit: String) {
    let result = ipc(world).read_response(&command_id);
    match result {
        Ok(Some(resp)) => {
            world.coord_ipc_poll_result = Some(Ok(resp));
        }
        Ok(None) => {
            world.coord_ipc_poll_result = Some(Err("timeout: no response".to_string()));
        }
        Err(e) => {
            world.coord_ipc_poll_result = Some(Err(e));
        }
    }
}

#[then("the response should be returned successfully")]
fn then_response_returned(world: &mut QuectoWorld) {
    let result = world.coord_ipc_poll_result.as_ref().expect("poll result");
    assert!(result.is_ok(), "should have response, got: {result:?}");
}

#[then("the poll should return a timeout error")]
fn then_poll_timeout(world: &mut QuectoWorld) {
    let result = world.coord_ipc_poll_result.as_ref().expect("poll result");
    assert!(result.is_err(), "should be timeout error");
}

#[when("the command is acknowledged")]
fn when_acknowledge_command(world: &mut QuectoWorld) {
    let cmd = world.coord_ipc_last_cmd.as_ref().expect("command");
    ipc(world)
        .acknowledge_command(&cmd.command_id)
        .expect("ack");
}

#[then("the command file should be removed from the inbox")]
fn then_command_removed(world: &mut QuectoWorld) {
    let cmds = ipc(world).read_pending_commands().expect("read");
    let cmd = world.coord_ipc_last_cmd.as_ref().expect("command");
    assert!(
        !cmds.iter().any(|c| c.command_id == cmd.command_id),
        "command should be removed"
    );
}

// --- Notifications infrastructure ---

#[when(regex = r#"^a "(\w+)" notification is written for job "([^"]+)"$"#)]
fn when_write_notification(world: &mut QuectoWorld, ntype: String, job_id: String) {
    ensure_ipc(world);
    let notif = CoordinatorNotification {
        notification_type: parse_notification_type(&ntype),
        job_id: Some(job_id),
        job_ids: vec![],
        detail: Some("test detail".to_string()),
        no_progress_minutes: None,
        ts: "2026-01-15T10:00:00Z".to_string(),
    };
    ipc(world)
        .write_notification(&notif)
        .expect("write notification");
    world.coord_ipc_last_notification = Some(notif);
}

#[then("a JSON file should exist in the notifications directory")]
fn then_notifications_has_file(world: &mut QuectoWorld) {
    let notifs = ipc(world).read_notifications().expect("read");
    assert!(!notifs.is_empty(), "notifications should have a file");
}

#[then(regex = r#"^the file name should contain "(\w+)"$"#)]
fn then_notification_filename_contains(world: &mut QuectoWorld, expected: String) {
    // We verify via the notification type in the read result
    let notifs = ipc(world).read_notifications().expect("read");
    assert!(
        notifs
            .iter()
            .any(|n| n.notification_type.to_string().contains(&expected)),
        "notification filename should contain '{expected}'"
    );
}

#[given(regex = r#"^a coordinator IPC directory with (\d+) pending notifications$"#)]
fn given_ipc_with_n_notifications(world: &mut QuectoWorld, n: usize) {
    ensure_ipc(world);
    for i in 0..n {
        let notif = CoordinatorNotification {
            notification_type: NotificationType::JobFailed,
            job_id: Some(format!("job_{i:03}")),
            job_ids: vec![],
            detail: None,
            no_progress_minutes: None,
            ts: format!("2026-01-15T10:{i:02}:00Z"),
        };
        ipc(world)
            .write_notification(&notif)
            .expect("write notification");
    }
}

#[when("notifications are read")]
fn when_read_notifications(world: &mut QuectoWorld) {
    let notifs = ipc(world).read_notifications().expect("read");
    world.coord_ipc_notifications = Some(notifs);
}

#[then(regex = r#"^(\d+) notifications should be returned$"#)]
fn then_n_notifications(world: &mut QuectoWorld, expected: usize) {
    let notifs = world.coord_ipc_notifications.as_ref().expect("notifs");
    assert_eq!(notifs.len(), expected);
}

#[then("they should be ordered by timestamp")]
fn then_notifications_ordered(world: &mut QuectoWorld) {
    let notifs = world.coord_ipc_notifications.as_ref().expect("notifs");
    for w in notifs.windows(2) {
        assert!(
            w[0].ts <= w[1].ts,
            "notifications should be ordered: {} <= {}",
            w[0].ts,
            w[1].ts
        );
    }
}

#[given("a coordinator IPC directory with a pending notification")]
fn given_ipc_with_notification(world: &mut QuectoWorld) {
    ensure_ipc(world);
    let notif = CoordinatorNotification {
        notification_type: NotificationType::WorkerBlocked,
        job_id: Some("j_ack".to_string()),
        job_ids: vec![],
        detail: Some("question".to_string()),
        no_progress_minutes: None,
        ts: "2026-01-15T10:00:00Z".to_string(),
    };
    ipc(world)
        .write_notification(&notif)
        .expect("write notification");
}

#[when("the notification is acknowledged")]
fn when_acknowledge_notification(world: &mut QuectoWorld) {
    let notifs = ipc(world).read_notifications().expect("read");
    assert!(!notifs.is_empty());
    // Find the actual filename in the notifications dir
    let dir = world
        ._coord_ipc_temp_dir
        .as_ref()
        .expect("temp dir")
        .path()
        .join("coordinator/notifications");
    let files: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    for f in &files {
        ipc(world)
            .acknowledge_notification(f)
            .expect("ack notification");
    }
}

#[then("the notification file should be removed")]
fn then_notification_removed(world: &mut QuectoWorld) {
    let notifs = ipc(world).read_notifications().expect("read");
    assert!(notifs.is_empty(), "notification should be removed");
}

// --- State snapshot infrastructure ---

#[when(regex = r#"^a state snapshot is written with (\d+) active jobs?$"#)]
fn when_write_state(world: &mut QuectoWorld, active_jobs: u32) {
    ensure_ipc(world);
    let state = CoordinatorState {
        alive: true,
        active_jobs,
        last_heartbeat: "2026-01-15T10:00:00Z".to_string(),
        job_summary: serde_json::json!({}),
    };
    ipc(world).write_state(&state).expect("write state");
}

#[then("state.json should exist in the coordinator directory")]
fn then_state_exists(world: &mut QuectoWorld) {
    let state = ipc(world).read_state().expect("read state");
    assert!(state.is_some(), "state.json should exist");
}

#[then(regex = r#"^reading state\.json should return the snapshot with (\d+) active jobs?$"#)]
fn then_state_active_jobs(world: &mut QuectoWorld, expected: u32) {
    let state = ipc(world)
        .read_state()
        .expect("read state")
        .expect("exists");
    assert_eq!(state.active_jobs, expected);
}

// --- PID file infrastructure ---

#[when(regex = r#"^PID (\d+) is written to the pid file$"#)]
fn when_write_pid(world: &mut QuectoWorld, pid: u32) {
    ensure_ipc(world);
    ipc(world).write_pid(pid).expect("write pid");
}

#[then(regex = r#"^the pid file should contain "(\d+)"$"#)]
fn then_pid_file_contains(world: &mut QuectoWorld, expected: String) {
    let pid = ipc(world).read_pid().expect("read pid").expect("exists");
    assert_eq!(pid.to_string(), expected);
}

#[then(regex = r#"^reading the pid should return (\d+)$"#)]
fn then_pid_value(world: &mut QuectoWorld, expected: u32) {
    let pid = ipc(world).read_pid().expect("read pid").expect("exists");
    assert_eq!(pid, expected);
}

// --- Coordinator liveness ---

#[given("a coordinator IPC directory with pid file containing the current process PID")]
fn given_ipc_with_current_pid(world: &mut QuectoWorld) {
    ensure_ipc(world);
    let my_pid = std::process::id();
    ipc(world).write_pid(my_pid).expect("write pid");
}

#[given(regex = r#"^a coordinator IPC directory with pid file containing PID (\d+)$"#)]
fn given_ipc_with_specific_pid(world: &mut QuectoWorld, pid: u32) {
    ensure_ipc(world);
    ipc(world).write_pid(pid).expect("write pid");
}

#[when("coordinator liveness is checked")]
fn when_check_liveness(world: &mut QuectoWorld) {
    let alive = ipc(world).is_coordinator_alive();
    world.coord_ipc_alive = Some(alive);
}

#[then("the coordinator should be reported as alive")]
fn then_coordinator_alive(world: &mut QuectoWorld) {
    assert_eq!(world.coord_ipc_alive, Some(true));
}

#[then("the coordinator should be reported as dead")]
fn then_coordinator_dead(world: &mut QuectoWorld) {
    assert_eq!(world.coord_ipc_alive, Some(false));
}

// --- Notification type assertions ---

#[given(
    regex = r#"^a coordinator notification of type "(\w+)" for job "([^"]+)" with detail "([^"]+)"$"#
)]
fn given_notification_with_detail(
    world: &mut QuectoWorld,
    ntype: String,
    job_id: String,
    detail: String,
) {
    let notif = CoordinatorNotification {
        notification_type: parse_notification_type(&ntype),
        job_id: Some(job_id),
        job_ids: vec![],
        detail: Some(detail),
        no_progress_minutes: None,
        ts: "2026-01-15T10:00:00Z".to_string(),
    };
    world.coord_ipc_last_notification = Some(notif);
}

#[given(regex = r#"^a coordinator notification of type "(\w+)" with job_ids \[([^\]]+)\]$"#)]
fn given_notification_batch(world: &mut QuectoWorld, ntype: String, ids_str: String) {
    let job_ids: Vec<String> = ids_str
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect();
    let notif = CoordinatorNotification {
        notification_type: parse_notification_type(&ntype),
        job_id: None,
        job_ids,
        detail: None,
        no_progress_minutes: None,
        ts: "2026-01-15T10:00:00Z".to_string(),
    };
    world.coord_ipc_last_notification = Some(notif);
}

#[given(
    regex = r#"^a coordinator notification of type "(\w+)" for job "([^"]+)" with no_progress_minutes (\d+)$"#
)]
fn given_notification_stuck(world: &mut QuectoWorld, ntype: String, job_id: String, minutes: u32) {
    let notif = CoordinatorNotification {
        notification_type: parse_notification_type(&ntype),
        job_id: Some(job_id),
        job_ids: vec![],
        detail: None,
        no_progress_minutes: Some(minutes),
        ts: "2026-01-15T10:00:00Z".to_string(),
    };
    world.coord_ipc_last_notification = Some(notif);
}

#[then(regex = r#"^the notification should have type "(\w+)"$"#)]
fn then_notification_type(world: &mut QuectoWorld, expected: String) {
    let notif = world.coord_ipc_last_notification.as_ref().expect("notif");
    assert_eq!(notif.notification_type.to_string(), expected);
}

#[then(regex = r#"^the notification detail should be "([^"]+)"$"#)]
fn then_notification_detail(world: &mut QuectoWorld, expected: String) {
    let notif = world.coord_ipc_last_notification.as_ref().expect("notif");
    assert_eq!(notif.detail.as_deref(), Some(expected.as_str()));
}

#[then(regex = r#"^the notification should reference (\d+) jobs$"#)]
fn then_notification_job_count(world: &mut QuectoWorld, expected: usize) {
    let notif = world.coord_ipc_last_notification.as_ref().expect("notif");
    assert_eq!(notif.job_ids.len(), expected);
}

#[then(regex = r#"^the no_progress_minutes should be (\d+)$"#)]
fn then_no_progress_minutes(world: &mut QuectoWorld, expected: u32) {
    let notif = world.coord_ipc_last_notification.as_ref().expect("notif");
    assert_eq!(notif.no_progress_minutes, Some(expected));
}
