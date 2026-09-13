//! Terminal handling of one monitored child (#1369 slice 3, #1936): the
//! monitor connection's EOF/reset or connect failure is a connection-level
//! observation of the child's end. It is handed to the application's
//! `ObserveOwnedChildExit`, which defers to the reaper while this harness
//! still retains the child's process and otherwise claims and runs the
//! exactly-once compensation (cleanup, membership, cascade removal, one
//! broadcast, exit signal and passive note).
use std::sync::Arc;

use crate::application::subagents::dto::{ObserveOwnedChildExitRequest, ObservedExit};
use crate::application::subagents::ports::ExitObservation;
use crate::application::subagents::use_cases::ObserveOwnedChildExit;

use super::SubagentRegistry;

pub(super) async fn notify_child_exited(
    registry: &SubagentRegistry,
    agent_id: &str,
    observer: &Arc<ObserveOwnedChildExit>,
    observation: ExitObservation,
) {
    let identity = {
        let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        entries.get(agent_id).map(|entry| {
            entry.delegated_identity().unwrap_or_else(|| {
                // A row without a launch generation (a fixture, a stub)
                // is still this harness's to compensate when its
                // connection ends; it just cannot be routed to.
                crate::domain::subagent_teardown::DelegatedAgentIdentity::new(
                    entry.agent_uuid.clone(),
                    crate::domain::subagent_teardown::LaunchGeneration::new(0),
                )
            })
        })
    };
    let Some(child) = identity else {
        tracing::debug!(agent = %agent_id, "monitor: connection ended for an unregistered child");
        return;
    };
    let observed = observer
        .execute(ObserveOwnedChildExitRequest { child, observation })
        .await;
    match observed {
        ObservedExit::Compensated { removed } => {
            tracing::info!(agent = %agent_id, removed = removed.len(), "monitor: child compensated");
        }
        ObservedExit::Joined(observation) => {
            tracing::debug!(agent = %agent_id, ?observation, "monitor: joined the child's compensation");
        }
        ObservedExit::DeferredToProcessExit => {
            tracing::debug!(agent = %agent_id, "monitor: connection ended; the reaper observes the exit");
        }
    }
}

pub fn notification_display_label(registry: &SubagentRegistry, agent_id: &str) -> String {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .map(|entry| entry.effective_display_name(agent_id).to_string())
        .unwrap_or_else(|| agent_id.to_string())
}

pub fn notification_agent_uuid(
    registry: &SubagentRegistry,
    agent_id: &str,
) -> crate::domain::ids::AgentUuid {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .map(|entry| entry.agent_uuid.clone())
        .unwrap_or_else(|| crate::domain::ids::AgentUuid::new(agent_id))
}
