#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentNotification {
    /// Child agent ended a turn and was observed idle; this is not a task-success verdict.
    Completed { agent_id: String },
    /// Workflow-bound child became idle before reaching a terminal workflow state.
    Stalled {
        agent_id: String,
        workflow_mode: String,
        steps_completed: u64,
        steps_total: u64,
    },
    /// Child agent's last tool execution returned an error.
    Errored { agent_id: String, error: String },
    /// Child agent process exited (connection closed or process reaped).
    Exited { agent_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequencedSubagentNotification {
    pub sequence: u64,
    pub notification: SubagentNotification,
    /// Hidden generation identity for internal routing (#1378).
    pub agent_uuid: Option<crate::domain::ids::AgentUuid>,
}

impl SequencedSubagentNotification {
    pub fn new(sequence: u64, notification: SubagentNotification) -> Self {
        Self {
            sequence,
            notification,
            agent_uuid: None,
        }
    }

    pub fn new_for_agent(
        sequence: u64,
        notification: SubagentNotification,
        agent_uuid: crate::domain::ids::AgentUuid,
    ) -> Self {
        Self {
            sequence,
            notification,
            agent_uuid: Some(agent_uuid),
        }
    }

    pub fn dedupe_key(&self) -> (String, u64) {
        let agent_id = match &self.notification {
            SubagentNotification::Completed { agent_id, .. }
            | SubagentNotification::Stalled { agent_id, .. }
            | SubagentNotification::Errored { agent_id, .. }
            | SubagentNotification::Exited { agent_id } => agent_id.clone(),
        };
        (agent_id, self.sequence)
    }

    /// Internal await-dedupe reference: UUID when stamped, else display label.
    pub fn await_dedupe_key(&self) -> (String, u64) {
        self.agent_uuid
            .as_ref()
            .map(|uuid| (uuid.to_string(), self.sequence))
            .unwrap_or_else(|| self.dedupe_key())
    }

    pub fn to_message(&self) -> String {
        self.notification.to_message()
    }

    /// `true` only for normal idle turn ends; failures must not coalesce (#894).
    pub fn is_completion(&self) -> bool {
        matches!(self.notification, SubagentNotification::Completed { .. })
    }
}

impl SubagentNotification {
    /// Format this notification as a human-readable parent message.
    pub fn to_message(&self) -> String {
        // One line; soft, not imperative (#894); #926-AC2 actionability deferred.
        match self {
            Self::Completed { agent_id, .. } => format!(
                "Sub-agent '{agent_id}' ended a turn (status: idle). Inspect agent_cmd get_messages before treating its work as complete."
            ),
            Self::Stalled {
                agent_id,
                workflow_mode,
                steps_completed,
                steps_total,
            } => format!(
                "Agent '{agent_id}' stalled: idle with workflow still {workflow_mode} at {steps_completed}/{steps_total}. Inspect output/state, then prompt, steer, abort, or kill it."
            ),
            Self::Errored { agent_id, error } => format!("Agent '{agent_id}' failed: {error}"),
            Self::Exited { agent_id } => format!("Agent '{agent_id}' exited unexpectedly"),
        }
    }
}

/// Sender half of the notification channel.
pub type NotificationTx = tokio::sync::mpsc::Sender<SequencedSubagentNotification>;

/// Receiver half of the notification channel.
pub type NotificationRx = tokio::sync::mpsc::Receiver<SequencedSubagentNotification>;
