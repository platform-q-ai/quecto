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

/// Build a [`WorkflowEventEmitter`] that serializes each event as a JSON line
/// and sends it through a `tokio::sync::broadcast` channel (#598).
///
/// Every emitted event is stamped with the emitting unit's identity —
/// `agent_id` (this agent's session name) and `parent_id` (the spawning
/// agent, or null at the root) — so any consumer can reconstruct the unit tree
/// from the event stream alone (PRD Stage B).
pub fn broadcast_emitter(
    tx: tokio::sync::broadcast::Sender<String>,
    agent_id: Option<String>,
    parent_id: Option<String>,
) -> WorkflowEventEmitter {
    Arc::new(move |mut event: serde_json::Value| {
        if let Some(obj) = event.as_object_mut() {
            obj.insert("agent_id".into(), serde_json::json!(agent_id));
            obj.insert("parent_id".into(), serde_json::json!(parent_id));
        }
        // serde_json::Value → String serialization is infallible.
        let mut line =
            serde_json::to_string(&event).expect("serde_json::Value serializes infallibly");
        line.push('\n');
        let _ = tx.send(line);
    })
}

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
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or("missing field: command")?;
                check_matching_guards_for_command(&engine, command)?;
                Ok("All workflow guards for command are satisfied.".into())
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
        Ok(format!(
            "Selected workflow template '{}'.{}",
            template,
            step_handoff(engine, "Current step")
        ))
    }

    fn do_check(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = parse_step(args)?;
        engine.check(step).map_err(|e| e.to_string())?;
        Ok(format!(
            "Step {} checked.{}",
            step,
            step_handoff(engine, "Next step")
        ))
    }

    fn do_uncheck(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = parse_step(args)?;
        engine.uncheck(step).map_err(|e| e.to_string())?;
        // Unchecking can move the current step BACKWARDS — re-orient the
        // model on where the workflow now stands (#1113 AC2).
        Ok(format!(
            "Step {} unchecked.{}",
            step,
            step_handoff(engine, "Current step")
        ))
    }

    fn do_skip(
        &self,
        engine: &mut WorkflowEngine,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let step = parse_step(args)?;
        engine.skip(step).map_err(|e| e.to_string())?;
        // Skipping advances the current step exactly like `check` — hand the
        // model the next step's label and guidance (#1113 AC2).
        Ok(format!(
            "Step {} skipped.{}",
            step,
            step_handoff(engine, "Next step")
        ))
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

/// Current-step handoff appended to every step-state-changing tool result
/// (`select_template`/`check`/`skip`/`uncheck`, #1113 AC2): with the system
/// prompt static for the whole session, the tool result is where the model
/// receives each step's guidance — plus workflow progress and the active
/// issue — exactly when the current step changes. Rendering is owned by the
/// engine's [`WorkflowEngine::step_handoff_text`], the same wording source
/// behind the idle-boundary nudges, so the channels cannot drift apart.
fn step_handoff(engine: &WorkflowEngine, heading: &str) -> String {
    engine.step_handoff_text(heading)
}

fn render_templates(templates: Vec<WorkflowTemplateSummary>) -> String {
    let mut out = String::from("Available workflow templates:\n");
    for t in templates {
        out.push_str(&format!("- {} — {}: {}\n", t.id, t.label, t.description));
        if let Some(when_to_use) = t.when_to_use {
            out.push_str(&format!("  When to use: {}\n", when_to_use));
        }
    }
    out
}

fn truncate_at_100(s: &str) -> String {
    s.chars().take(100).collect()
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
        let display = truncate_at_100(s);
        return s
            .parse::<u32>()
            .map_err(|_| format!("invalid step value: {}", display));
    }
    Err(format!(
        "invalid step value: {}",
        truncate_at_100(&val.to_string())
    ))
}

fn parse_issue(args: &serde_json::Value) -> Result<(u32, String), String> {
    parse_issue_fields(args)?.ok_or("missing field: issueNumber".into())
}

fn parse_optional_issue(args: &serde_json::Value) -> Result<Option<(u32, String)>, String> {
    parse_issue_fields(args)
}

fn parse_issue_fields(args: &serde_json::Value) -> Result<Option<(u32, String)>, String> {
    let Some(val) = args.get("issueNumber") else {
        if args.get("issueTitle").is_some() {
            return Err("issueTitle requires issueNumber".into());
        }
        return Ok(None);
    };
    let number = if let Some(n) = val.as_u64() {
        if n > u32::MAX as u64 {
            return Err("issueNumber exceeds u32 range".into());
        }
        n as u32
    } else if let Some(s) = val.as_str() {
        let display = truncate_at_100(s);
        s.parse::<u32>()
            .map_err(|_| format!("invalid issueNumber: {}", display))?
    } else {
        return Err(format!(
            "invalid issueNumber: {}",
            truncate_at_100(&val.to_string())
        ));
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
            description: "Manage the active development workflow. Discover templates with action=list_templates, activate one with action=select_template, mark steps done with action=check (the result hands you the next step's guidance), and use action=status for current progress and guidance.".into(),
            parameters_schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","list_templates","select_template","check","uncheck","skip","reset","set_issue","clear_issue","check_guards"]},"template":{"type":"string"},"step":{"type":"integer"},"issueNumber":{"type":"integer"},"issueTitle":{"type":"string"},"command":{"type":"string"}},"required":["action"]}"#.into(),
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
    /// Cache of guard rules with their command patterns parsed once, keyed by
    /// the active template id. Honours the parse-once contract documented in
    /// `command_match.rs` — without it, every bash call re-cloned the template
    /// guards and re-parsed every pattern (#996 item 3).
    rule_cache: std::sync::Mutex<Option<(String, std::sync::Arc<Vec<ParsedGuardRule>>)>>,
}

#[derive(Debug)]
struct ParsedGuardRule {
    parsed_commands: Vec<(String, Vec<String>)>,
    before_step_key: String,
    message: String,
}

impl WorkflowGuard {
    pub fn new(engine: WorkflowEngineHandle) -> Self {
        Self {
            engine,
            rule_cache: std::sync::Mutex::new(None),
        }
    }

    /// Return the parsed guard rules for `template`, building (and caching) them
    /// the first time each template id is seen. Template definitions are
    /// immutable once loaded, so the id fully identifies the parsed rule set.
    fn parsed_rules_for(
        &self,
        template: &crate::domain::workflow::WorkflowTemplate,
    ) -> std::sync::Arc<Vec<ParsedGuardRule>> {
        let mut cache = self.rule_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((id, rules)) = cache.as_ref() {
            if id == &template.id {
                return rules.clone();
            }
        }
        let rules: std::sync::Arc<Vec<ParsedGuardRule>> = std::sync::Arc::new(
            template
                .guards
                .iter()
                .cloned()
                .map(|r: WorkflowGuardRule| ParsedGuardRule {
                    parsed_commands: parse_patterns(&r.commands),
                    before_step_key: r.before_step_key,
                    message: r.message,
                })
                .collect(),
        );
        *cache = Some((template.id.clone(), rules.clone()));
        rules
    }
}

fn check_matching_guards_for_command(engine: &WorkflowEngine, command: &str) -> Result<(), String> {
    let template = engine
        .active_template()
        .ok_or_else(|| "select_template before checking workflow guards".to_string())?;
    let parsed_rules: Vec<_> = template
        .guards
        .iter()
        .map(|rule| (rule, parse_patterns(&rule.commands)))
        .collect();
    engine
        .check_matching_guards(|guard| {
            parsed_rules.iter().any(|(rule, parsed_commands)| {
                std::ptr::eq(*rule, guard) && command_matches_parsed(command, parsed_commands)
            })
        })
        .map_err(|e| e.to_string())
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
        let rules = self.parsed_rules_for(template);
        if rules.is_empty() {
            return Ok(());
        }
        let done = engine.all_step_statuses();
        for rule in rules.iter() {
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
#[path = "workflow_tool_comprehensive_tests.rs"]
mod comprehensive_tests;

#[cfg(test)]
#[path = "workflow_tool_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_tool_cov_tests.rs"]
mod cov_tests;
