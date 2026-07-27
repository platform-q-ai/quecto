//! Typed protocol values for TUI workflow wire payloads.
//!
//! Follows the mapper convention in [`crate::protocol::model_payloads`].
//! Feature/view code converts these DTOs into component state; it must not
//! re-parse the JSON fields.

/// One step from a `workflow_state` / `get_state.workflow` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStepSnapshot {
    pub id: u32,
    pub label: String,
    pub phase: String,
    pub done: bool,
}

/// Typed workflow snapshot (progress, issue, template, automation flags).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkflowSnapshot {
    pub steps: Vec<WorkflowStepSnapshot>,
    pub done: u32,
    pub total: u32,
    pub issue_number: Option<u32>,
    pub issue_title: Option<String>,
    pub mode: Option<String>,
    pub template_name: Option<String>,
    pub template_count: u32,
    pub workflow_auto_continue: bool,
    pub workflow_completion_nudge: bool,
}

/// Automation flags from a workflow snapshot or `set_workflow_automation` ack.
///
/// Each field is `None` when the key is absent so callers can leave their
/// existing value unchanged (App-global automation sync parity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkflowAutomationFlags {
    pub auto_continue: Option<bool>,
    pub completion_nudge: Option<bool>,
}

/// Parse a `workflow_state` JSON event / `get_state.workflow` object.
///
/// Parity quirks preserved here:
/// - step id: V2 `index` wins over V1 `id`
/// - issue / template keys: camelCase or snake_case
/// - activeIssue may be `{number,title}` or a two-element array
/// - automation: `autoContinue`/`auto_continue`, `completionNudge`/`completion_nudge`
/// - missing automation block → both flags false
pub fn parse_workflow_snapshot(data: &serde_json::Value) -> WorkflowSnapshot {
    let steps: Vec<WorkflowStepSnapshot> = data
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    let id = s.get("index").or_else(|| s.get("id"))?.as_u64()? as u32;
                    Some(WorkflowStepSnapshot {
                        id,
                        label: s
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        phase: s.get("phase")?.as_str()?.to_string(),
                        done: s.get("done")?.as_bool()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let done = data
        .get("progress")
        .and_then(|p| p.get("done"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or_else(|| steps.iter().filter(|s| s.done).count() as u32);
    let total = data
        .get("progress")
        .and_then(|p| p.get("total"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(steps.len() as u32);

    let issue_number = data
        .get("activeIssue")
        .or_else(|| data.get("active_issue"))
        .and_then(|i| {
            i.get("number")
                .or_else(|| i.as_array().and_then(|a| a.first()))
        })
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let issue_title = data
        .get("activeIssue")
        .or_else(|| data.get("active_issue"))
        .and_then(|i| {
            i.get("title")
                .or_else(|| i.as_array().and_then(|a| a.get(1)))
        })
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mode = data
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let template_name = data
        .get("activeTemplate")
        .or_else(|| data.get("active_template"))
        .and_then(|t| t.get("label"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let template_count = data
        .get("availableTemplates")
        .or_else(|| data.get("available_templates"))
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);

    // Snapshot path: only the nested `automation` block counts (missing → false).
    // Top-level fallback is reserved for set_workflow_automation acks via
    // `parse_workflow_automation`.
    let nested = data
        .get("automation")
        .map(parse_automation_object)
        .unwrap_or_default();
    WorkflowSnapshot {
        steps,
        done,
        total,
        issue_number,
        issue_title,
        mode,
        template_name,
        template_count,
        workflow_auto_continue: nested.auto_continue.unwrap_or(false),
        workflow_completion_nudge: nested.completion_nudge.unwrap_or(false),
    }
}

/// Parse automation flags from a workflow object or a `set_workflow_automation`
/// response. Looks under `automation` when present, otherwise at the top level
/// (App `sync_workflow_automation` parity for ack payloads).
pub fn parse_workflow_automation(data: &serde_json::Value) -> WorkflowAutomationFlags {
    parse_automation_object(data.get("automation").unwrap_or(data))
}

fn parse_automation_object(automation: &serde_json::Value) -> WorkflowAutomationFlags {
    WorkflowAutomationFlags {
        auto_continue: automation
            .get("autoContinue")
            .or_else(|| automation.get("auto_continue"))
            .and_then(|v| v.as_bool()),
        completion_nudge: automation
            .get("completionNudge")
            .or_else(|| automation.get("completion_nudge"))
            .and_then(|v| v.as_bool()),
    }
}

#[cfg(test)]
#[path = "workflow_payloads_tests.rs"]
mod tests;
