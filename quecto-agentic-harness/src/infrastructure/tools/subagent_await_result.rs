// Await-result types for agent_cmd await (#612), split out of subagent_registry
// to respect the 750-line file cap. No behavior change.

use crate::domain::workflow::{VerdictStatus, WorkflowMode};

/// Result of an `agent_cmd await` call.
///
/// `status`/`reason` describe the await lifecycle (idle/exited/timeout/error);
/// `result` is the typed verdict a parent branches on without parsing prose
/// (PRD Stage A R-A3).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AwaitResult {
    pub status: String,
    pub reason: Option<String>,
    pub agent_id: String,
    pub elapsed_ms: u64,
    pub workflow: Option<WorkflowSnapshot>,
    pub result: WorkflowResult,
    /// Actual run-level error cause (for example a provider/model failure),
    /// surfaced so a parent can triage without reading logs (#752). Only
    /// present when the await terminated because the child's run failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AwaitResult {
    /// Build an `AwaitResult`, deriving the typed [`WorkflowResult`] verdict from
    /// the lifecycle status, reason, and workflow snapshot.
    pub fn new(
        status: &str,
        reason: Option<&str>,
        agent_id: String,
        elapsed_ms: u64,
        workflow: Option<WorkflowSnapshot>,
    ) -> Self {
        Self::with_error(status, reason, agent_id, elapsed_ms, workflow, None)
    }

    /// Like [`AwaitResult::new`], but carries the actual run-level error cause
    /// (#752). The cause is redacted (see [`redact_secrets`]) before it crosses
    /// the trust boundary into the parent context, then threaded into
    /// [`WorkflowResult::derive`] so the verdict and summary stay derived in one
    /// place; the same redacted value populates the structured `error` field.
    pub fn with_error(
        status: &str,
        reason: Option<&str>,
        agent_id: String,
        elapsed_ms: u64,
        workflow: Option<WorkflowSnapshot>,
        error: Option<&str>,
    ) -> Self {
        // Redact once: provider/HTTP error strings can embed secrets (bearer
        // tokens, api keys, auth headers) and these responses reach the parent
        // model context and logs (#752, security review).
        let redacted = error.map(redact_secrets);
        let result = WorkflowResult::derive(status, reason, workflow.as_ref(), redacted.as_deref());
        Self {
            status: status.to_string(),
            reason: reason.map(str::to_string),
            agent_id,
            elapsed_ms,
            workflow,
            result,
            error: redacted,
        }
    }
}

/// Strip known secret patterns and bound the length of an error cause before it
/// is surfaced to the parent agent (#752). This is defense-in-depth: provider
/// error strings are not guaranteed to be sanitized upstream and can echo
/// bearer tokens, API keys, or auth query params.
///
/// Thin wrapper over the shared [`crate::domain::redaction`] redactor (single
/// source of truth, also used by the audit log, #790) that adds the length
/// bound specific to parent-context surfacing.
fn redact_secrets(cause: &str) -> String {
    const MAX_LEN: usize = 2000;
    crate::domain::redaction::redact_and_bound(cause, MAX_LEN)
}

/// Snapshot of workflow state at the moment `await` returns.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkflowSnapshot {
    pub mode: String,
    pub steps_completed: u32,
    pub steps_total: u32,
}

/// Typed verdict for an awaited subagent — the structured outcome a parent can
/// branch on (PRD Stage A R-A3).
///
/// NOTE: the verdict reflects what the parent *observed* and is derived from the
/// child-reported workflow snapshot; it is NOT an integrity boundary. A
/// compromised child (which already sits inside the parent's trust boundary)
/// could influence its own reported status.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkflowResult {
    pub status: VerdictStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_progress: Option<ResultProgress>,
}

/// Step progress carried in a [`WorkflowResult`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ResultProgress {
    pub done: u32,
    pub total: u32,
}

impl WorkflowResult {
    /// Derive the verdict from the await lifecycle status, reason, and workflow.
    /// - `Completed` — idle AND workflow `complete` (only when positively observed).
    /// - `Failed` — an await/agent error, or a non-clean process exit.
    /// - `Incomplete` — went idle without completing, or exited cleanly before it
    ///   (completion is observed at idle, never inferred from exit).
    /// - `Running` — await timed out while the child is still running (#925): a
    ///   check-in, not a failure/"gave up"; summary names progress + next action.
    pub fn derive(
        status: &str,
        reason: Option<&str>,
        workflow: Option<&WorkflowSnapshot>,
        error: Option<&str>,
    ) -> Self {
        let workflow_progress = workflow.map(|w| ResultProgress {
            done: w.steps_completed,
            total: w.steps_total,
        });
        let complete = workflow.is_some_and(|w| w.mode == WorkflowMode::Complete.wire_str());
        // A workflow only "exists to complete" once a template with steps is
        // bound. No snapshot at all, or a snapshot still selecting a template
        // with zero steps, means there was nothing to complete — going idle is
        // a clean finish, not an abandoned workflow (#…: avoids false "stalled"
        // reads for plain-task agents that were never given a workflow).
        let no_workflow = workflow.is_none_or(|w| w.steps_total == 0 && !complete);
        let steps = workflow.map(|w| format!("{}/{} steps", w.steps_completed, w.steps_total));
        let progress = || steps.clone().unwrap_or_else(|| "no workflow".to_string());
        let (verdict, summary): (VerdictStatus, String) = match status {
            "idle" if complete => (
                VerdictStatus::Completed,
                format!("workflow complete ({})", progress()),
            ),
            "idle" if no_workflow => (
                VerdictStatus::Completed,
                "went idle (no workflow assigned — nothing to complete)".to_string(),
            ),
            "idle" => (
                VerdictStatus::Incomplete,
                format!("went idle without completing the workflow ({})", progress()),
            ),
            "exited" => {
                let clean = reason.is_none_or(|r| r == "exit_code_0");
                if clean {
                    (
                        VerdictStatus::Incomplete,
                        "subagent exited before completion was observed".to_string(),
                    )
                } else {
                    (
                        VerdictStatus::Failed,
                        format!("subagent exited: {}", reason.unwrap_or("unknown")),
                    )
                }
            }
            // #925: a timeout fires only after idle/exit did NOT — the child is
            // almost always STILL RUNNING; frame as a check-in, not an error.
            "timeout" => {
                let at = steps
                    .as_deref()
                    .map(|s| format!(" at {s}"))
                    .unwrap_or_default();
                let msg = format!(
                    "await timed out; sub-agent still running{at} — \
                     re-await, steer, or wait (not an error)"
                );
                (VerdictStatus::Running, msg)
            }
            // Append the concrete run-level cause when surfaced (#752); keep the
            // summary derived here rather than post-mutated by callers.
            "error" => {
                let base = format!("await error: {}", reason.unwrap_or("unknown"));
                let summary = match error {
                    Some(cause) => format!("{base} — {cause}"),
                    None => base,
                };
                (VerdictStatus::Failed, summary)
            }
            other => (
                VerdictStatus::Incomplete,
                format!("subagent status: {other}"),
            ),
        };
        WorkflowResult {
            status: verdict,
            summary,
            workflow_progress,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::VerdictStatus;

    #[test]
    fn verdict_completed_when_idle_and_workflow_complete() {
        let wf = WorkflowSnapshot {
            mode: "complete".into(),
            steps_completed: 7,
            steps_total: 7,
        };
        let r = WorkflowResult::derive("idle", Some("idle"), Some(&wf), None);
        assert_eq!(r.status, VerdictStatus::Completed);
        assert_eq!(
            r.workflow_progress,
            Some(ResultProgress { done: 7, total: 7 })
        );
    }

    #[test]
    fn verdict_incomplete_when_idle_but_workflow_active() {
        let wf = WorkflowSnapshot {
            mode: "active".into(),
            steps_completed: 3,
            steps_total: 7,
        };
        let r = WorkflowResult::derive("idle", Some("idle"), Some(&wf), None);
        assert_eq!(r.status, VerdictStatus::Incomplete);
    }

    #[test]
    fn verdict_completed_when_idle_without_workflow() {
        // A plain-task agent (never given a workflow) that goes idle has
        // finished cleanly — there is nothing to "complete". It must NOT be
        // framed as incomplete/stalled, which previously confused parent agents.
        let r = WorkflowResult::derive("idle", Some("idle"), None, None);
        assert_eq!(r.status, VerdictStatus::Completed);
        assert!(r.workflow_progress.is_none());
        assert!(
            !r.summary.contains("without completing"),
            "summary must not imply a stall: {}",
            r.summary
        );
    }

    #[test]
    fn verdict_completed_when_idle_with_zero_step_template() {
        // Still selecting a template / zero steps bound == no workflow to
        // complete; going idle is a clean finish, not an abandonment.
        let wf = WorkflowSnapshot {
            mode: "selecting_template".into(),
            steps_completed: 0,
            steps_total: 0,
        };
        let r = WorkflowResult::derive("idle", Some("idle"), Some(&wf), None);
        assert_eq!(r.status, VerdictStatus::Completed);
    }

    #[test]
    fn verdict_failed_on_error_and_nonzero_exit() {
        assert_eq!(
            WorkflowResult::derive("error", Some("connection_failed"), None, None).status,
            VerdictStatus::Failed
        );
        assert_eq!(
            WorkflowResult::derive("exited", Some("exit_code_1"), None, None).status,
            VerdictStatus::Failed
        );
        // A clean exit is NOT completion — completion is observed at idle.
        assert_eq!(
            WorkflowResult::derive("exited", Some("exit_code_0"), None, None).status,
            VerdictStatus::Incomplete
        );
    }

    #[test]
    fn verdict_running_on_timeout() {
        // #925: a timeout is a check-in on a STILL-RUNNING child, not a failure or
        // "gave up" (see await_extra_tests for the progress framing with a workflow).
        let r = WorkflowResult::derive("timeout", None, None, None);
        assert_eq!(r.status, VerdictStatus::Running);
    }

    #[test]
    fn error_cause_is_threaded_into_summary_preserving_reason() {
        // #752: the run cause is appended to the verdict summary in one place
        // (derive), and the reason context (`agent_error`) is preserved.
        let r = WorkflowResult::derive("error", Some("agent_error"), None, Some("HTTP 429"));
        assert_eq!(r.status, VerdictStatus::Failed);
        assert_eq!(r.summary, "await error: agent_error — HTTP 429");
    }

    #[test]
    fn redact_secrets_strips_known_patterns_and_bounds_length() {
        // #752 security review: secrets must not cross the trust boundary into
        // the parent context verbatim.
        assert_eq!(
            redact_secrets("auth failed: Authorization: Bearer abc.def.ghi"),
            "auth failed: Authorization: [REDACTED]"
        );
        assert_eq!(
            redact_secrets("bad key sk-ABCDEFGH12345678 rejected"),
            "bad key [REDACTED] rejected"
        );
        assert_eq!(
            redact_secrets("url ?api_key=topsecret&x=1"),
            "url ?[REDACTED]"
        );
        assert_eq!(redact_secrets("token=hunter2"), "[REDACTED]");
        // Non-secret text is preserved verbatim.
        assert_eq!(redact_secrets("usage_limit_reached"), "usage_limit_reached");
        // Over-long causes are truncated.
        let long = "x".repeat(5000);
        let out = redact_secrets(&long);
        assert!(out.ends_with("…[truncated]"));
        assert!(out.len() < 5000);
    }

    #[test]
    fn with_error_redacts_cause_in_both_field_and_summary() {
        let r = AwaitResult::with_error(
            "error",
            Some("agent_error"),
            "bot-1".into(),
            10,
            None,
            Some("Authorization: Bearer sk-secrettoken123"),
        );
        let err = r.error.unwrap();
        assert!(
            !err.contains("sk-secrettoken123"),
            "field leaked secret: {err}"
        );
        assert!(err.contains("[REDACTED]"));
        assert!(
            !r.result.summary.contains("sk-secrettoken123"),
            "summary leaked secret: {}",
            r.result.summary
        );
    }
}
