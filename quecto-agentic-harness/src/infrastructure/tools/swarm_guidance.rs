//! #2125: refusals that name the next valid step. A weaker model given only
//! "swarm is setup" or "unknown op stop" flailed and gave up; each message
//! here says what is allowed now and how to move on.

/// Every op the swarm tool accepts (pinned to the schema's `op` enum).
pub(super) const VALID_OPS: &[&str] = &[
    "create",
    "summary",
    "reconcile",
    "cancel_run",
    "pause",
    "resume",
    "events",
    "usage",
    "usage_budget",
    // The structured board ops (#2279), `swarm_board_ops::BOARD_OPS`.
    "task",
    "tasks",
    "file_owners",
    "task_create",
    "dependencies",
    "claim",
    "release",
    "block",
    "unblock",
    "submit",
    "reserve",
    "release_files",
    "send",
    "withdraw",
    "inbox",
    "ack",
    "evidence",
    "amend",
    "verify_task",
    "revalidate_task",
    "recover",
    "revoke",
    "complete",
    "stop",
    "usage_report",
];

/// The harness op `usage` as a refusal allows it (#2279 review N9): not
/// its board-op alias `usage_report`, which the running gate refuses.
const USAGE: &str = "usage (op=usage; the board op usage_report needs a running run)";

/// Why `op` (a structured board op, #2279, which keeps the removed
/// `op=run`'s running gate by owner decision) is refused while the run is
/// in `status` (anything but running; `None` when the status could not be
/// read).
pub(crate) fn op_refused(op: &str, status: Option<&str>) -> String {
    match status {
        Some("setup") => format!(
            "no swarm run exists yet (status setup), so op={op} is unavailable. \
             Next: swarm {{\"op\":\"create\",\"goal\":\"...\",\"constraints\":[],\"criteria\":\
             [{{\"id\":\"tests\",\"kind\":\"command\",\"description\":\"...\"}}],\
             \"member_limit\":3,\"deadline_in_seconds\":3600}}. Allowed now: create, summary, \
             events, {USAGE}, usage_budget, reconcile."
        ),
        Some("paused") => format!(
            "the swarm run is paused, so op={op} is unavailable. Allowed: \
             summary, events, {USAGE}, reconcile; the coordinator may also usage_budget and \
             cancel_run; the supervisor outside the swarm resumes or closes it (agent_cmd \
             swarm_control)."
        ),
        Some(ended) => format!(
            "the swarm run is {ended}, so op={op} is unavailable. Allowed: summary, events, \
             {USAGE}; report the outcome from the summary."
        ),
        None => "the swarm run status could not be read; call op=summary.".to_string(),
    }
}

/// Why `op` is refused once the running run's deadline has passed.
pub(crate) fn op_deadline_passed(op: &str) -> String {
    format!(
        "the swarm run's deadline has passed (budget-exhausted), so op={op} is unavailable. \
         Allowed: summary, events, {USAGE}; the supervisor outside the swarm grants more time \
         (agent_cmd swarm_control extend)."
    )
}

/// The Python workbench ops #2282 removed: unknown ops now, counted apart in
/// the refusal's telemetry.
pub(super) const REMOVED_OPS: &[&str] = &["run", "status", "output", "cancel"];

/// The refusal for a call that names no op (#2282: a call without one no
/// longer runs Python).
pub(super) fn op_required() -> String {
    format!("op is required; valid ops: {}.", VALID_OPS.join(", "))
}

/// The refusal for a call whose op is present but not a string (a number,
/// `null`, a list): not a missing op (#2282 review N1).
pub(super) fn op_not_a_string() -> String {
    format!(
        "op must be a string naming one of: {}.",
        VALID_OPS.join(", ")
    )
}

/// The refusal for an op the tool does not have, the removed Python
/// workbench ops (`run`, `status`, `output`, `cancel`, #2282) included.
pub(super) fn unknown_op(op: &str) -> String {
    format!(
        "unknown op {op}; valid ops: {}. To end a run, the coordinator calls op=stop \
         (status, reason) or op=complete (revision); op=cancel_run cancels it for the parent \
         or user.",
        VALID_OPS.join(", ")
    )
}

#[cfg(test)]
#[path = "swarm_guidance_tests.rs"]
mod tests;
