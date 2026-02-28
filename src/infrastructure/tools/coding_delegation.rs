//! Coordinator delegation tool: thin IPC bridge for the main agent.
//!
//! Replaces `CodingJobTool` in the main agent process. Instead of dispatching
//! to an in-process `CodingJobService`, it writes command JSON to the
//! coordinator's inbox and reads responses from the outbox.
//!
//! The tool name and schema remain "coding_job" for backward compatibility.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::coding_ipc::{
    CoordinatorIpc, CoordinatorIpcCommand, CoordinatorIpcResponse, CoordinatorSpawner,
    notification_filename,
};
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Known actions that the delegation tool accepts.
const KNOWN_ACTIONS: &[&str] = &[
    "create",
    "import",
    "run",
    "status",
    "cancel",
    "cleanup",
    "cleanup_all",
    "list",
    "shutdown",
];

/// Tool that delegates coding job commands to the coordinator via file-based IPC.
///
/// The main agent uses this instead of `CodingJobTool`. It writes commands
/// to `coordinator/inbox/` and polls `coordinator/outbox/` for responses.
pub struct CoordinatorDelegationTool {
    ipc: Arc<dyn CoordinatorIpc>,
    /// Optional spawner for auto-starting the coordinator process.
    spawner: Option<Arc<dyn CoordinatorSpawner>>,
    /// Maximum time to wait for a response, in milliseconds.
    poll_timeout_ms: u64,
    /// Maximum number of poll attempts before timing out.
    poll_max_attempts: u32,
}

impl std::fmt::Debug for CoordinatorDelegationTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoordinatorDelegationTool").finish()
    }
}

impl CoordinatorDelegationTool {
    pub fn new(ipc: Arc<dyn CoordinatorIpc>) -> Self {
        Self {
            ipc,
            spawner: None,
            poll_timeout_ms: 50,
            poll_max_attempts: 600, // 30 seconds at 50ms intervals
        }
    }

    /// Create with an auto-spawner that ensures the coordinator is alive.
    pub fn with_spawner(
        ipc: Arc<dyn CoordinatorIpc>,
        spawner: Arc<dyn CoordinatorSpawner>,
    ) -> Self {
        Self {
            ipc,
            spawner: Some(spawner),
            poll_timeout_ms: 50,
            poll_max_attempts: 600,
        }
    }

    /// Create with custom polling parameters (for testing).
    pub fn with_polling(ipc: Arc<dyn CoordinatorIpc>, timeout_ms: u64, max_attempts: u32) -> Self {
        Self {
            ipc,
            spawner: None,
            poll_timeout_ms: timeout_ms,
            poll_max_attempts: max_attempts,
        }
    }

    /// Create with spawner and custom polling parameters (for testing).
    pub fn with_spawner_and_polling(
        ipc: Arc<dyn CoordinatorIpc>,
        spawner: Arc<dyn CoordinatorSpawner>,
        timeout_ms: u64,
        max_attempts: u32,
    ) -> Self {
        Self {
            ipc,
            spawner: Some(spawner),
            poll_timeout_ms: timeout_ms,
            poll_max_attempts: max_attempts,
        }
    }

    fn handle_action(&self, arguments: &str) -> Result<String, String> {
        // Auto-spawn the coordinator if a spawner is configured.
        if let Some(ref spawner) = self.spawner {
            spawner.ensure_alive()?;
        }

        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {e}"))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: action")?;

        if !KNOWN_ACTIONS.contains(&action) {
            return Err(format!("unknown action: {action}"));
        }

        // Build IPC command
        let command_id = format!("cmd_{}", uuid::Uuid::new_v4().as_simple());
        let cmd = CoordinatorIpcCommand {
            command_id: command_id.clone(),
            action: action.to_string(),
            payload: args.clone(),
        };

        // Write command to inbox
        self.ipc
            .write_command(&cmd)
            .map_err(|e| format!("ipc write: {e}"))?;

        // Poll outbox for response
        let response = self.poll_response(&command_id)?;

        // Check for pending notifications and acknowledge them
        let notifications = self.ipc.read_notifications().unwrap_or_default();
        for notif in &notifications {
            let filename = notification_filename(notif);
            let _ = self.ipc.acknowledge_notification(&filename);
        }

        // Build result
        if response.ok {
            let mut result_value = response
                .body
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

            // Attach notifications if any
            if !notifications.is_empty() {
                if let Some(obj) = result_value.as_object_mut() {
                    let notifs_json: Vec<serde_json::Value> = notifications
                        .iter()
                        .map(|n| serde_json::to_value(n).unwrap_or_default())
                        .collect();
                    obj.insert(
                        "notifications".to_string(),
                        serde_json::Value::Array(notifs_json),
                    );
                }
            }

            serde_json::to_string(&result_value).map_err(|e| format!("serialize: {e}"))
        } else {
            Err(format!(
                "error: {}",
                response.error.unwrap_or_else(|| "unknown".to_string())
            ))
        }
    }

    fn poll_response(&self, command_id: &str) -> Result<CoordinatorIpcResponse, String> {
        let mut delay_ms = self.poll_timeout_ms;
        for _ in 0..self.poll_max_attempts {
            match self.ipc.read_response(command_id) {
                Ok(Some(resp)) => return Ok(resp),
                Ok(None) => {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    // Exponential backoff: double the delay each miss, cap at 500ms
                    delay_ms = (delay_ms * 2).min(500);
                }
                Err(e) => return Err(format!("ipc read: {e}")),
            }
        }
        Err("timeout: coordinator did not respond".to_string())
    }
}

impl Tool for CoordinatorDelegationTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "coding_job".to_string(),
            description: CODING_JOB_DESCRIPTION.to_string(),
            parameters_schema: CODING_JOB_SCHEMA.to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            // Run synchronous IPC in a blocking task to avoid blocking the tokio runtime
            let result = tokio::task::spawn_blocking({
                let ipc = self.ipc.clone();
                let spawner = self.spawner.clone();
                let poll_timeout_ms = self.poll_timeout_ms;
                let poll_max_attempts = self.poll_max_attempts;
                move || {
                    let tool = CoordinatorDelegationTool {
                        ipc,
                        spawner,
                        poll_timeout_ms,
                        poll_max_attempts,
                    };
                    tool.handle_action(&args)
                }
            })
            .await;

            match result {
                Ok(Ok(content)) => Ok(ToolResult {
                    content,
                    is_error: false,
                }),
                Ok(Err(e)) => Ok(ToolResult {
                    content: e,
                    is_error: true,
                }),
                Err(_) => Ok(ToolResult {
                    content: "coding_job delegation task failed".to_string(),
                    is_error: true,
                }),
            }
        })
    }
}

// Same description and schema as CodingJobTool for backward compatibility.
const CODING_JOB_DESCRIPTION: &str = "\
Manage coding repositories and jobs. Repos live in the workspace directory.

WORKFLOW:
1. create     - Create a new empty repo (git init + initial commit) in the workspace.
2. import     - Clone a remote repo (HTTPS/SSH) into the workspace.
3. run        - Launch an async coding job on a workspace repo. A sandboxed worker \
                gets a full clone, checks out a job branch, and works toward the goal \
                using edit/grep/find/read tools. Returns job_id and run_id.
4. status     - Poll job progress (state, todos, artifacts, last_event_ts). \
                Each call advances the job internally.
5. cancel     - Stop a running job and kill its worker.
6. cleanup    - Remove a single terminal job's artifacts.
7. cleanup_all- Remove all terminal jobs in bulk (filter by state, skip non-terminal).
8. list       - List jobs with metadata (created_at, state_entered_at, last_event_ts).

REPO RULES:
- 'repo' must be a directory name in the workspace (e.g. \"my-project\"), not a URL.
- Use 'create' to make a new repo from scratch, or 'import' to clone from GitHub.
- 'base_ref' must be a valid branch/tag/commit in the repo (e.g. \"main\"). \
  On invalid_base_ref the error includes the default branch and available refs.
- Each run() targets exactly one repo. Multi-repo goals require multiple jobs.

JOB STATES: queued -> preparing -> running -> succeeded/failed/canceled
- 'running' jobs are automatically killed if max_wall_seconds elapses.

VISIBILITY:
- status() returns last_event_ts/last_event_type so you can detect hung jobs.
- list() returns created_at and state_entered_at; use (now - state_entered_at) \
  to detect jobs that have been stuck in 'running' longer than expected.

TYPICAL USAGE:
- New project:      create(name) -> run(repo=name, base_ref=\"main\", goal=\"...\")
- Existing remote:  import(url) -> run(repo=name, base_ref=\"main\", goal=\"...\")
- Monitor:          status(job_id) repeatedly until succeeded/failed
- Bulk cleanup:     cleanup_all() or cleanup_all(state_filter=[\"succeeded\",\"failed\"])
- Done:             cleanup(job_id)";

const CODING_JOB_SCHEMA: &str = r#"{"type":"object","properties":{"action":{"type":"string","enum":["create","import","run","status","cancel","cleanup","cleanup_all","list"],"description":"The action to perform"},"name":{"type":"string","description":"Repository name (for create/import)"},"description":{"type":"string","description":"Project description (for create, optional)"},"url":{"type":"string","description":"Remote git URL to clone (for import)"},"goal":{"type":"string","description":"Goal description (for run)"},"repo":{"type":"string","description":"Workspace repo name (for run)"},"base_ref":{"type":"string","description":"Base branch/ref (for run, e.g. main)"},"priority":{"type":"string","enum":["low","medium","high"],"description":"Job priority (for run, default: medium)"},"labels":{"type":"array","items":{"type":"string"},"description":"Labels (for run)"},"skills":{"type":"array","items":{"type":"string"},"description":"Skill names (for run)"},"profile":{"type":"string","description":"Profile name (for run, default: default)"},"max_wall_seconds":{"type":"integer","description":"Wall-clock timeout in seconds (for run)"},"job_id":{"type":"string","description":"Job ID (for status/cancel/cleanup)"},"run_id":{"type":"string","description":"Run ID (for status)"},"state_filter":{"type":"array","items":{"type":"string"},"description":"Filter by job states (for list/cleanup_all)"},"keep_artifacts":{"type":"boolean","description":"Keep artifacts on cleanup/cleanup_all (default: true)"},"terminal_only":{"type":"boolean","description":"Skip non-terminal jobs in cleanup_all instead of erroring (default: true)"}},"required":["action"]}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coding_ipc::*;
    use std::sync::Mutex;

    /// Mock IPC that records commands and returns pre-configured responses.
    #[derive(Debug)]
    struct MockIpc {
        commands: Mutex<Vec<CoordinatorIpcCommand>>,
        response: Mutex<Option<CoordinatorIpcResponse>>,
        notifications: Mutex<Vec<CoordinatorNotification>>,
        timeout: bool,
    }

    impl MockIpc {
        fn new() -> Self {
            Self {
                commands: Mutex::new(vec![]),
                response: Mutex::new(None),
                notifications: Mutex::new(vec![]),
                timeout: false,
            }
        }

        fn with_response(ok: bool, body: Option<serde_json::Value>, error: Option<String>) -> Self {
            let ipc = Self::new();
            *ipc.response.lock().unwrap() = Some(CoordinatorIpcResponse {
                command_id: String::new(), // will be overwritten
                ok,
                body,
                error,
            });
            ipc
        }

        fn with_timeout() -> Self {
            Self {
                commands: Mutex::new(vec![]),
                response: Mutex::new(None),
                notifications: Mutex::new(vec![]),
                timeout: true,
            }
        }
    }

    impl CoordinatorIpc for MockIpc {
        fn write_command(&self, cmd: &CoordinatorIpcCommand) -> Result<(), String> {
            self.commands.lock().unwrap().push(cmd.clone());
            // Auto-populate response command_id
            if let Some(resp) = self.response.lock().unwrap().as_mut() {
                resp.command_id = cmd.command_id.clone();
            }
            Ok(())
        }

        fn read_pending_commands(&self) -> Result<Vec<CoordinatorIpcCommand>, String> {
            Ok(self.commands.lock().unwrap().clone())
        }

        fn acknowledge_command(&self, _command_id: &str) -> Result<(), String> {
            Ok(())
        }

        fn write_response(&self, _resp: &CoordinatorIpcResponse) -> Result<(), String> {
            Ok(())
        }

        fn read_response(
            &self,
            _command_id: &str,
        ) -> Result<Option<CoordinatorIpcResponse>, String> {
            if self.timeout {
                return Ok(None);
            }
            Ok(self.response.lock().unwrap().clone())
        }

        fn write_notification(&self, _notif: &CoordinatorNotification) -> Result<(), String> {
            Ok(())
        }

        fn read_notifications(&self) -> Result<Vec<CoordinatorNotification>, String> {
            Ok(self.notifications.lock().unwrap().clone())
        }

        fn acknowledge_notification(&self, _filename: &str) -> Result<(), String> {
            Ok(())
        }

        fn write_state(&self, _state: &CoordinatorState) -> Result<(), String> {
            Ok(())
        }

        fn read_state(&self) -> Result<Option<CoordinatorState>, String> {
            Ok(None)
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

    fn make_tool_ok(body: serde_json::Value) -> CoordinatorDelegationTool {
        let ipc = Arc::new(MockIpc::with_response(true, Some(body), None));
        CoordinatorDelegationTool::with_polling(ipc, 1, 3)
    }

    fn make_tool_err(error: &str) -> CoordinatorDelegationTool {
        let ipc = Arc::new(MockIpc::with_response(false, None, Some(error.to_string())));
        CoordinatorDelegationTool::with_polling(ipc, 1, 3)
    }

    fn make_tool_timeout() -> CoordinatorDelegationTool {
        let ipc = Arc::new(MockIpc::with_timeout());
        CoordinatorDelegationTool::with_polling(ipc, 1, 2) // 2 attempts at 1ms = fast timeout
    }

    fn exec(tool: &CoordinatorDelegationTool, input: &str) -> ToolResult {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(tool.execute(input)).expect("should not panic")
    }

    #[test]
    fn test_definition_name() {
        let tool = make_tool_ok(serde_json::json!({}));
        assert_eq!(tool.definition().name, "coding_job");
    }

    #[test]
    fn test_definition_schema_has_action() {
        let tool = make_tool_ok(serde_json::json!({}));
        assert!(tool.definition().parameters_schema.contains("action"));
    }

    #[test]
    fn test_definition_description() {
        let tool = make_tool_ok(serde_json::json!({}));
        assert!(tool.definition().description.contains("WORKFLOW"));
    }

    #[test]
    fn test_run_action() {
        let tool =
            make_tool_ok(serde_json::json!({"run_id": "r1", "job_id": "j1", "state": "queued"}));
        let r = exec(
            &tool,
            r#"{"action":"run","goal":"Fix","repo":"test","base_ref":"main"}"#,
        );
        assert!(!r.is_error);
        assert!(r.content.contains("job_id"));
    }

    #[test]
    fn test_status_action() {
        let tool = make_tool_ok(serde_json::json!({"state": "running", "progress": 50}));
        let r = exec(&tool, r#"{"action":"status","job_id":"j1"}"#);
        assert!(!r.is_error);
        assert!(r.content.contains("running"));
    }

    #[test]
    fn test_cancel_action() {
        let tool = make_tool_ok(serde_json::json!({"job_id": "j1", "state": "canceled"}));
        let r = exec(&tool, r#"{"action":"cancel","job_id":"j1"}"#);
        assert!(!r.is_error);
        assert!(r.content.contains("canceled"));
    }

    #[test]
    fn test_list_action() {
        let tool = make_tool_ok(serde_json::json!({"jobs": []}));
        let r = exec(&tool, r#"{"action":"list"}"#);
        assert!(!r.is_error);
        assert!(r.content.contains("jobs"));
    }

    #[test]
    fn test_error_response() {
        let tool = make_tool_err("not_found");
        let r = exec(&tool, r#"{"action":"status","job_id":"miss"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("not_found"));
    }

    #[test]
    fn test_timeout() {
        let tool = make_tool_timeout();
        let r = exec(&tool, r#"{"action":"status","job_id":"j1"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("timeout"));
    }

    #[test]
    fn test_invalid_json() {
        let tool = make_tool_ok(serde_json::json!({}));
        let r = exec(&tool, "not json");
        assert!(r.is_error);
        assert!(r.content.contains("invalid JSON"));
    }

    #[test]
    fn test_missing_action() {
        let tool = make_tool_ok(serde_json::json!({}));
        let r = exec(&tool, r#"{"goal":"t"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("action"));
    }

    #[test]
    fn test_unknown_action() {
        let tool = make_tool_ok(serde_json::json!({}));
        let r = exec(&tool, r#"{"action":"explode"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("unknown action"));
    }

    #[test]
    fn test_command_written_to_inbox() {
        let mock = Arc::new(MockIpc::with_response(
            true,
            Some(serde_json::json!({"ok": true})),
            None,
        ));
        let tool = CoordinatorDelegationTool::with_polling(mock.clone(), 1, 3);
        exec(&tool, r#"{"action":"run","goal":"test"}"#);
        let cmds = mock.commands.lock().unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, "run");
    }

    #[test]
    fn test_notifications_included_in_response() {
        let mock = Arc::new(MockIpc::with_response(
            true,
            Some(serde_json::json!({"state": "failed"})),
            None,
        ));
        mock.notifications
            .lock()
            .unwrap()
            .push(CoordinatorNotification {
                notification_type: NotificationType::JobFailed,
                job_id: Some("j1".to_string()),
                job_ids: vec![],
                detail: Some("OOM".to_string()),
                no_progress_minutes: None,
                ts: "2026-01-15T10:00:00Z".to_string(),
            });
        let tool = CoordinatorDelegationTool::with_polling(mock, 1, 3);
        let r = exec(&tool, r#"{"action":"status","job_id":"j1"}"#);
        assert!(!r.is_error);
        assert!(r.content.contains("notifications"));
        assert!(r.content.contains("job_failed"));
    }

    #[test]
    fn test_debug_format() {
        let tool = make_tool_ok(serde_json::json!({}));
        assert!(format!("{tool:?}").contains("CoordinatorDelegationTool"));
    }

    // --- Auto-spawn integration tests ---

    /// Mock spawner that records calls and returns configurable results.
    #[derive(Debug)]
    struct MockSpawner {
        alive: bool,
        existing_pid: u32,
        spawned_pid: u32,
        fail: bool,
        calls: Mutex<u32>,
        spawns: Mutex<u32>,
    }

    impl MockSpawner {
        fn already_alive(pid: u32) -> Self {
            Self {
                alive: true,
                existing_pid: pid,
                spawned_pid: 0,
                fail: false,
                calls: Mutex::new(0),
                spawns: Mutex::new(0),
            }
        }

        fn needs_spawn(new_pid: u32) -> Self {
            Self {
                alive: false,
                existing_pid: 0,
                spawned_pid: new_pid,
                fail: false,
                calls: Mutex::new(0),
                spawns: Mutex::new(0),
            }
        }

        fn failing() -> Self {
            Self {
                alive: false,
                existing_pid: 0,
                spawned_pid: 0,
                fail: true,
                calls: Mutex::new(0),
                spawns: Mutex::new(0),
            }
        }
    }

    impl CoordinatorSpawner for MockSpawner {
        fn ensure_alive(&self) -> Result<SpawnResult, String> {
            *self.calls.lock().unwrap() += 1;
            if self.fail {
                return Err("spawn failed: mock".to_string());
            }
            if self.alive {
                Ok(SpawnResult {
                    pid: self.existing_pid,
                    spawned: false,
                })
            } else {
                *self.spawns.lock().unwrap() += 1;
                Ok(SpawnResult {
                    pid: self.spawned_pid,
                    spawned: true,
                })
            }
        }
    }

    #[test]
    fn test_auto_spawn_calls_spawner() {
        let ipc = Arc::new(MockIpc::with_response(
            true,
            Some(serde_json::json!({"ok": true})),
            None,
        ));
        let spawner = Arc::new(MockSpawner::needs_spawn(999));
        let tool = CoordinatorDelegationTool::with_spawner_and_polling(ipc, spawner.clone(), 1, 3);
        let r = exec(&tool, r#"{"action":"list"}"#);
        assert!(!r.is_error);
        assert_eq!(*spawner.calls.lock().unwrap(), 1);
        assert_eq!(*spawner.spawns.lock().unwrap(), 1);
    }

    #[test]
    fn test_auto_spawn_skips_when_alive() {
        let ipc = Arc::new(MockIpc::with_response(
            true,
            Some(serde_json::json!({"ok": true})),
            None,
        ));
        let spawner = Arc::new(MockSpawner::already_alive(42));
        let tool = CoordinatorDelegationTool::with_spawner_and_polling(ipc, spawner.clone(), 1, 3);
        let r = exec(&tool, r#"{"action":"list"}"#);
        assert!(!r.is_error);
        assert_eq!(*spawner.calls.lock().unwrap(), 1);
        assert_eq!(*spawner.spawns.lock().unwrap(), 0);
    }

    #[test]
    fn test_auto_spawn_failure_returns_error() {
        let ipc = Arc::new(MockIpc::with_response(
            true,
            Some(serde_json::json!({"ok": true})),
            None,
        ));
        let spawner = Arc::new(MockSpawner::failing());
        let tool = CoordinatorDelegationTool::with_spawner_and_polling(ipc, spawner, 1, 3);
        let r = exec(&tool, r#"{"action":"list"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("spawn failed"));
    }

    #[test]
    fn test_no_spawner_works_normally() {
        // Default construction (no spawner) should work without auto-spawn
        let tool = make_tool_ok(serde_json::json!({"ok": true}));
        let r = exec(&tool, r#"{"action":"list"}"#);
        assert!(!r.is_error);
    }
}
