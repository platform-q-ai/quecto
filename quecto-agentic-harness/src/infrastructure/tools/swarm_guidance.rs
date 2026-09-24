//! #2125: refusals that name the next valid step. A weaker model given only
//! "swarm is setup" or "unknown op stop" flailed and gave up; each message
//! here says what is allowed now and how to move on.

/// Every op the swarm tool accepts.
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

/// Why `op=run` is refused while the run is `status` (anything but running).
pub(super) fn run_refused(status: &str) -> String {
    match status {
        "setup" => "no swarm run exists yet (status setup), so op=run is unavailable. Next: \
             swarm {\"op\":\"create\",\"goal\":\"...\",\"constraints\":[],\"criteria\":\
             [{\"id\":\"tests\",\"kind\":\"command\",\"description\":\"...\"}],\
             \"member_limit\":3,\"deadline\":<unix seconds within 7 days>}. \
             Allowed now: create, summary, events, usage."
            .to_string(),
        "paused" => "the swarm run is paused, so op=run is unavailable. Allowed: summary, \
             events, usage; only the supervisor outside the swarm resumes or closes it \
             (agent_cmd swarm_control)."
            .to_string(),
        other => format!(
            "the swarm run is {other}, so op=run is unavailable. Allowed: summary, events, \
             usage; report the outcome from the summary."
        ),
    }
}

/// The refusal for an op the tool does not have.
pub(super) fn unknown_op(op: &str) -> String {
    format!(
        "unknown op {op}; valid ops: {}. To end a run, the coordinator calls \
         board.stop(status, reason) or board.complete(revision) via op=run; the parent \
         uses op=cancel_run.",
        VALID_OPS.join(", ")
    )
}

#[cfg(test)]
#[path = "swarm_guidance_tests.rs"]
mod tests;
