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
