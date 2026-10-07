//! The one-line parent message each sub-agent notification reads as.
use super::SubagentNotification;

impl SubagentNotification {
    /// A child's exit note whose end is not known beyond `reason`.
    pub fn exited(agent_id: impl Into<String>, reason: Option<&str>) -> Self {
        Self::Exited {
            agent_id: agent_id.into(),
            reason: reason.map(str::to_owned),
            detail: None,
        }
    }

    /// Format this notification as a human-readable parent message.
    pub fn to_message(&self) -> String {
        // One line; soft, not imperative (#894); #926-AC2 actionability deferred.
        match self {
            // Every label and error a child chose is shown escaped and
            // capped (#2192 review): none can open a line of its own.
            Self::Completed { agent_id, .. } => format!(
                "Sub-agent '{}' ended a turn (status: idle). Inspect agent_cmd get_messages before treating its work as complete.",
                shown_label(agent_id)
            ),
            Self::Stalled {
                agent_id,
                workflow_mode,
                steps_completed,
                steps_total,
            } => format!(
                "Agent '{}' stalled: idle with workflow still {} at {steps_completed}/{steps_total}. Inspect output/state, then prompt, steer, abort, or kill it.",
                shown_label(agent_id),
                crate::domain::agents::value_objects::child_end::shown(workflow_mode, 32)
            ),
            Self::SwarmState { agent_id, state } => swarm_state_message(agent_id, state),
            Self::Errored { agent_id, error } => format!(
                "Agent '{}' failed: {}",
                shown_label(agent_id),
                crate::domain::agents::value_objects::child_end::shown(
                    error,
                    MAX_SHOWN_ERROR_BYTES
                )
            ),
            // The label may be a merged descendant's, from a child's own
            // snapshot: shown escaped and capped (#2192).
            Self::Exited {
                agent_id,
                detail: Some(end),
                ..
            } => format!("Sub-agent '{}' {end}.", shown_label(agent_id)),
            // Nothing known of the end: the same wording, with how it was
            // observed.
            Self::Exited {
                agent_id, reason, ..
            } => {
                let end =
                    crate::domain::agents::value_objects::child_end::ChildEnd::default().reason();
                let observed = match reason.as_deref() {
                    Some("") | None => String::new(),
                    Some(reason) => {
                        format!(
                            " ({})",
                            crate::domain::agents::value_objects::child_end::shown(reason, 64)
                        )
                    }
                };
                format!("Sub-agent '{}' {end}{observed}.", shown_label(agent_id))
            }
        }
    }
}

/// A swarm coordinator's wake note (#2467). Not built yet.
fn swarm_state_message(agent_id: &str, state: &super::SwarmNoteState) -> String {
    let _ = state;
    shown_label(agent_id)
}

/// The most of a child's error a notice shows, in bytes.
const MAX_SHOWN_ERROR_BYTES: usize = 512;

/// A sub-agent's label as a notice shows it: escaped and capped.
fn shown_label(label: &str) -> String {
    crate::domain::agents::value_objects::child_end::shown(
        label,
        crate::domain::agents::value_objects::child_end::MAX_SHOWN_NAME_BYTES,
    )
}
