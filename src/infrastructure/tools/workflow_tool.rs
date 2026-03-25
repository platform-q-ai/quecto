//! Workflow V2 tool and guard.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolGuard, ToolResult};
use crate::domain::workflow::{
    WorkflowEngine, WorkflowGuardRule, WorkflowSnapshot, WorkflowTemplateSummary,
};

pub type WorkflowEventEmitter = Arc<dyn Fn(serde_json::Value) + Send + Sync>;
pub type WorkflowEngineHandle = Arc<Mutex<WorkflowEngine>>;

pub struct WorkflowTool {
    engine: WorkflowEngineHandle,
    event_emitter: Option<WorkflowEventEmitter>,
}

impl std::fmt::Debug for WorkflowTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowTool").finish()
    }
}

impl WorkflowTool {
    pub fn new(engine: WorkflowEngineHandle) -> Self {
        Self {
            engine,
            event_emitter: None,
        }
    }

    pub fn with_event_emitter(engine: WorkflowEngineHandle, emitter: WorkflowEventEmitter) -> Self {
        Self {
            engine,
            event_emitter: Some(emitter),
        }
    }

    pub fn set_event_emitter(&mut self, emitter: WorkflowEventEmitter) {
        self.event_emitter = Some(emitter);
    }

    pub fn engine(&self) -> &WorkflowEngineHandle {
        &self.engine
    }

    fn lock_engine(&self) -> Result<std::sync::MutexGuard<'_, WorkflowEngine>, String> {
        self.engine
            .lock()
            .map_err(|e| format!("workflow engine poisoned: {}", e))
    }

    fn handle_action(&self, arguments: &str) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {}", e))?;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: action")?;

        let mut engine = self.lock_engine()?;
        let result = match action {
            "status" => Ok(engine.status_text()),
            "list_templates" => Ok(render_templates(engine.list_templates())),
            "select_template" => self.do_select_template(&mut engine, &args),
            "check" => self.do_check(&mut engine, &args),
            "uncheck" => self.do_uncheck(&mut engine, &args),
            "skip" => self.do_skip(&mut engine, &args),
            "reset" => {
                engine.reset();
                Ok("Workflow reset to template selection mode.".into())
            }
            "set_issue" => self.do_set_issue(&mut engine, &args),
            "clear_issue" => {
                engine.clear_issue();
                Ok("Active issue cleared.".into())
            }
            "check_guards" => {
                engine.check_guards().map_err(|e| e.to_string())?;
                Ok("All workflow guards satisfied.".into())
            }
            _ => Err(format!("unknown action: {}", action)),
        };

        let event = if result.is_ok()
            && action != "status"
            && action != "list_templates"
            && action != "check_guards"
        {
            Some(snapshot_to_event(&engine.snapshot(true)))
        } else {
            None
        };
        drop(engine);
        if let Some(event) = event {
            self.emit_event(event);
        }
        result
    }

    fn do_select_template(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let template = args
            .get("template")
            .and_then(|v| v.as_str())
            .ok_or("missing field: template")?;
        let issue = parse_optional_issue(args)?;
        engine
            .select_template(template, issue)
            .map_err(|e| e.to_string())?;
        Ok(format!("Selected workflow template '{}'.", template))
    }

    fn do_check(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = parse_step(args)?;
        engine.check(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} checked.", step))
    }

    fn do_uncheck(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = parse_step(args)?;
        engine.uncheck(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} unchecked.", step))
    }

    fn do_skip(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = parse_step(args)?;
        engine.skip(step).map_err(|e| e.to_string())?;
        Ok(format!("Step {} skipped.", step))
    }

    fn do_set_issue(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let (number, title) = parse_issue(args)?;
        engine.set_issue(number, title.clone());
        Ok(format!("Active issue set: #{} — {}", number, title))
    }

    fn emit_event(&self, event: serde_json::Value) {
        if let Some(ref emitter) = self.event_emitter {
            emitter(event);
        }
    }
}

fn render_templates(templates: Vec<WorkflowTemplateSummary>) -> String {
    let mut out = String::from("Available workflow templates:\n");
    for t in templates {
        out.push_str(&format!("- {} — {}: {}\n", t.id, t.label, t.description));
    }
    out
}

fn parse_step(args: &serde_json::Value) -> Result<u32, String> {
    let val = args.get("step").ok_or("missing field: step")?;
    if let Some(n) = val.as_u64() {
        if n > u32::MAX as u64 {
            return Err(format!("step value {} exceeds valid range", n));
        }
        return Ok(n as u32);
    }
    if let Some(s) = val.as_str() {
        let display = if s.len() > 100 { &s[..100] } else { s };
        return s
            .parse::<u32>()
            .map_err(|_| format!("invalid step value: {}", display));
    }
    Err(format!("invalid step value: {}", val))
}

fn parse_issue(args: &serde_json::Value) -> Result<(u32, String), String> {
    let issue = parse_optional_issue(args)?.ok_or("missing field: issueNumber")?;
    Ok(issue)
}

fn parse_optional_issue(args: &serde_json::Value) -> Result<Option<(u32, String)>, String> {
    let Some(val) = args.get("issueNumber") else {
        return Ok(None);
    };
    let number = if let Some(n) = val.as_u64() {
        if n > u32::MAX as u64 {
            return Err("issueNumber exceeds u32 range".into());
        }
        n as u32
    } else if let Some(s) = val.as_str() {
        let display = if s.len() > 100 { &s[..100] } else { s };
        s.parse::<u32>()
            .map_err(|_| format!("invalid issueNumber: {}", display))?
    } else {
        return Err(format!("invalid issueNumber: {}", val));
    };
    let title = args
        .get("issueTitle")
        .and_then(|v| v.as_str())
        .ok_or("missing field: issueTitle")?
        .to_string();
    Ok(Some((number, title)))
}

pub fn snapshot_to_event(snapshot: &WorkflowSnapshot) -> serde_json::Value {
    let mut event = serde_json::json!({
        "type": "workflow_state",
        "enabled": snapshot.enabled,
        "guardsEnabled": snapshot.guards_enabled,
        "mode": snapshot.mode,
        "progress": snapshot.progress,
        "availableTemplates": snapshot.available_templates,
        "steps": snapshot.steps,
    });
    if let Some(template) = &snapshot.active_template {
        event["activeTemplate"] = serde_json::to_value(template).unwrap();
    }
    if let Some(issue) = &snapshot.active_issue {
        event["activeIssue"] = serde_json::json!({ "number": issue.0, "title": issue.1 });
    }
    if let Some(step) = &snapshot.current_step {
        event["currentStep"] = serde_json::to_value(step).unwrap();
    }
    event
}

impl Tool for WorkflowTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "workflow".into(),
            description: "Manage the active UDS workflow template and step progression.".into(),
            parameters_schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","list_templates","select_template","check","uncheck","skip","reset","set_issue","clear_issue","check_guards"]},"template":{"type":"string"},"step":{"type":"integer"},"issueNumber":{"type":"integer"},"issueTitle":{"type":"string"}},"required":["action"]}"#.into(),
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
                Err(content) => Ok(ToolResult {
                    content,
                    is_error: true,
                    image_blocks: vec![],
                }),
            }
        })
    }
}

#[derive(Debug)]
pub struct WorkflowGuard {
    engine: WorkflowEngineHandle,
}

#[derive(Debug)]
struct ParsedGuardRule {
    parsed_commands: Vec<(String, Vec<String>)>,
    before_step_key: String,
    message: String,
}

impl WorkflowGuard {
    pub fn new(engine: WorkflowEngineHandle) -> Self {
        Self { engine }
    }
}

impl ToolGuard for WorkflowGuard {
    fn check(&self, tool_name: &str, arguments: &str) -> Result<(), String> {
        if tool_name != "bash" {
            return Ok(());
        }
        let command = extract_bash_command(arguments);
        let engine = self
            .engine
            .lock()
            .map_err(|e| format!("workflow engine poisoned: {}", e))?;
        let template = match engine.active_template() {
            Some(t) => t,
            None => {
                return Err(
                    "BLOCKED: select a workflow template before running guarded commands.".into(),
                );
            }
        };
        let rules: Vec<ParsedGuardRule> = template
            .guards
            .iter()
            .cloned()
            .map(|r: WorkflowGuardRule| ParsedGuardRule {
                parsed_commands: parse_patterns(&r.commands),
                before_step_key: r.before_step_key,
                message: r.message,
            })
            .collect();
        if rules.is_empty() {
            return Ok(());
        }
        let done = engine.snapshot(true).steps;
        for rule in &rules {
            if !command_matches_parsed(&command, &rule.parsed_commands) {
                continue;
            }
            let idx = template
                .steps
                .iter()
                .position(|s| s.key == rule.before_step_key)
                .ok_or_else(|| {
                    format!(
                        "invalid guard configuration: unknown step key '{}'",
                        rule.before_step_key
                    )
                })?;
            for prior_idx in 0..idx {
                if !done.get(prior_idx).map(|s| s.done).unwrap_or(false) {
                    return Err(format!(
                        "BLOCKED: {} Run workflow(action='status') to see current progress.",
                        rule.message
                    ));
                }
            }
        }
        Ok(())
    }
}

use super::command_match::{command_matches_parsed, extract_bash_command, parse_patterns};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{WorkflowConfig, WorkflowMode};

    fn test_tool() -> WorkflowTool {
        let engine = Arc::new(Mutex::new(
            WorkflowEngine::new(WorkflowConfig::default(), true).unwrap(),
        ));
        WorkflowTool::new(engine)
    }

    #[tokio::test]
    async fn status_starts_in_selector_mode() {
        let tool = test_tool();
        let result = tool.execute(r#"{"action":"status"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Template Selection"));
    }

    #[tokio::test]
    async fn list_templates_works() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"list_templates"}"#)
            .await
            .unwrap();
        assert!(result.content.contains("feature"));
    }

    #[tokio::test]
    async fn select_template_and_check_flow() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"select_template","template":"fix"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        let result = tool
            .execute(r#"{"action":"check","step":1}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn no_template_selected_errors_for_check() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"action":"check","step":1}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("select_template"));
    }

    #[test]
    fn snapshot_event_contains_mode() {
        let engine = WorkflowEngine::new(WorkflowConfig::default(), true).unwrap();
        let event = snapshot_to_event(&engine.snapshot(true));
        assert_eq!(event["type"], "workflow_state");
        assert_eq!(
            event["mode"],
            serde_json::json!(WorkflowMode::SelectingTemplate)
        );
    }
}
