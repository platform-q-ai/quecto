//! Coordinator inbox processor — application-layer orchestration.
//!
//! Reads pending commands from the inbox, dispatches each to the
//! `CodingJobService`, writes responses to the outbox, acknowledges
//! processed commands, and writes state snapshots. Operates through
//! domain ports only (no direct I/O).

use crate::domain::coding_command::{
    CleanupRequest, CommandError, CreateRequest, ImportRequest, ListRequest, RunRequest,
};
use crate::domain::coding_ipc::{
    CoordinatorIpc, CoordinatorIpcCommand, CoordinatorIpcResponse, CoordinatorState,
};
use crate::domain::coding_ports::CodingJobService;

/// Result of a single `tick()` call.
#[derive(Debug, Clone)]
pub struct TickResult {
    /// Number of commands processed in this tick.
    pub processed: usize,
    /// Whether a "shutdown" command was received.
    pub shutdown_requested: bool,
}

/// Process all pending inbox commands against the job service, writing
/// responses to the outbox and acknowledging each command.
///
/// This is the core of the coordinator subagent's event loop. The caller
/// invokes `tick()` repeatedly (with a sleep interval) to drain the inbox.
pub fn tick(
    ipc: &dyn CoordinatorIpc,
    service: &mut dyn CodingJobService,
) -> Result<TickResult, String> {
    let commands = ipc.read_pending_commands()?;
    let mut processed = 0;
    let mut shutdown_requested = false;

    for cmd in &commands {
        let response = dispatch_command(cmd, service, &mut shutdown_requested);

        ipc.write_response(&response)
            .map_err(|e| format!("write_response({}): {e}", cmd.command_id))?;

        ipc.acknowledge_command(&cmd.command_id)
            .map_err(|e| format!("acknowledge({}): {e}", cmd.command_id))?;

        processed += 1;
    }

    // Only write state snapshot when commands were processed to avoid
    // unnecessary disk I/O on idle ticks (~172k writes/day at 500ms).
    if processed > 0 {
        let active_jobs = service.list(&ListRequest { state_filter: None }).jobs.len() as u32;
        let state = CoordinatorState {
            alive: true,
            active_jobs,
            last_heartbeat: String::new(), // Caller provides timestamp
            job_summary: serde_json::Value::Object(serde_json::Map::new()),
        };
        // Best-effort state write — don't fail the tick if it errors.
        let _ = ipc.write_state(&state);
    }

    Ok(TickResult {
        processed,
        shutdown_requested,
    })
}

/// Dispatch a single command to the job service and build a response.
fn dispatch_command(
    cmd: &CoordinatorIpcCommand,
    service: &mut dyn CodingJobService,
    shutdown_requested: &mut bool,
) -> CoordinatorIpcResponse {
    let result = match cmd.action.as_str() {
        "create" => handle_create(&cmd.payload, service),
        "import" => handle_import(&cmd.payload, service),
        "run" => handle_run(&cmd.payload, service),
        "status" => handle_status(&cmd.payload, service),
        "cancel" => handle_cancel(&cmd.payload, service),
        "cleanup" => handle_cleanup(&cmd.payload, service),
        "list" => handle_list(&cmd.payload, service),
        "shutdown" => {
            *shutdown_requested = true;
            Ok(serde_json::json!({"shutdown": true}))
        }
        other => Err(format!("unknown action: {other}")),
    };

    match result {
        Ok(body) => CoordinatorIpcResponse {
            command_id: cmd.command_id.clone(),
            ok: true,
            body: Some(body),
            error: None,
        },
        Err(e) => CoordinatorIpcResponse {
            command_id: cmd.command_id.clone(),
            ok: false,
            body: None,
            error: Some(e),
        },
    }
}

fn handle_create(
    payload: &serde_json::Value,
    service: &mut dyn CodingJobService,
) -> Result<serde_json::Value, String> {
    let req: CreateRequest = serde_json::from_value(strip_action(payload))
        .map_err(|e| format!("invalid create: {e}"))?;
    let resp = service
        .create_repo(req)
        .map_err(|e| format_command_error(&e))?;
    serde_json::to_value(&resp).map_err(|e| format!("serialize: {e}"))
}

fn handle_import(
    payload: &serde_json::Value,
    service: &mut dyn CodingJobService,
) -> Result<serde_json::Value, String> {
    let req: ImportRequest = serde_json::from_value(strip_action(payload))
        .map_err(|e| format!("invalid import: {e}"))?;
    let resp = service
        .import_repo(req)
        .map_err(|e| format_command_error(&e))?;
    serde_json::to_value(&resp).map_err(|e| format!("serialize: {e}"))
}

fn handle_run(
    payload: &serde_json::Value,
    service: &mut dyn CodingJobService,
) -> Result<serde_json::Value, String> {
    let req: RunRequest =
        serde_json::from_value(strip_action(payload)).map_err(|e| format!("invalid run: {e}"))?;
    let resp = service.run(req).map_err(|e| format_command_error(&e))?;
    serde_json::to_value(&resp).map_err(|e| format!("serialize: {e}"))
}

fn handle_status(
    payload: &serde_json::Value,
    service: &dyn CodingJobService,
) -> Result<serde_json::Value, String> {
    let job_id = payload.get("job_id").and_then(|v| v.as_str());
    let run_id = payload.get("run_id").and_then(|v| v.as_str());
    let resp = match (job_id, run_id) {
        (Some(jid), _) => service
            .status_by_job_id(jid)
            .map_err(|e| format_command_error(&e))?,
        (_, Some(rid)) => service
            .status_by_run_id(rid)
            .map_err(|e| format_command_error(&e))?,
        _ => return Err("status requires job_id or run_id".to_string()),
    };
    serde_json::to_value(&resp).map_err(|e| format!("serialize: {e}"))
}

fn handle_cancel(
    payload: &serde_json::Value,
    service: &mut dyn CodingJobService,
) -> Result<serde_json::Value, String> {
    let job_id = payload
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or("cancel requires job_id")?;
    let resp = service
        .cancel(job_id)
        .map_err(|e| format_command_error(&e))?;
    serde_json::to_value(&resp).map_err(|e| format!("serialize: {e}"))
}

fn handle_cleanup(
    payload: &serde_json::Value,
    service: &mut dyn CodingJobService,
) -> Result<serde_json::Value, String> {
    let req: CleanupRequest = serde_json::from_value(strip_action(payload))
        .map_err(|e| format!("invalid cleanup: {e}"))?;
    let resp = service
        .cleanup(&req.job_id, req.keep_artifacts)
        .map_err(|e| format_command_error(&e))?;
    serde_json::to_value(&resp).map_err(|e| format!("serialize: {e}"))
}

fn handle_list(
    payload: &serde_json::Value,
    service: &dyn CodingJobService,
) -> Result<serde_json::Value, String> {
    let req: ListRequest = if payload.is_null() || payload.as_object().is_some_and(|m| m.is_empty())
    {
        ListRequest { state_filter: None }
    } else {
        serde_json::from_value(strip_action(payload)).map_err(|e| format!("invalid list: {e}"))?
    };
    let resp = service.list(&req);
    serde_json::to_value(&resp).map_err(|e| format!("serialize: {e}"))
}

/// Strip the "action" field from a payload before deserializing into a
/// domain request struct (which doesn't have an "action" field).
///
/// Takes ownership to avoid a deep clone — the caller's payload is
/// consumed (it was already serialized into the response if needed).
fn strip_action(payload: &serde_json::Value) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(obj) if obj.contains_key("action") => {
            let mut stripped = obj.clone();
            stripped.remove("action");
            serde_json::Value::Object(stripped)
        }
        other => other.clone(),
    }
}

/// Format a `CommandError` into a human-readable string.
fn format_command_error(e: &CommandError) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coding_command::*;
    use crate::domain::coding_ipc::*;
    use crate::domain::coding_job::JobState;
    use std::sync::Mutex;

    // ---- Mock IPC ----

    #[derive(Debug)]
    struct TestIpc {
        commands: Mutex<Vec<CoordinatorIpcCommand>>,
        responses: Mutex<Vec<CoordinatorIpcResponse>>,
        acknowledged: Mutex<Vec<String>>,
        state: Mutex<Option<CoordinatorState>>,
    }

    impl TestIpc {
        fn new() -> Self {
            Self {
                commands: Mutex::new(vec![]),
                responses: Mutex::new(vec![]),
                acknowledged: Mutex::new(vec![]),
                state: Mutex::new(None),
            }
        }

        fn add_command(&self, action: &str, payload: serde_json::Value) -> String {
            let id = format!("cmd_{}", self.commands.lock().unwrap().len());
            self.commands.lock().unwrap().push(CoordinatorIpcCommand {
                command_id: id.clone(),
                action: action.to_string(),
                payload,
            });
            id
        }
    }

    impl CoordinatorIpc for TestIpc {
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
        fn read_response(
            &self,
            command_id: &str,
        ) -> Result<Option<CoordinatorIpcResponse>, String> {
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

    // ---- Mock CodingJobService ----

    #[derive(Debug, Default)]
    struct MockJobService {
        jobs: std::collections::HashMap<String, JobState>,
        next_job_id: u32,
        fail_with: Option<String>,
    }

    impl CodingJobService for MockJobService {
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
            if let Some(ref msg) = self.fail_with {
                return Err(CommandError::Internal(msg.clone()));
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
                    })
                    .collect(),
            }
        }
    }

    #[test]
    fn test_tick_empty_inbox() {
        let ipc = TestIpc::new();
        let mut svc = MockJobService::default();
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 0);
        assert!(!result.shutdown_requested);
    }

    #[test]
    fn test_tick_run_command() {
        let ipc = TestIpc::new();
        let cmd_id = ipc.add_command(
            "run",
            serde_json::json!({"goal":"test","repo":"r","base_ref":"main"}),
        );
        let mut svc = MockJobService::default();
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 1);
        assert!(!result.shutdown_requested);

        let responses = ipc.responses.lock().unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].command_id, cmd_id);
        assert!(responses[0].ok);
        let body = responses[0].body.as_ref().unwrap();
        assert!(body.get("job_id").is_some());
    }

    #[test]
    fn test_tick_list_command() {
        let ipc = TestIpc::new();
        ipc.add_command("list", serde_json::json!({}));
        let mut svc = MockJobService::default();
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 1);

        let responses = ipc.responses.lock().unwrap();
        assert!(responses[0].ok);
    }

    #[test]
    fn test_tick_status_command() {
        let ipc = TestIpc::new();
        ipc.add_command("status", serde_json::json!({"job_id":"j1"}));
        let mut svc = MockJobService::default();
        svc.jobs.insert("j1".to_string(), JobState::Running);
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 1);

        let responses = ipc.responses.lock().unwrap();
        assert!(responses[0].ok);
        let body_str = serde_json::to_string(responses[0].body.as_ref().unwrap()).unwrap();
        assert!(body_str.contains("running"));
    }

    #[test]
    fn test_tick_unknown_action() {
        let ipc = TestIpc::new();
        ipc.add_command("explode", serde_json::json!({}));
        let mut svc = MockJobService::default();
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 1);

        let responses = ipc.responses.lock().unwrap();
        assert!(!responses[0].ok);
        assert!(
            responses[0]
                .error
                .as_ref()
                .unwrap()
                .contains("unknown action")
        );
    }

    #[test]
    fn test_tick_service_error() {
        let ipc = TestIpc::new();
        ipc.add_command("status", serde_json::json!({"job_id":"missing"}));
        let mut svc = MockJobService::default();
        // No job "missing" exists, so status_by_job_id returns NotFound
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 1);

        let responses = ipc.responses.lock().unwrap();
        assert!(!responses[0].ok);
        assert!(responses[0].error.as_ref().unwrap().contains("not_found"));
    }

    #[test]
    fn test_tick_shutdown() {
        let ipc = TestIpc::new();
        ipc.add_command("shutdown", serde_json::json!({}));
        let mut svc = MockJobService::default();
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 1);
        assert!(result.shutdown_requested);

        let responses = ipc.responses.lock().unwrap();
        assert!(responses[0].ok);
    }

    #[test]
    fn test_tick_multiple_commands() {
        let ipc = TestIpc::new();
        ipc.add_command("list", serde_json::json!({}));
        ipc.add_command("list", serde_json::json!({}));
        let mut svc = MockJobService::default();
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 2);

        let responses = ipc.responses.lock().unwrap();
        assert_eq!(responses.len(), 2);
        assert!(responses.iter().all(|r| r.ok));
    }

    #[test]
    fn test_tick_writes_state() {
        let ipc = TestIpc::new();
        ipc.add_command("list", serde_json::json!({}));
        let mut svc = MockJobService::default();
        tick(&ipc, &mut svc).expect("tick");

        let state = ipc.state.lock().unwrap();
        assert!(state.is_some());
        let s = state.as_ref().unwrap();
        assert!(s.alive);
    }

    #[test]
    fn test_tick_acknowledges_commands() {
        let ipc = TestIpc::new();
        let cmd_id = ipc.add_command("list", serde_json::json!({}));
        let mut svc = MockJobService::default();
        tick(&ipc, &mut svc).expect("tick");

        let acked = ipc.acknowledged.lock().unwrap();
        assert_eq!(acked.len(), 1);
        assert_eq!(acked[0], cmd_id);
    }

    #[test]
    fn test_strip_action() {
        let payload = serde_json::json!({"action": "run", "goal": "test"});
        let stripped = strip_action(&payload);
        assert!(stripped.get("action").is_none());
        assert_eq!(stripped.get("goal").unwrap().as_str().unwrap(), "test");
    }

    #[test]
    fn test_cancel_command() {
        let ipc = TestIpc::new();
        ipc.add_command("cancel", serde_json::json!({"job_id":"j1"}));
        let mut svc = MockJobService::default();
        svc.jobs.insert("j1".to_string(), JobState::Running);
        let result = tick(&ipc, &mut svc).expect("tick");
        assert_eq!(result.processed, 1);

        let responses = ipc.responses.lock().unwrap();
        assert!(responses[0].ok);
    }
}
