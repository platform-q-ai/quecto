//! `delete_all_subagents`: invoke the fleet teardown and present it (#1938).
//!
//! Both delivery paths — the dispatch loop when the harness is idle and the
//! connection's reader task when a turn is running (#1626) — invoke the one
//! [`TerminateAllDelegatedAgents`] use case with reason `operator_request`
//! and present its outcome. Nothing is drained, signalled or broadcast here:
//! the fleet teardown claims, asks, concludes and compensates every direct
//! child (the compensation broadcasts the survivor set), then prunes the
//! exited tombstones so the roster the client sees is empty.
use std::sync::Arc;

use crate::application::subagents::dto::{
    FleetTeardownError, FleetTeardownOutcome, TerminateAllDelegatedAgentsRequest,
};
use crate::application::subagents::use_cases::TerminateAllDelegatedAgents;
use crate::domain::subagent_teardown::ShutdownReason;

use super::protocol::AgentEvent;

const COMMAND: &str = "delete_all_subagents";

/// Run the fleet teardown for an operator and present the correlated
/// response. A harness without a fleet teardown (no subagent registry) has
/// nothing to delete and answers with a correlated error.
pub(super) async fn respond(
    fleet: Option<&Arc<TerminateAllDelegatedAgents>>,
    id: Option<&str>,
) -> AgentEvent {
    let Some(fleet) = fleet else {
        return AgentEvent::err(id, COMMAND, "no sub-agent registry available");
    };
    match fleet
        .execute(TerminateAllDelegatedAgentsRequest {
            reason: ShutdownReason::OperatorRequest,
            // delete-all is the owner's explicit word (#2070).
            authority: crate::application::subagents::dto::FleetTeardownAuthority::Owner,
        })
        .await
    {
        Ok(outcome) => AgentEvent::ok(id, COMMAND, Some(present(&outcome))),
        Err(FleetTeardownError::Interrupted) => {
            AgentEvent::err(id, COMMAND, FleetTeardownError::Interrupted.to_string())
        }
    }
}

/// The wire shape: `removed` counts every row the run moved out of the
/// roster (settled children and pruned tombstones, as before); `settled`
/// and `unsettled` say how each direct child ended.
fn present(outcome: &FleetTeardownOutcome) -> serde_json::Value {
    serde_json::json!({
        "removed": outcome.removed_count(),
        "joined": outcome.joined,
        "settled": outcome
            .settled
            .iter()
            .map(|settled| serde_json::json!({
                "agent": settled.child.uuid.as_str(),
                "result": settled.result.as_str(),
            }))
            .collect::<Vec<_>>(),
        "unsettled": outcome
            .unsettled
            .iter()
            .map(|(uuid, detail)| serde_json::json!({
                "agent": uuid.as_str(),
                "detail": detail,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
#[path = "uds_delete_all_subagents_tests.rs"]
mod tests;
