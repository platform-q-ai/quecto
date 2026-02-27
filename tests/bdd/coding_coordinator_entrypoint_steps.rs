//! BDD step definitions for the coordinator entrypoint feature.
//!
//! Tests argument parsing, tick loop integration, PID writing, and
//! state snapshot writing — all through the production `coordinator_inbox::tick()`
//! function and `parse_coordinator_args()`.

use super::*;

use quecto::application::coordinator_inbox;
use quecto::domain::coding_command::*;
use quecto::domain::coding_job::JobState;
use quecto::interface::cli::coordinator::parse_coordinator_args;

// ── Mock IPC for entrypoint scenarios ───────────────────────────────────

/// Mock IPC that stores commands/responses/state in memory.
#[derive(Debug)]
pub struct BddEntrypointMockIpc {
    pub commands: Mutex<Vec<CoordinatorIpcCommand>>,
    pub responses: Mutex<Vec<CoordinatorIpcResponse>>,
    pub acknowledged: Mutex<Vec<String>>,
    pub state: Mutex<Option<CoordinatorState>>,
    pub pid: Mutex<Option<u32>>,
}

impl BddEntrypointMockIpc {
    pub fn new() -> Self {
        Self {
            commands: Mutex::new(vec![]),
            responses: Mutex::new(vec![]),
            acknowledged: Mutex::new(vec![]),
            state: Mutex::new(None),
            pid: Mutex::new(None),
        }
    }

    pub fn add_command(&self, action: &str, payload: serde_json::Value) -> String {
        let id = format!("ep_cmd_{}", self.commands.lock().unwrap().len());
        self.commands.lock().unwrap().push(CoordinatorIpcCommand {
            command_id: id.clone(),
            action: action.to_string(),
            payload,
        });
        id
    }
}

impl CoordinatorIpc for BddEntrypointMockIpc {
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

    fn write_pid(&self, pid: u32) -> Result<(), String> {
        *self.pid.lock().unwrap() = Some(pid);
        Ok(())
    }

    fn read_pid(&self) -> Result<Option<u32>, String> {
        Ok(*self.pid.lock().unwrap())
    }

    fn is_coordinator_alive(&self) -> bool {
        false
    }
}

// ── Mock CodingJobService for entrypoint scenarios ──────────────────────

/// Minimal mock job service that handles list and shutdown.
#[derive(Debug, Default)]
pub struct BddEntrypointMockJobService {
    pub jobs: std::collections::HashMap<String, JobState>,
}

impl quecto::domain::coding_ports::CodingJobService for BddEntrypointMockJobService {
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
        let job_id = format!("job_{:06}", self.jobs.len() + 1);
        let run_id = format!("run_{:06}", self.jobs.len() + 1);
        self.jobs.insert(job_id.clone(), JobState::Queued);
        Ok(RunResponse {
            job_id,
            run_id,
            state: JobState::Queued,
        })
    }

    fn status_by_job_id(&self, job_id: &str) -> Result<StatusResponse, CommandError> {
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

// ── Step definitions: Argument parsing ──────────────────────────────────

#[given(regex = r#"^coordinator args "(.*)"$"#)]
fn given_coordinator_args(world: &mut QuectoWorld, args_str: String) {
    world.ep_coord_args_str = Some(args_str);
}

#[when("the coordinator args are parsed")]
fn when_coordinator_args_parsed(world: &mut QuectoWorld) {
    let args_str = world.ep_coord_args_str.as_ref().expect("args_str set");
    let args: Vec<String> = shell_split(args_str);
    world.ep_coord_parse_result = Some(parse_coordinator_args(&args));
}

#[then(regex = r#"^the coordinator ipc_dir should be "(.*)"$"#)]
fn then_coordinator_ipc_dir(world: &mut QuectoWorld, expected: String) {
    let result = world.ep_coord_parse_result.as_ref().expect("parse result");
    let args = result.as_ref().expect("should have parsed successfully");
    assert_eq!(args.ipc_dir, expected);
}

#[then(regex = r#"^the coordinator poll_interval_ms should be (\d+)$"#)]
fn then_coordinator_poll_interval(world: &mut QuectoWorld, expected: u64) {
    let result = world.ep_coord_parse_result.as_ref().expect("parse result");
    let args = result.as_ref().expect("should have parsed successfully");
    assert_eq!(args.poll_interval_ms, expected);
}

#[then(regex = r#"^the coordinator parse should fail with "(.*)"$"#)]
fn then_coordinator_parse_fail(world: &mut QuectoWorld, expected_msg: String) {
    let result = world.ep_coord_parse_result.as_ref().expect("parse result");
    let err = result.as_ref().expect_err("should have failed");
    assert!(
        err.contains(&expected_msg),
        "expected error containing '{expected_msg}', got: {err}"
    );
}

// ── Step definitions: Tick loop integration ─────────────────────────────

#[given("a coordinator entrypoint with mock service")]
fn given_coordinator_entrypoint_with_mock(world: &mut QuectoWorld) {
    world.ep_coord_ipc = Some(BddEntrypointMockIpc::new());
    world.ep_coord_svc = Some(BddEntrypointMockJobService::default());
}

#[given(regex = r#"^a pending coordinator inbox command "(.*)" with payload:$"#)]
fn given_pending_inbox_command(world: &mut QuectoWorld, step: &gherkin::Step, action: String) {
    let payload_str = step.docstring().expect("step should have a docstring");
    let ipc = world.ep_coord_ipc.as_ref().expect("ipc set");
    let payload: serde_json::Value =
        serde_json::from_str(payload_str.trim()).expect("valid JSON payload");
    ipc.add_command(&action, payload);
}

#[when("the coordinator runs one tick")]
fn when_coordinator_runs_one_tick(world: &mut QuectoWorld) {
    let ipc = world.ep_coord_ipc.as_ref().expect("ipc set");
    let svc = world.ep_coord_svc.as_mut().expect("svc set");
    let result = coordinator_inbox::tick(ipc, svc).expect("tick should succeed");
    world.ep_coord_tick_result = Some(result);
}

#[then(regex = r#"^the coordinator tick should process (\d+) commands?$"#)]
fn then_coordinator_tick_processed(world: &mut QuectoWorld, expected: usize) {
    let result = world.ep_coord_tick_result.as_ref().expect("tick result");
    assert_eq!(
        result.processed, expected,
        "expected {expected} processed, got {}",
        result.processed
    );
}

#[then("the coordinator tick should not request shutdown")]
fn then_coordinator_no_shutdown(world: &mut QuectoWorld) {
    let result = world.ep_coord_tick_result.as_ref().expect("tick result");
    assert!(!result.shutdown_requested);
}

#[then("the coordinator tick should request shutdown")]
fn then_coordinator_shutdown(world: &mut QuectoWorld) {
    let result = world.ep_coord_tick_result.as_ref().expect("tick result");
    assert!(result.shutdown_requested);
}

// ── Step definitions: PID writing ───────────────────────────────────────

#[when("the coordinator writes its PID")]
fn when_coordinator_writes_pid(world: &mut QuectoWorld) {
    let ipc = world.ep_coord_ipc.as_ref().expect("ipc set");
    let pid = std::process::id();
    ipc.write_pid(pid).expect("write_pid should succeed");
}

#[then("the coordinator PID file should contain the current process PID")]
fn then_coordinator_pid_matches(world: &mut QuectoWorld) {
    let ipc = world.ep_coord_ipc.as_ref().expect("ipc set");
    let stored_pid = ipc.read_pid().expect("read_pid").expect("pid present");
    assert_eq!(stored_pid, std::process::id());
}

// ── Step definitions: State snapshot ────────────────────────────────────

#[then("the coordinator state should show alive")]
fn then_coordinator_state_alive(world: &mut QuectoWorld) {
    let ipc = world.ep_coord_ipc.as_ref().expect("ipc set");
    let state = ipc
        .read_state()
        .expect("read_state")
        .expect("state present");
    assert!(state.alive, "coordinator state should show alive=true");
}

// ── Step definitions: Signal-driven shutdown ────────────────────────────

#[given("the coordinator external shutdown flag is set")]
fn given_external_shutdown_flag_set(world: &mut QuectoWorld) {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    world.ep_coord_shutdown_flag = Some(flag);
}

#[when("the coordinator loop runs with the external flag")]
fn when_coordinator_loop_runs_with_flag(world: &mut QuectoWorld) {
    let ipc = world.ep_coord_ipc.as_ref().expect("ipc set");
    let svc = world.ep_coord_svc.as_mut().expect("svc set");
    let flag = world
        .ep_coord_shutdown_flag
        .as_ref()
        .expect("shutdown flag set");

    let exit_code = quecto::interface::cli::coordinator::run_coordinator_loop_with_flag(
        ipc,
        svc,
        std::time::Duration::from_millis(10),
        flag,
    );
    world.ep_coord_loop_exit_code = Some(exit_code);
}

#[then(regex = r#"^the coordinator loop should exit with code (\d+)$"#)]
fn then_coordinator_loop_exit_code(world: &mut QuectoWorld, expected: i32) {
    let code = world.ep_coord_loop_exit_code.expect("exit code set");
    assert_eq!(code, expected, "expected exit code {expected}, got {code}");
}
