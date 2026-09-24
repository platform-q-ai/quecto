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
    "run",
    "status",
    "output",
    "cancel",
];

/// Why `op=run` is refused while the run is in `status` (anything but
/// running; `None` when the status could not be read).
pub(super) fn run_refused(status: Option<&str>) -> String {
    match status {
        Some("setup") => "no swarm run exists yet (status setup), so op=run is unavailable. \
             Next: swarm {\"op\":\"create\",\"goal\":\"...\",\"constraints\":[],\"criteria\":\
             [{\"id\":\"tests\",\"kind\":\"command\",\"description\":\"...\"}],\
             \"member_limit\":3,\"deadline_in_seconds\":3600}. Allowed now: create, summary, \
             events, usage, usage_budget, reconcile."
            .to_string(),
        Some("paused") => "the swarm run is paused, so op=run is unavailable. Allowed: \
             summary, events, usage, usage_budget, reconcile, cancel_run; the supervisor \
             outside the swarm resumes or closes it (agent_cmd swarm_control)."
            .to_string(),
        Some(ended) => format!(
            "the swarm run is {ended}, so op=run is unavailable. Allowed: summary, events, \
             usage; report the outcome from the summary."
        ),
        None => "the swarm run status could not be read; call op=summary.".to_string(),
    }
}

/// Why `op=run` is refused once the running run's deadline has passed.
pub(super) fn deadline_passed() -> String {
    "the swarm run's deadline has passed (budget-exhausted), so op=run is unavailable. \
     Allowed: summary, events, usage; the supervisor outside the swarm grants more time \
     (agent_cmd swarm_control extend)."
        .to_string()
}

/// The refusal for an op the tool does not have.
pub(super) fn unknown_op(op: &str) -> String {
    format!(
        "unknown op {op}; valid ops: {}. To end a run, the coordinator calls \
         board.stop(status, reason) or board.complete(revision) via op=run; op=cancel_run \
         cancels it for the parent or user.",
        VALID_OPS.join(", ")
    )
}

#[cfg(test)]
#[path = "swarm_guidance_tests.rs"]
mod tests;
