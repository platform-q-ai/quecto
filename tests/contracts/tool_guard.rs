//! Contract tests for the `ToolGuard` port.
//!
//! The port contract: `check(tool_name, arguments) -> Result<(), String>`.
//! - `Ok(())` allows the tool call through.
//! - `Err(msg)` blocks it; the msg is surfaced to the LLM as `is_error: true`.
//! - Guards are pure w.r.t. their internal state: no side effects on tool
//!   execution (they only observe).

use quecto::domain::tool::ToolGuard;
use quecto::domain::workflow::{WorkflowConfig, WorkflowEngine, WorkflowTemplate};
use quecto::infrastructure::tools::workflow_tool::WorkflowGuard;
use std::sync::{Arc, Mutex};

fn workflow_guard(templates: Vec<WorkflowTemplate>) -> Arc<dyn ToolGuard> {
    let config = WorkflowConfig {
        auto_continue: true,
        completion_nudge: true,
        selector_prompt: None,
        templates,
    };
    let engine = WorkflowEngine::new(config, true).unwrap();
    Arc::new(WorkflowGuard::new(Arc::new(Mutex::new(engine))))
}

#[test]
fn non_bash_tools_are_always_allowed() {
    // Contract: WorkflowGuard only inspects `bash`. Every other tool name
    // must pass through unchanged.
    let guard = workflow_guard(vec![]);
    assert!(guard.check("read", r#"{"path":"x"}"#).is_ok());
    assert!(guard.check("write", r#"{"path":"x","content":""}"#).is_ok());
    assert!(guard.check("grep", r#"{"pattern":"x"}"#).is_ok());
}

#[test]
fn bash_is_blocked_when_no_template_is_active() {
    // Contract with no templates: the guard short-circuits with an error
    // directing the caller to select a template first.
    let guard = workflow_guard(vec![]);
    let r = guard.check("bash", r#"{"command":"ls"}"#);
    assert!(r.is_err(), "bash must be blocked when no template is active");
    let msg = r.unwrap_err();
    assert!(msg.contains("template") || msg.contains("BLOCKED"),
        "blocking message should mention template selection, got: {msg}");
}

#[test]
fn check_is_pure_and_idempotent() {
    // Calling check() repeatedly with the same args must produce the same
    // verdict — the port must not carry per-call state.
    let guard = workflow_guard(vec![]);
    let a = guard.check("bash", r#"{"command":"ls"}"#);
    let b = guard.check("bash", r#"{"command":"ls"}"#);
    assert_eq!(a.is_err(), b.is_err());
    assert_eq!(a.err().unwrap_or_default(), b.err().unwrap_or_default());
}
