// Coding job tool: bridge between the agent loop and the CodingJobService.
//
// Accepts JSON with an "action" field and dispatches to the appropriate
// service method. Returns JSON-serialized responses.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::coding_command::{
    CommandError, CreateRequest, ImportRequest, ListRequest, RunRequest,
};
use crate::domain::coding_ports::CodingJobService;
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Tool that lets the LLM agent manage coding jobs through the service port.
///
/// The tool holds a shared reference to the service behind a mutex,
/// since `Tool::execute` takes `&self` but service methods need `&mut self`.
///
/// NOTE: The underlying service's job maps grow until `cleanup()` is called.
/// Callers (the LLM agent) must call cleanup on terminal jobs to reclaim
/// memory. A max-jobs policy or auto-eviction is planned for a future PR.
pub struct CodingJobTool {
    // NOTE: std::sync::Mutex is intentional — all operations under the lock
    // are synchronous (HashMap lookups, state transitions, event emission).
    // If this service is ever shared across concurrent agent loops, switch
    // to tokio::sync::Mutex or acquire-compute-release to avoid blocking
    // the tokio worker thread.
    service: Arc<Mutex<dyn CodingJobService>>,
}

impl std::fmt::Debug for CodingJobTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingJobTool").finish()
    }
}

impl CodingJobTool {
    pub fn new(service: Arc<Mutex<dyn CodingJobService>>) -> Self {
        Self { service }
    }

    fn handle_action(&self, arguments: &str) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {e}"))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: action")?;

        match action {
            "create" => self.handle_create(arguments),
            "import" => self.handle_import(arguments),
            "run" => self.handle_run(arguments),
            "status" => self.handle_status(&args),
            "cancel" => self.handle_cancel(&args),
            "cleanup" => self.handle_cleanup(&args),
            "cleanup_all" => self.handle_cleanup_all(arguments),
            "list" => self.handle_list(arguments),
            "message" => Err("message action requires subagent coordinator mode. \
                 Set tools.coding.coordinator_mode = \"subagent\" in config."
                .to_string()),
            other => Err(format!("unknown action: {other}")),
        }
    }

    fn handle_create(&self, raw: &str) -> Result<String, String> {
        let req: CreateRequest = deserialize_stripping_action(raw)
            .map_err(|e| format!("invalid create request: {e}"))?;
        let mut svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = svc.create_repo(req).map_err(|e| format_command_error(&e))?;
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }

    fn handle_import(&self, raw: &str) -> Result<String, String> {
        let req: ImportRequest = deserialize_stripping_action(raw)
            .map_err(|e| format!("invalid import request: {e}"))?;
        let mut svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = svc.import_repo(req).map_err(|e| format_command_error(&e))?;
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }

    fn handle_run(&self, raw: &str) -> Result<String, String> {
        let req: RunRequest =
            deserialize_stripping_action(raw).map_err(|e| format!("invalid run request: {e}"))?;
        let mut svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = svc.run(req).map_err(|e| format_command_error(&e))?;
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }

    fn handle_status(&self, args: &serde_json::Value) -> Result<String, String> {
        let job_id = args.get("job_id").and_then(|v| v.as_str());
        let run_id = args.get("run_id").and_then(|v| v.as_str());
        let svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = match (job_id, run_id) {
            (Some(jid), _) => svc
                .status_by_job_id(jid)
                .map_err(|e| format_command_error(&e))?,
            (_, Some(rid)) => svc
                .status_by_run_id(rid)
                .map_err(|e| format_command_error(&e))?,
            _ => return Err("status requires job_id or run_id".to_string()),
        };
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }

    fn handle_cancel(&self, args: &serde_json::Value) -> Result<String, String> {
        let job_id = args
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or("cancel requires job_id")?;
        let mut svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = svc.cancel(job_id).map_err(|e| format_command_error(&e))?;
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }

    fn handle_cleanup(&self, args: &serde_json::Value) -> Result<String, String> {
        let job_id = args
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or("cleanup requires job_id")?;
        let keep_artifacts = args
            .get("keep_artifacts")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let mut svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = svc
            .cleanup(job_id, keep_artifacts)
            .map_err(|e| format_command_error(&e))?;
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }

    fn handle_cleanup_all(&self, raw: &str) -> Result<String, String> {
        use crate::domain::coding_command::CleanupAllRequest;
        let req: CleanupAllRequest = deserialize_stripping_action(raw)
            .map_err(|e| format!("invalid cleanup_all request: {e}"))?;
        let mut svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = svc
            .cleanup_all(&req)
            .map_err(|e| format_command_error(&e))?;
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }

    fn handle_list(&self, raw: &str) -> Result<String, String> {
        let req: ListRequest =
            deserialize_stripping_action(raw).map_err(|e| format!("invalid list request: {e}"))?;
        let svc = self.service.lock().map_err(|e| format!("lock: {e}"))?;
        let resp = svc.list(&req);
        serde_json::to_string(&resp).map_err(|e| format!("serialize: {e}"))
    }
}

/// Deserialize `T` from a JSON string, first stripping the `"action"` key.
///
/// This avoids cloning the entire `serde_json::Value` tree — we parse once
/// into a `Value`, remove the key in-place, then deserialize into `T`.
fn deserialize_stripping_action<T: serde::de::DeserializeOwned>(
    raw: &str,
) -> Result<T, serde_json::Error> {
    let mut val: serde_json::Value = serde_json::from_str(raw)?;
    if let Some(obj) = val.as_object_mut() {
        obj.remove("action");
    }
    serde_json::from_value(val)
}

fn format_command_error(err: &CommandError) -> String {
    format!("error: {err}")
}

impl Tool for CodingJobTool {
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
        let service = self.service.clone();
        Box::pin(async move {
            let outcome = tokio::task::spawn_blocking(move || {
                let tool = CodingJobTool { service };
                tool.handle_action(&args)
            })
            .await;

            match outcome {
                Ok(Ok(content)) => Ok(ToolResult {
                    content,
                    is_error: false,
                }),
                Ok(Err(e)) => Ok(ToolResult {
                    content: e,
                    is_error: true,
                }),
                Err(_) => Ok(ToolResult {
                    content: "coding_job task failed".to_string(),
                    is_error: true,
                }),
            }
        })
    }
}

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
9. message    - Send a freeform text instruction to the coordinator agent. Use this \
                to triage issues, ask the coordinator to investigate stuck jobs, \
                change priorities, or give any open-ended directive. \
                (Requires subagent coordinator mode.)

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
- Triage:           message(text=\"job X seems stuck, investigate and retry if needed\")
- Bulk cleanup:     cleanup_all() or cleanup_all(state_filter=[\"succeeded\",\"failed\"])
- Done:             cleanup(job_id)";

const CODING_JOB_SCHEMA: &str = r#"{"type":"object","properties":{"action":{"type":"string","enum":["create","import","run","status","cancel","cleanup","cleanup_all","list","message"],"description":"The action to perform"},"name":{"type":"string","description":"Repository name (for create/import)"},"description":{"type":"string","description":"Project description (for create, optional)"},"url":{"type":"string","description":"Remote git URL to clone (for import)"},"goal":{"type":"string","description":"Goal description (for run)"},"repo":{"type":"string","description":"Workspace repo name (for run)"},"base_ref":{"type":"string","description":"Base branch/ref (for run, e.g. main)"},"priority":{"type":"string","enum":["low","medium","high"],"description":"Job priority (for run, default: medium)"},"labels":{"type":"array","items":{"type":"string"},"description":"Labels (for run)"},"skills":{"type":"array","items":{"type":"string"},"description":"Skill names (for run)"},"profile":{"type":"string","description":"Profile name (for run, default: default)"},"max_wall_seconds":{"type":"integer","description":"Wall-clock timeout in seconds (for run)"},"job_id":{"type":"string","description":"Job ID (for status/cancel/cleanup)"},"run_id":{"type":"string","description":"Run ID (for status)"},"state_filter":{"type":"array","items":{"type":"string"},"description":"Filter by job states (for list/cleanup_all)"},"keep_artifacts":{"type":"boolean","description":"Keep artifacts on cleanup/cleanup_all (default: true)"},"terminal_only":{"type":"boolean","description":"Skip non-terminal jobs in cleanup_all instead of erroring (default: true)"},"text":{"type":"string","description":"Freeform text instruction for the coordinator (for message)"}},"required":["action"]}"#;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::application::coding_coordinator::{CodingCoordinator, CoordinatorPolicy};
    use crate::domain::coding_ports::{CodingJobService, RepoValidator, SkillResolver};
    use crate::domain::tool::Tool;

    use super::CodingJobTool;

    #[derive(Debug, Clone)]
    struct TestRepo;
    impl RepoValidator for TestRepo {
        fn repo_exists(&self, repo: &str) -> bool {
            repo == "test-repo"
        }
        fn ref_exists(&self, repo: &str, r: &str) -> bool {
            repo == "test-repo" && r == "main"
        }
    }

    #[derive(Debug, Clone)]
    struct TestSkills;
    impl SkillResolver for TestSkills {
        fn skill_exists(&self, _: &str) -> bool {
            true
        }
    }

    fn make_tool() -> CodingJobTool {
        let coord = CodingCoordinator::new(TestRepo, TestSkills, CoordinatorPolicy::default());
        let svc: Arc<Mutex<dyn CodingJobService>> = Arc::new(Mutex::new(coord));
        CodingJobTool::new(svc)
    }

    fn exec(tool: &CodingJobTool, input: &str) -> crate::domain::tool::ToolResult {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(tool.execute(input)).expect("should not panic")
    }

    fn create_job(tool: &CodingJobTool) -> String {
        let r = exec(
            tool,
            r#"{"action":"run","goal":"t","repo":"test-repo","base_ref":"main"}"#,
        );
        assert!(!r.is_error, "run failed: {}", r.content);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        v["job_id"].as_str().unwrap().to_string()
    }

    #[test]
    fn test_definition_name() {
        assert_eq!(make_tool().definition().name, "coding_job");
    }

    #[test]
    fn test_definition_schema_has_action() {
        let def = make_tool().definition();
        assert!(def.parameters_schema.contains("action"));
        assert!(def.parameters_schema.contains("required"));
    }

    #[test]
    fn test_definition_description() {
        let desc = make_tool().definition().description;
        assert!(desc.contains("WORKFLOW"));
        assert!(desc.contains("create"));
        assert!(desc.contains("import"));
        assert!(desc.contains("run"));
    }

    #[test]
    fn test_definition_schema_has_create_import() {
        let schema = make_tool().definition().parameters_schema;
        assert!(schema.contains("\"create\""));
        assert!(schema.contains("\"import\""));
        assert!(schema.contains("\"url\""));
        assert!(schema.contains("\"name\""));
    }

    // create/import on bare coordinator returns Internal error (not supported)
    #[test]
    fn test_create_on_bare_coordinator() {
        let r = exec(&make_tool(), r#"{"action":"create","name":"my-proj"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("internal"));
    }

    #[test]
    fn test_import_on_bare_coordinator() {
        let r = exec(
            &make_tool(),
            r#"{"action":"import","url":"https://github.com/org/repo"}"#,
        );
        assert!(r.is_error);
        assert!(r.content.contains("internal"));
    }

    #[test]
    fn test_create_missing_name() {
        let r = exec(&make_tool(), r#"{"action":"create"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("invalid create request"));
    }

    #[test]
    fn test_import_missing_url() {
        let r = exec(&make_tool(), r#"{"action":"import"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("invalid import request"));
    }

    #[test]
    fn test_run_success() {
        let tool = make_tool();
        let r = exec(
            &tool,
            r#"{"action":"run","goal":"Add tests","repo":"test-repo","base_ref":"main"}"#,
        );
        assert!(!r.is_error);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        assert!(v["run_id"].is_string());
        assert!(v["job_id"].is_string());
        assert_eq!(v["state"].as_str().unwrap(), "queued");
    }

    #[test]
    fn test_run_invalid_repo() {
        let r = exec(
            &make_tool(),
            r#"{"action":"run","goal":"x","repo":"bad","base_ref":"main"}"#,
        );
        assert!(r.is_error);
        assert!(r.content.contains("invalid_repo"));
    }

    #[test]
    fn test_run_invalid_base_ref() {
        let r = exec(
            &make_tool(),
            r#"{"action":"run","goal":"x","repo":"test-repo","base_ref":"nope"}"#,
        );
        assert!(r.is_error);
        assert!(r.content.contains("invalid_base_ref"));
    }

    #[test]
    fn test_run_with_priority() {
        let r = exec(
            &make_tool(),
            r#"{"action":"run","goal":"r","repo":"test-repo","base_ref":"main","priority":"high"}"#,
        );
        assert!(!r.is_error);
        assert!(r.content.contains("job_id"));
    }

    #[test]
    fn test_run_missing_goal() {
        let r = exec(
            &make_tool(),
            r#"{"action":"run","repo":"test-repo","base_ref":"main"}"#,
        );
        assert!(r.is_error);
        assert!(r.content.contains("invalid run request"));
    }

    #[test]
    fn test_status_by_job_id() {
        let tool = make_tool();
        let jid = create_job(&tool);
        let r = exec(&tool, &format!(r#"{{"action":"status","job_id":"{jid}"}}"#));
        assert!(!r.is_error);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["state"].as_str().unwrap(), "queued");
    }

    #[test]
    fn test_status_by_run_id() {
        let tool = make_tool();
        let r = exec(
            &tool,
            r#"{"action":"run","goal":"t","repo":"test-repo","base_ref":"main"}"#,
        );
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        let rid = v["run_id"].as_str().unwrap();
        let s = exec(&tool, &format!(r#"{{"action":"status","run_id":"{rid}"}}"#));
        assert!(!s.is_error);
    }

    #[test]
    fn test_status_not_found() {
        let r = exec(
            &make_tool(),
            r#"{"action":"status","job_id":"nonexistent"}"#,
        );
        assert!(r.is_error);
        assert!(r.content.contains("not_found"));
    }

    #[test]
    fn test_status_missing_ids() {
        let r = exec(&make_tool(), r#"{"action":"status"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("job_id or run_id"));
    }

    #[test]
    fn test_cancel_queued() {
        let tool = make_tool();
        let jid = create_job(&tool);
        let r = exec(&tool, &format!(r#"{{"action":"cancel","job_id":"{jid}"}}"#));
        assert!(!r.is_error);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["state"].as_str().unwrap(), "canceled");
    }

    #[test]
    fn test_cancel_not_found() {
        let r = exec(
            &make_tool(),
            r#"{"action":"cancel","job_id":"nonexistent"}"#,
        );
        assert!(r.is_error);
        assert!(r.content.contains("not_found"));
    }

    #[test]
    fn test_cleanup_terminal() {
        let tool = make_tool();
        let jid = create_job(&tool);
        exec(&tool, &format!(r#"{{"action":"cancel","job_id":"{jid}"}}"#));
        let r = exec(
            &tool,
            &format!(r#"{{"action":"cleanup","job_id":"{jid}"}}"#),
        );
        assert!(!r.is_error);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        assert!(v["cleaned"].as_bool().unwrap());
    }

    #[test]
    fn test_cleanup_running_rejected() {
        // The CodingJobService trait doesn't expose begin_preparation/mark_ready,
        // so we test cleanup rejection on a queued job (also non-terminal).
        let tool = make_tool();
        let jid = create_job(&tool);
        let r = exec(
            &tool,
            &format!(r#"{{"action":"cleanup","job_id":"{jid}"}}"#),
        );
        assert!(r.is_error);
        assert!(r.content.contains("job_not_terminal"));
    }

    #[test]
    fn test_list_all() {
        let tool = make_tool();
        create_job(&tool);
        create_job(&tool);
        let r = exec(&tool, r#"{"action":"list"}"#);
        assert!(!r.is_error);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["jobs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_list_with_filter() {
        let tool = make_tool();
        let jid = create_job(&tool);
        create_job(&tool);
        exec(&tool, &format!(r#"{{"action":"cancel","job_id":"{jid}"}}"#));
        let r = exec(&tool, r#"{"action":"list","state_filter":["queued"]}"#);
        assert!(!r.is_error);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["jobs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_list_empty() {
        let r = exec(&make_tool(), r#"{"action":"list"}"#);
        assert!(!r.is_error);
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        assert!(v["jobs"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_message_action_requires_subagent_mode() {
        let r = exec(
            &make_tool(),
            r#"{"action":"message","text":"investigate stuck jobs"}"#,
        );
        assert!(r.is_error);
        assert!(r.content.contains("subagent"));
    }

    #[test]
    fn test_unknown_action() {
        let r = exec(&make_tool(), r#"{"action":"explode"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("unknown action"));
    }

    #[test]
    fn test_invalid_json() {
        let r = exec(&make_tool(), "not json");
        assert!(r.is_error);
        assert!(r.content.contains("invalid JSON"));
    }

    #[test]
    fn test_missing_action() {
        let r = exec(&make_tool(), r#"{"goal":"t"}"#);
        assert!(r.is_error);
        assert!(r.content.contains("action"));
    }

    #[test]
    fn test_debug_format() {
        assert!(format!("{:?}", make_tool()).contains("CodingJobTool"));
    }
}
