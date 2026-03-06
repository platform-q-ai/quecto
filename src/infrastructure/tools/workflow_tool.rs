//! Workflow tool: lets the agent track BDD/TDD development workflow progress.
//!
//! Actions: status, check, uncheck, reset, skip, set_issue, clear_issue.
//! Every state mutation emits a `workflow_state` event via an optional callback.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolGuard, ToolResult};
use crate::domain::workflow::{GuardRule, WorkflowState, WorkflowStateSnapshot};

/// Callback for emitting workflow_state UDS events.
pub type WorkflowEventEmitter = Arc<dyn Fn(serde_json::Value) + Send + Sync>;

/// Tool that lets the agent manage the development workflow.
pub struct WorkflowTool {
    state: Arc<Mutex<WorkflowState>>,
    event_emitter: Option<WorkflowEventEmitter>,
    guards: Vec<GuardRule>,
}

impl std::fmt::Debug for WorkflowTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowTool").finish()
    }
}

impl WorkflowTool {
    /// Create a new workflow tool with the given state.
    pub fn new(state: Arc<Mutex<WorkflowState>>) -> Self {
        Self {
            state,
            event_emitter: None,
            guards: vec![],
        }
    }

    /// Create a new workflow tool with a UDS event emitter.
    pub fn with_event_emitter(
        state: Arc<Mutex<WorkflowState>>,
        emitter: WorkflowEventEmitter,
    ) -> Self {
        Self {
            state,
            event_emitter: Some(emitter),
            guards: vec![],
        }
    }

    /// Set the guard rules for the workflow tool.
    pub fn set_guards(&mut self, guards: Vec<GuardRule>) {
        self.guards = guards;
    }

    /// Set the event emitter.
    pub fn set_event_emitter(&mut self, emitter: WorkflowEventEmitter) {
        self.event_emitter = Some(emitter);
    }

    /// Get a reference to the shared workflow state.
    pub fn state(&self) -> &Arc<Mutex<WorkflowState>> {
        &self.state
    }

    /// Acquire the lock, recovering from poison. Returns a graceful error
    /// string instead of panicking if the mutex was poisoned.
    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, WorkflowState>, String> {
        self.state
            .lock()
            .map_err(|e| format!("workflow state poisoned: {}", e))
    }

    fn handle_action(&self, arguments: &str) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {}", e))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: action")?;

        // For status, no mutation — read-only, no event emission.
        if action == "status" {
            let state = self.lock_state()?;
            return Ok(state.system_prompt_snippet());
        }

        // check_commit is read-only — checks all guard rules.
        if action == "check_commit" {
            let state = self.lock_state()?;
            for rule in &self.guards {
                if let Err(e) = state.check_steps_complete(rule.before_step) {
                    return Err(format!(
                        "{} (blocked commands: {})",
                        e,
                        rule.commands.join(", ")
                    ));
                }
            }
            return Ok("All guard rules satisfied.".to_string());
        }

        // All other actions mutate state. Acquire lock once for both
        // mutation and event snapshot to eliminate TOCTOU gap.
        let mut state = self.lock_state()?;
        let result = match action {
            "check" => self.do_check(&mut state, &args),
            "uncheck" => self.do_uncheck(&mut state, &args),
            "reset" => self.do_reset(&mut state),
            "skip" => self.do_skip(&mut state, &args),
            "set_issue" => self.do_set_issue(&mut state, &args),
            "clear_issue" => self.do_clear_issue(&mut state),
            _ => Err(format!("unknown action: {}", action)),
        };

        // Snapshot under the same lock, then drop before emitting.
        let maybe_event = if result.is_ok() {
            Some(snapshot_to_event(&state.snapshot()))
        } else {
            None
        };
        drop(state);

        // Emit outside the lock so I/O in the emitter doesn't block state access.
        if let Some(event) = maybe_event {
            self.emit_event(event);
        }

        result
    }

    fn do_check(
        &self,
        state: &mut WorkflowState,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = self.parse_step(args)?;
        state.check(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} checked.", step))
    }

    fn do_uncheck(
        &self,
        state: &mut WorkflowState,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = self.parse_step(args)?;
        state.uncheck(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} unchecked.", step))
    }

    fn do_reset(&self, state: &mut WorkflowState) -> Result<String, String> {
        state.reset();
        Ok("Workflow reset. All steps cleared.".to_string())
    }

    fn do_skip(
        &self,
        state: &mut WorkflowState,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = self.parse_step(args)?;
        state.skip(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} skipped (force-marked done).", step))
    }

    fn do_set_issue(
        &self,
        state: &mut WorkflowState,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let raw_number = args
            .get("issueNumber")
            .and_then(|v| v.as_u64())
            .ok_or("missing field: issueNumber")?;
        if raw_number > u32::MAX as u64 {
            return Err("issueNumber exceeds u32 range".to_string());
        }
        let number = raw_number as u32;
        let title = args
            .get("issueTitle")
            .and_then(|v| v.as_str())
            .ok_or("missing field: issueTitle")?;
        state.set_issue(number, title.to_string());
        Ok(format!("Active issue set: #{} — {}", number, title))
    }

    fn do_clear_issue(&self, state: &mut WorkflowState) -> Result<String, String> {
        state.clear_issue();
        Ok("Active issue cleared.".to_string())
    }

    fn parse_step(&self, args: &serde_json::Value) -> Result<u32, String> {
        args.get("step")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .ok_or_else(|| "missing field: step".to_string())
    }

    fn emit_event(&self, event: serde_json::Value) {
        if let Some(ref emitter) = self.event_emitter {
            emitter(event);
        }
    }
}

/// Convert a domain snapshot to a `serde_json::Value` for UDS emission.
/// Lives in infrastructure (not domain) to keep `serde_json` out of the
/// domain layer.
pub fn snapshot_to_event(snapshot: &WorkflowStateSnapshot) -> serde_json::Value {
    let steps: Vec<serde_json::Value> = snapshot
        .steps
        .iter()
        .map(|(step, done)| {
            serde_json::json!({
                "id": step.id,
                "label": step.label,
                "phase": step.phase,
                "done": done,
            })
        })
        .collect();

    let mut event = serde_json::json!({
        "type": "workflow_state",
        "steps": steps,
        "progress": {
            "done": snapshot.progress.done,
            "total": snapshot.progress.total,
            "percent": snapshot.progress.percent,
        },
    });

    if let Some((num, title)) = &snapshot.active_issue {
        event["activeIssue"] = serde_json::json!({
            "number": num,
            "title": title,
        });
    }

    event
}

impl Tool for WorkflowTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "workflow".into(),
            description: "Manage the development workflow checklist. Track BDD/TDD Red-Green-Refactor progress.\n\nExample: {\"action\":\"status\"}\nExample: {\"action\":\"check\",\"step\":1}\nExample: {\"action\":\"set_issue\",\"issueNumber\":42,\"issueTitle\":\"Add feature X\"}"
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","check","uncheck","reset","skip","set_issue","clear_issue"],"description":"The workflow action to perform"},"step":{"type":"integer","description":"Step number (1-based, for check/uncheck/skip)"},"issueNumber":{"type":"integer","description":"GitHub issue number (for set_issue)"},"issueTitle":{"type":"string","description":"GitHub issue title (for set_issue)"}},"required":["action"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            match self.handle_action(&args) {
                Ok(content) => Ok(ToolResult {
                    content,
                    is_error: false,
                    image_blocks: vec![],
                }),
                Err(e) => Ok(ToolResult {
                    content: e,
                    is_error: true,
                    image_blocks: vec![],
                }),
            }
        })
    }
}

// ─── WorkflowGuard ───────────────────────────────────────────────────────────

/// Pre-parsed guard rule with patterns already split into (binary, subcmds).
#[derive(Debug)]
struct ParsedGuardRule {
    parsed_commands: Vec<(String, Vec<String>)>,
    before_step: u32,
    message: String,
}

/// Guard that blocks configured commands in bash calls when the
/// workflow hasn't reached the required step.
///
/// Non-bash tools always pass. Commands not matching any guard rule pass.
/// Empty `rules` means no blocking.
#[derive(Debug)]
pub struct WorkflowGuard {
    state: Arc<Mutex<WorkflowState>>,
    rules: Vec<ParsedGuardRule>,
}

impl WorkflowGuard {
    /// Create a new workflow guard with configurable rules.
    /// Patterns are pre-parsed at construction time to avoid per-call allocation.
    pub fn new(state: Arc<Mutex<WorkflowState>>, rules: Vec<GuardRule>) -> Self {
        let parsed = rules
            .into_iter()
            .map(|r| ParsedGuardRule {
                parsed_commands: parse_patterns(&r.commands),
                before_step: r.before_step,
                message: r.message,
            })
            .collect();
        Self {
            state,
            rules: parsed,
        }
    }
}

impl ToolGuard for WorkflowGuard {
    fn check(&self, tool_name: &str, arguments: &str) -> Result<(), String> {
        if tool_name != "bash" || self.rules.is_empty() {
            return Ok(());
        }

        let command = extract_bash_command(arguments);

        // Lock once for all rules (avoid TOCTOU between rules)
        let state = self
            .state
            .lock()
            .map_err(|e| format!("workflow state poisoned: {}", e))?;

        for rule in &self.rules {
            if command_matches_parsed(&command, &rule.parsed_commands) {
                state.check_steps_complete(rule.before_step).map_err(|_| {
                    format!(
                        "BLOCKED: {} Run workflow(action='status') to see current progress.",
                        rule.message
                    )
                })?;
            }
        }

        Ok(())
    }
}

use super::command_match::{command_matches_parsed, extract_bash_command, parse_patterns};

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tool() -> WorkflowTool {
        let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
        WorkflowTool::new(state)
    }

    fn test_tool_with_emitter() -> (WorkflowTool, Arc<Mutex<Vec<serde_json::Value>>>) {
        let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
        let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = events.clone();
        let emitter: WorkflowEventEmitter = Arc::new(move |event| {
            events_clone.lock().unwrap().push(event);
        });
        (WorkflowTool::with_event_emitter(state, emitter), events)
    }

    #[tokio::test]
    async fn test_status() {
        let tool = test_tool();
        let result = tool.execute(r#"{"action":"status"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("0/16"));
    }

    #[tokio::test]
    async fn test_check() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"check","step":1}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("checked"));
    }

    #[tokio::test]
    async fn test_uncheck() {
        let tool = test_tool();
        tool.execute(r#"{"action":"check","step":1}"#)
            .await
            .unwrap();
        let result = tool
            .execute(r#"{"action":"uncheck","step":1}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("unchecked"));
    }

    #[tokio::test]
    async fn test_reset() {
        let tool = test_tool();
        tool.execute(r#"{"action":"check","step":1}"#)
            .await
            .unwrap();
        let result = tool.execute(r#"{"action":"reset"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("reset"));
    }

    #[tokio::test]
    async fn test_skip() {
        let tool = test_tool();
        let result = tool.execute(r#"{"action":"skip","step":5}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("skipped"));
    }

    #[tokio::test]
    async fn test_set_issue() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"set_issue","issueNumber":42,"issueTitle":"My feature"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("#42"));
    }

    #[tokio::test]
    async fn test_clear_issue() {
        let tool = test_tool();
        tool.execute(r#"{"action":"set_issue","issueNumber":42,"issueTitle":"My feature"}"#)
            .await
            .unwrap();
        let result = tool.execute(r#"{"action":"clear_issue"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("cleared"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = test_tool();
        let result = tool.execute(r#"{"action":"unknown"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("unknown action"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        let tool = test_tool();
        let result = tool.execute(r#"{}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("action"));
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let tool = test_tool();
        let result = tool.execute("not json").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("invalid JSON"));
    }

    #[test]
    fn test_definition() {
        let tool = test_tool();
        let def = tool.definition();
        assert_eq!(def.name, "workflow");
        assert!(def.description.contains("workflow"));
        assert!(def.description.contains("Example"));
    }

    #[test]
    fn test_debug_format() {
        let tool = test_tool();
        let debug = format!("{:?}", tool);
        assert!(debug.contains("WorkflowTool"));
    }

    #[tokio::test]
    async fn test_check_ordering_enforced() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"check","step":3}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("complete step 1 first"));
    }

    #[tokio::test]
    async fn test_check_out_of_range() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"check","step":0}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("invalid step"));
    }

    #[tokio::test]
    async fn test_set_issue_missing_number() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"set_issue","issueTitle":"test"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("issueNumber"));
    }

    #[tokio::test]
    async fn test_set_issue_missing_title() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"set_issue","issueNumber":42}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("issueTitle"));
    }

    // ─── Event emission tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_event_emitted_on_check() {
        let (tool, events) = test_tool_with_emitter();
        tool.execute(r#"{"action":"check","step":1}"#)
            .await
            .unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "workflow_state");
        assert!(events[0]["steps"].is_array());
        assert!(events[0]["progress"].is_object());
    }

    #[tokio::test]
    async fn test_event_emitted_on_reset() {
        let (tool, events) = test_tool_with_emitter();
        tool.execute(r#"{"action":"reset"}"#).await.unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_event_emitted_on_set_issue() {
        let (tool, events) = test_tool_with_emitter();
        tool.execute(r#"{"action":"set_issue","issueNumber":42,"issueTitle":"My feature"}"#)
            .await
            .unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].get("activeIssue").is_some());
    }

    #[tokio::test]
    async fn test_no_event_on_status() {
        let (tool, events) = test_tool_with_emitter();
        tool.execute(r#"{"action":"status"}"#).await.unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 0);
    }

    #[tokio::test]
    async fn test_no_event_on_error() {
        let (tool, events) = test_tool_with_emitter();
        tool.execute(r#"{"action":"check","step":3}"#)
            .await
            .unwrap(); // ordering error
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 0);
    }

    // ─── snapshot_to_event tests ─────────────────────────────────────────────

    #[test]
    fn test_snapshot_to_event() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        state.set_issue(42, "My feature".into());
        let event = snapshot_to_event(&state.snapshot());
        assert_eq!(event["type"], "workflow_state");
        assert!(event["steps"].is_array());
        assert_eq!(event["steps"].as_array().unwrap().len(), 16);
        assert_eq!(event["steps"][0]["done"], true);
        assert_eq!(event["steps"][1]["done"], false);
        assert_eq!(event["progress"]["done"], 1);
        assert_eq!(event["activeIssue"]["number"], 42);
    }

    #[test]
    fn test_snapshot_to_event_no_issue() {
        let state = WorkflowState::default_bdd();
        let event = snapshot_to_event(&state.snapshot());
        assert!(event.get("activeIssue").is_none());
    }

    // ─── check_commit action tests ──────────────────────────────────────────

    fn test_tool_with_guards(guards: Vec<GuardRule>) -> WorkflowTool {
        let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
        let mut tool = WorkflowTool::new(state);
        tool.set_guards(guards);
        tool
    }

    #[tokio::test]
    async fn test_check_commit_blocked() {
        let tool = test_tool_with_guards(vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Complete steps first.".into(),
        }]);
        let result = tool.execute(r#"{"action":"check_commit"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("step 1"));
    }

    #[tokio::test]
    async fn test_check_commit_allowed_after_steps() {
        let tool = test_tool_with_guards(vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Complete steps first.".into(),
        }]);
        for i in 1..=6 {
            tool.execute(&format!(r#"{{"action":"check","step":{}}}"#, i))
                .await
                .unwrap();
        }
        let result = tool.execute(r#"{"action":"check_commit"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("satisfied"));
    }

    #[tokio::test]
    async fn test_check_commit_no_guards() {
        let tool = test_tool_with_guards(vec![]);
        let result = tool.execute(r#"{"action":"check_commit"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("satisfied"));
    }

    // Pattern matching tests are in command_match.rs
}
