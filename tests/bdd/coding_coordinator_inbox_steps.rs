//! BDD step definitions for coordinator inbox processing.
//!
//! These steps test `application::coordinator_inbox::tick()` — the core
//! inbox-processing loop that reads commands from the IPC inbox, dispatches
//! them through the `CodingJobService`, and writes responses/state.

use cucumber::{given, then, when};
use std::sync::Mutex;

use quecto::application::coordinator_inbox;
use quecto::domain::coding_command::*;
use quecto::domain::coding_ipc::*;
use quecto::domain::coding_job::JobState;
use quecto::domain::coding_ports::CodingJobService;

use crate::QuectoWorld;

// ============================================================================
// Mock IPC for inbox BDD scenarios
// ============================================================================

#[derive(Debug)]
pub struct BddInboxMockIpc {
    pub commands: Mutex<Vec<CoordinatorIpcCommand>>,
    pub responses: Mutex<Vec<CoordinatorIpcResponse>>,
    pub acknowledged: Mutex<Vec<String>>,
    pub state: Mutex<Option<CoordinatorState>>,
}

impl Default for BddInboxMockIpc {
    fn default() -> Self {
        Self {
            commands: Mutex::new(vec![]),
            responses: Mutex::new(vec![]),
            acknowledged: Mutex::new(vec![]),
            state: Mutex::new(None),
        }
    }
}

impl CoordinatorIpc for BddInboxMockIpc {
    fn write_command(&self, cmd: &CoordinatorIpcCommand) -> Result<(), String> {
        self.commands.lock().unwrap().push(cmd.clone());
        Ok(())
    }
    fn read_pending_commands(&self) -> Result<Vec<CoordinatorIpcCommand>, String> {
        Ok(self.commands.lock().unwrap().drain(..).collect())
    }
    fn acknowledge_command(&self, command_id: &str) -> Result<(), String> {
        self.acknowledged
            .lock()
            .unwrap()
            .push(command_id.to_string());
        Ok(())
    }
    fn write_response(&self, resp: &CoordinatorIpcResponse) -> Result<(), String> {
        self.responses.lock().unwrap().push(resp.clone());
        Ok(())
    }
    fn read_response(&self, command_id: &str) -> Result<Option<CoordinatorIpcResponse>, String> {
        Ok(self
            .responses
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.command_id == command_id)
            .cloned())
    }
    fn write_notification(&self, _notif: &CoordinatorNotification) -> Result<(), String> {
        Ok(())
    }
    fn read_notifications(&self) -> Result<Vec<CoordinatorNotification>, String> {
        Ok(vec![])
    }
    fn acknowledge_notification(&self, _filename: &str) -> Result<(), String> {
        Ok(())
    }
    fn write_state(&self, state: &CoordinatorState) -> Result<(), String> {
        *self.state.lock().unwrap() = Some(state.clone());
        Ok(())
    }
    fn read_state(&self) -> Result<Option<CoordinatorState>, String> {
        Ok(self.state.lock().unwrap().clone())
    }
    fn write_pid(&self, _pid: u32) -> Result<(), String> {
        Ok(())
    }
    fn read_pid(&self) -> Result<Option<u32>, String> {
        Ok(None)
    }
    fn is_coordinator_alive(&self) -> bool {
        false
    }
}

// ============================================================================
// Mock CodingJobService for inbox BDD scenarios
// ============================================================================

#[derive(Debug, Default)]
pub struct BddInboxMockJobService {
    pub jobs: std::collections::HashMap<String, JobState>,
    next_job_id: u32,
    fail_with: Option<CommandError>,
}

impl CodingJobService for BddInboxMockJobService {
    fn create_repo(&mut self, req: CreateRequest) -> Result<CreateResponse, CommandError> {
        Ok(CreateResponse {
            name: req.name,
            path: "/workspace/test".to_string(),
            created: true,
        })
    }
    fn import_repo(&mut self, req: ImportRequest) -> Result<ImportResponse, CommandError> {
        Ok(ImportResponse {
            name: req.name.unwrap_or_else(|| "imported".to_string()),
            path: "/workspace/imported".to_string(),
            imported: true,
        })
    }
    fn run(&mut self, _req: RunRequest) -> Result<RunResponse, CommandError> {
        self.next_job_id += 1;
        let job_id = format!("job_{:06}", self.next_job_id);
        let run_id = format!("run_{:06}", self.next_job_id);
        self.jobs.insert(job_id.clone(), JobState::Queued);
        Ok(RunResponse {
            job_id,
            run_id,
            state: JobState::Queued,
        })
    }
    fn status_by_job_id(&self, job_id: &str) -> Result<StatusResponse, CommandError> {
        if let Some(ref err) = self.fail_with {
            return Err(err.clone());
        }
        let state = self
            .jobs
            .get(job_id)
            .copied()
            .ok_or(CommandError::NotFound)?;
        Ok(StatusResponse {
            job_id: job_id.to_string(),
            run_id: "run_001".to_string(),
            state,
            summary: None,
            progress: None,
            todos: vec![],
            artifacts: vec![],
            error_code: None,
            error_detail: None,
            cancel_reason: None,
            state_entered_at: None,
            created_at: None,
            last_event_ts: None,
            last_event_type: None,
        })
    }
    fn status_by_run_id(&self, run_id: &str) -> Result<StatusResponse, CommandError> {
        Ok(StatusResponse {
            job_id: "job_001".to_string(),
            run_id: run_id.to_string(),
            state: JobState::Running,
            summary: None,
            progress: None,
            todos: vec![],
            artifacts: vec![],
            error_code: None,
            error_detail: None,
            cancel_reason: None,
            state_entered_at: None,
            created_at: None,
            last_event_ts: None,
            last_event_type: None,
        })
    }
    fn cancel(&mut self, job_id: &str) -> Result<CancelResponse, CommandError> {
        self.jobs
            .get_mut(job_id)
            .map(|s| *s = JobState::Canceled)
            .ok_or(CommandError::NotFound)?;
        Ok(CancelResponse {
            job_id: job_id.to_string(),
            state: JobState::Canceled,
        })
    }
    fn cleanup(
        &mut self,
        job_id: &str,
        _keep_artifacts: bool,
    ) -> Result<CleanupResponse, CommandError> {
        self.jobs.remove(job_id).ok_or(CommandError::NotFound)?;
        Ok(CleanupResponse {
            job_id: job_id.to_string(),
            cleaned: true,
        })
    }
    fn cleanup_all(&mut self, req: &CleanupAllRequest) -> Result<CleanupAllResponse, CommandError> {
        let candidates: Vec<String> = self
            .jobs
            .iter()
            .filter(|(_, s)| {
                req.state_filter
                    .as_ref()
                    .map(|f| f.contains(s))
                    .unwrap_or(true)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut cleaned = vec![];
        let mut skipped = vec![];
        for job_id in candidates {
            let terminal = self
                .jobs
                .get(&job_id)
                .map(|s| s.is_terminal())
                .unwrap_or(false);
            if !terminal {
                skipped.push(job_id);
                continue;
            }
            self.jobs.remove(&job_id);
            cleaned.push(job_id);
        }
        Ok(CleanupAllResponse {
            cleaned_count: cleaned.len(),
            cleaned_job_ids: cleaned,
            skipped_job_ids: skipped,
        })
    }

    fn list(&self, _req: &ListRequest) -> ListResponse {
        ListResponse {
            jobs: self
                .jobs
                .iter()
                .map(|(id, state)| ListJobEntry {
                    job_id: id.clone(),
                    run_id: "run_001".to_string(),
                    state: *state,
                    summary: None,
                    created_at: None,
                    state_entered_at: None,
                    last_event_ts: None,
                    last_event_type: None,
                })
                .collect(),
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_job_state(s: &str) -> JobState {
    match s {
        "queued" => JobState::Queued,
        "preparing" => JobState::Preparing,
        "running" => JobState::Running,
        "blocked" => JobState::Blocked,
        "failed" => JobState::Failed,
        "succeeded" => JobState::Succeeded,
        "canceled" => JobState::Canceled,
        other => panic!("unknown job state: {other}"),
    }
}

// ============================================================================
// Given steps
// ============================================================================

#[given("a coordinator inbox processor with a mock job service")]
fn given_inbox_processor(world: &mut QuectoWorld) {
    world.inbox_ipc = Some(BddInboxMockIpc::default());
    world.inbox_svc = Some(BddInboxMockJobService::default());
    world.inbox_tick_result = None;
    world.inbox_last_cmd_id = None;
}

#[given(regex = r#"^a pending inbox command with action "(\w+)" and payload (.+)$"#)]
fn given_pending_command(world: &mut QuectoWorld, action: String, payload_str: String) {
    let payload: serde_json::Value =
        serde_json::from_str(&payload_str).expect("valid JSON payload");
    let ipc = world.inbox_ipc.as_ref().expect("inbox IPC not initialized");
    let id = {
        let mut cmds = ipc.commands.lock().unwrap();
        let id = format!("cmd_{}", cmds.len());
        cmds.push(CoordinatorIpcCommand {
            command_id: id.clone(),
            action,
            payload,
        });
        id
    };
    // Track the last command ID for single-command scenarios
    world.inbox_last_cmd_id = Some(id);
}

#[given(regex = r#"^the mock job service has a job "([^"]+)" in state "(\w+)"$"#)]
fn given_job_in_state(world: &mut QuectoWorld, job_id: String, state_str: String) {
    let svc = world
        .inbox_svc
        .as_mut()
        .expect("mock service not initialized");
    svc.jobs.insert(job_id, parse_job_state(&state_str));
}

#[given(regex = r#"^the mock job service will fail with "([^"]+)"$"#)]
fn given_service_will_fail(world: &mut QuectoWorld, error_kind: String) {
    let svc = world
        .inbox_svc
        .as_mut()
        .expect("mock service not initialized");
    svc.fail_with = Some(match error_kind.as_str() {
        "not_found" => CommandError::NotFound,
        "already_exists" => CommandError::AlreadyExists,
        other => CommandError::Internal(other.to_string()),
    });
}

// ============================================================================
// When steps
// ============================================================================

#[when("the processor ticks once")]
fn when_tick_once(world: &mut QuectoWorld) {
    let ipc = world.inbox_ipc.as_ref().expect("inbox IPC not initialized");
    let svc = world
        .inbox_svc
        .as_mut()
        .expect("mock service not initialized");
    let result = coordinator_inbox::tick(ipc, svc).expect("tick should succeed");
    world.inbox_tick_result = Some(result);
}

// ============================================================================
// Then steps
// ============================================================================

#[then("the inbox outbox should contain a response for the command")]
fn then_outbox_has_response(world: &mut QuectoWorld) {
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let cmd_id = world.inbox_last_cmd_id.as_ref().expect("last cmd id");
    let responses = ipc.responses.lock().unwrap();
    assert!(
        responses.iter().any(|r| &r.command_id == cmd_id),
        "expected response for command {cmd_id}, found: {:?}",
        responses.iter().map(|r| &r.command_id).collect::<Vec<_>>()
    );
}

#[then(regex = r#"^the inbox response should have ok (true|false)$"#)]
fn then_response_ok(world: &mut QuectoWorld, expected_str: String) {
    let expected = expected_str == "true";
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let cmd_id = world.inbox_last_cmd_id.as_ref().expect("last cmd id");
    let responses = ipc.responses.lock().unwrap();
    let resp = responses
        .iter()
        .find(|r| &r.command_id == cmd_id)
        .expect("response not found");
    assert_eq!(resp.ok, expected, "response.ok should be {expected}");
}

#[then(regex = r#"^the inbox response body should contain "([^"]+)"$"#)]
fn then_response_body_contains(world: &mut QuectoWorld, needle: String) {
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let cmd_id = world.inbox_last_cmd_id.as_ref().expect("last cmd id");
    let responses = ipc.responses.lock().unwrap();
    let resp = responses
        .iter()
        .find(|r| &r.command_id == cmd_id)
        .expect("response not found");
    let body = resp.body.as_ref().expect("response should have a body");
    let body_str = serde_json::to_string(body).unwrap();
    assert!(
        body_str.contains(&needle),
        "response body should contain '{needle}', got: {body_str}"
    );
}

#[then(regex = r#"^the inbox response error should contain "([^"]+)"$"#)]
fn then_response_error_contains(world: &mut QuectoWorld, needle: String) {
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let cmd_id = world.inbox_last_cmd_id.as_ref().expect("last cmd id");
    let responses = ipc.responses.lock().unwrap();
    let resp = responses
        .iter()
        .find(|r| &r.command_id == cmd_id)
        .expect("response not found");
    let error = resp.error.as_ref().expect("response should have an error");
    assert!(
        error.contains(&needle),
        "response error should contain '{needle}', got: {error}"
    );
}

#[then("the inbox command should be acknowledged")]
fn then_command_acknowledged(world: &mut QuectoWorld) {
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let cmd_id = world.inbox_last_cmd_id.as_ref().expect("last cmd id");
    let acked = ipc.acknowledged.lock().unwrap();
    assert!(
        acked.iter().any(|a| a == cmd_id),
        "command {cmd_id} should be acknowledged"
    );
}

#[then(regex = r#"^the inbox outbox should contain (\d+) responses$"#)]
fn then_outbox_count(world: &mut QuectoWorld, expected: usize) {
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let responses = ipc.responses.lock().unwrap();
    assert_eq!(
        responses.len(),
        expected,
        "expected {expected} responses, got {}",
        responses.len()
    );
}

#[then("all inbox responses should have ok true")]
fn then_all_responses_ok(world: &mut QuectoWorld) {
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let responses = ipc.responses.lock().unwrap();
    assert!(
        responses.iter().all(|r| r.ok),
        "all responses should have ok=true"
    );
}

#[then("the processor should signal shutdown")]
fn then_shutdown_signaled(world: &mut QuectoWorld) {
    let result = world.inbox_tick_result.as_ref().expect("tick result");
    assert!(
        result.shutdown_requested,
        "tick should signal shutdown_requested"
    );
}

#[then("a state snapshot should exist")]
fn then_state_exists(world: &mut QuectoWorld) {
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let state = ipc.state.lock().unwrap();
    assert!(state.is_some(), "state snapshot should have been written");
}

#[then(regex = r#"^the state snapshot should have alive (true|false)$"#)]
fn then_state_alive(world: &mut QuectoWorld, expected_str: String) {
    let expected = expected_str == "true";
    let ipc = world.inbox_ipc.as_ref().expect("ipc");
    let state = ipc.state.lock().unwrap();
    let s = state.as_ref().expect("state snapshot");
    assert_eq!(s.alive, expected, "state.alive should be {expected}");
}
