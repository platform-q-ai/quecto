//! #2125: refusals that name the next valid step. A weaker model given only
//! "swarm is setup" or "unknown op stop" flailed and gave up; each message
//! here says what is allowed now and how to move on.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::domain::swarm::RefusalKind;

/// Every op the swarm tool names in its guidance: the harness's own, then
/// the structured board ops (#2279), each by the name the schema
/// advertises (#2478 slice B: a task item's in place of a task's; the old
/// names are still taken), both read from the one op table
/// (`swarm_board_ops`), so the two cannot drift.
pub(crate) fn valid_ops() -> Vec<&'static str> {
    use super::super::swarm_board_ops::{BOARD_OPS, HARNESS_OPS, advertised_op};
    HARNESS_OPS
        .iter()
        .copied()
        .chain(BOARD_OPS.iter().map(|spec| advertised_op(spec.name)))
        .collect()
}

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
             events, {USAGE}, reconcile."
        ),
        Some("paused") => format!(
            "the swarm run is paused, so op={op} is unavailable. Allowed: \
             summary, events, {USAGE}, reconcile; the coordinator may also \
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
    format!("op is required; valid ops: {}.", valid_ops().join(", "))
}

/// The refusal for a call whose op is present but not a string (a number,
/// `null`, a list): not a missing op (#2282 review N1).
pub(super) fn op_not_a_string() -> String {
    format!(
        "op must be a string naming one of: {}.",
        valid_ops().join(", ")
    )
}

/// The refusal for an op the tool does not have, the removed Python
/// workbench ops (`run`, `status`, `output`, `cancel`, #2282) included.
pub(super) fn unknown_op(op: &str) -> String {
    format!(
        "unknown op {op}; valid ops: {}. To end a run, the coordinator calls op=stop \
         (status, reason) or op=complete (revision); op=cancel_run cancels it for the parent \
         or user.",
        valid_ops().join(", ")
    )
}

/// The running gate of a structured board op (`op=run`'s until #2282
/// removed it; #2478 moved it here, beside its guidance): the refusal's
/// kind, the gate's name and the guidance, unless the run is running within
/// its deadline.
pub(crate) fn running_gate(
    op: &str,
    status: &Value,
) -> Option<(RefusalKind, &'static str, String)> {
    let guide = op_refused;
    match status["status"].as_str() {
        Some("running") => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs_f64();
            match status["deadline"].as_f64() {
                Some(deadline) if deadline > now => None,
                Some(_) | None => Some((
                    RefusalKind::BudgetExhausted,
                    "deadline",
                    op_deadline_passed(op),
                )),
            }
        }
        Some(other) => Some((
            RefusalKind::NotRunning,
            "not_running",
            guide(op, Some(other)),
        )),
        None => Some((RefusalKind::Store, "unreadable", guide(op, None))),
    }
}

#[cfg(test)]
#[path = "swarm_guidance_tests.rs"]
mod tests;
