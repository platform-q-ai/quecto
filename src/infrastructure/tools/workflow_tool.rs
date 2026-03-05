//! Workflow tool: lets the agent track BDD/TDD development workflow progress.
//!
//! Actions: status, check, uncheck, reset, skip, set_issue, clear_issue.
//! Every state mutation emits a `workflow_state` event via an optional callback.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::domain::workflow::{WorkflowState, workflow_state_event};

/// Callback for emitting workflow_state UDS events.
pub type WorkflowEventEmitter = Arc<dyn Fn(serde_json::Value) + Send + Sync>;

/// Tool that lets the agent manage the development workflow.
pub struct WorkflowTool {
    state: Arc<Mutex<WorkflowState>>,
    event_emitter: Option<WorkflowEventEmitter>,
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
        }
    }

    /// Get a reference to the shared workflow state.
    pub fn state(&self) -> &Arc<Mutex<WorkflowState>> {
        &self.state
    }

    fn handle_action(&self, arguments: &str) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {}", e))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: action")?;

        let result = match action {
            "status" => self.handle_status(),
            "check" => self.handle_check(&args),
            "uncheck" => self.handle_uncheck(&args),
            "reset" => self.handle_reset(),
            "skip" => self.handle_skip(&args),
            "set_issue" => self.handle_set_issue(&args),
            "clear_issue" => self.handle_clear_issue(),
            _ => Err(format!("unknown action: {}", action)),
        };

        // Emit event on successful mutation
        if result.is_ok() && action != "status" {
            self.emit_event();
        }

        result
    }

    fn handle_status(&self) -> Result<String, String> {
        let state = self.state.lock().unwrap();
        Ok(state.system_prompt_snippet())
    }

    fn handle_check(&self, args: &serde_json::Value) -> Result<String, String> {
        let step = self.parse_step(args)?;
        let mut state = self.state.lock().unwrap();
        state.check(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} checked.", step))
    }

    fn handle_uncheck(&self, args: &serde_json::Value) -> Result<String, String> {
        let step = self.parse_step(args)?;
        let mut state = self.state.lock().unwrap();
        state.uncheck(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} unchecked.", step))
    }

    fn handle_reset(&self) -> Result<String, String> {
        let mut state = self.state.lock().unwrap();
        state.reset();
        Ok("Workflow reset. All steps cleared.".to_string())
    }

    fn handle_skip(&self, args: &serde_json::Value) -> Result<String, String> {
        let step = self.parse_step(args)?;
        let mut state = self.state.lock().unwrap();
        state.skip(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} skipped (force-marked done).", step))
    }

    fn handle_set_issue(&self, args: &serde_json::Value) -> Result<String, String> {
        let number = args
            .get("issueNumber")
            .and_then(|v| v.as_u64())
            .ok_or("missing field: issueNumber")? as u32;
        let title = args
            .get("issueTitle")
            .and_then(|v| v.as_str())
            .ok_or("missing field: issueTitle")?;
        let mut state = self.state.lock().unwrap();
        state.set_issue(number, title.to_string());
        Ok(format!("Active issue set: #{} — {}", number, title))
    }

    fn handle_clear_issue(&self) -> Result<String, String> {
        let mut state = self.state.lock().unwrap();
        state.clear_issue();
        Ok("Active issue cleared.".to_string())
    }

    fn parse_step(&self, args: &serde_json::Value) -> Result<u32, String> {
        args.get("step")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .ok_or_else(|| "missing field: step".to_string())
    }

    fn emit_event(&self) {
        if let Some(ref emitter) = self.event_emitter {
            let state = self.state.lock().unwrap();
            let event = workflow_state_event(&state);
            emitter(event);
        }
    }
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
}
