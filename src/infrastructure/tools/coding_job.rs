// Coding job tool: bridge between the agent loop and the CodingJobService.
//
// Accepts JSON with an "action" field and dispatches to the appropriate
// service method. Returns JSON-serialized responses.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::coding_command::{CommandError, ListRequest, RunRequest};
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
            "run" => self.handle_run(arguments),
            "status" => self.handle_status(&args),
            "cancel" => self.handle_cancel(&args),
            "cleanup" => self.handle_cleanup(&args),
            "list" => self.handle_list(arguments),
            other => Err(format!("unknown action: {other}")),
        }
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
            description: "Manage coding jobs (run, status, cancel, cleanup, list). \
                Jobs execute asynchronously in sandboxed workers."
                .to_string(),
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

const CODING_JOB_SCHEMA: &str = r#"{"type":"object","properties":{"action":{"type":"string","enum":["run","status","cancel","cleanup","list"],"description":"The coding job action to perform"},"goal":{"type":"string","description":"Goal description (for run)"},"repo":{"type":"string","description":"Repository identifier (for run)"},"base_ref":{"type":"string","description":"Base branch/ref (for run)"},"priority":{"type":"string","enum":["low","medium","high"],"description":"Job priority (for run, default: medium)"},"labels":{"type":"array","items":{"type":"string"},"description":"Labels (for run)"},"skills":{"type":"array","items":{"type":"string"},"description":"Skill names (for run)"},"profile":{"type":"string","description":"Profile name (for run, default: default)"},"max_wall_seconds":{"type":"integer","description":"Wall-clock timeout in seconds (for run)"},"job_id":{"type":"string","description":"Job ID (for status/cancel/cleanup)"},"run_id":{"type":"string","description":"Run ID (for status)"},"state_filter":{"type":"array","items":{"type":"string"},"description":"Filter by job states (for list)"},"keep_artifacts":{"type":"boolean","description":"Keep artifacts on cleanup (default: true)"}},"required":["action"]}"#;

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
        assert!(make_tool().definition().description.contains("coding job"));
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
