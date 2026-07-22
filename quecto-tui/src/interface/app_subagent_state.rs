use super::*;

/// Subagent entry with optional expiry timestamp (#540).
#[derive(Debug, Clone)]
pub(super) struct TrackedSubagent {
    pub(super) info: crate::infrastructure::client::SubagentInfoEvent,
    /// When the subagent was first observed (for the elapsed-time display).
    pub(super) started_at: tokio::time::Instant,
    /// When the subagent last stopped being active (idle/error/exited), used to
    /// freeze the elapsed-time display. `None` while active.
    pub(super) stopped_at: Option<tokio::time::Instant>,
    /// When the subagent entered the "exited" state. `None` if not exited. GC
    /// grace counts from exit, while the timer freezes from first going idle.
    pub(super) exited_at: Option<tokio::time::Instant>,
    /// `true` while this is a local optimistic guess (from the spawn ToolStart)
    /// the kernel has not yet confirmed. Such an entry is not dropped just
    /// because a snapshot predating its registration omits it (#866); cleared
    /// once any payload includes the agent.
    pub(super) optimistic: bool,
    /// Direct feed whose roster is authoritative for this entry. Master snapshots
    /// may preserve membership, but must not overwrite fresher direct metadata.
    pub(super) roster_source: Option<String>,
}

/// Whether a subagent status counts as "actively running" for the timer.
pub(super) fn subagent_status_is_active(status: &str) -> bool {
    matches!(status, "starting" | "running")
}

pub(super) fn next_exited_subagent_gc_deadline(
    map: &std::collections::BTreeMap<String, TrackedSubagent>,
    grace: Duration,
) -> Option<tokio::time::Instant> {
    if map
        .values()
        .any(|entry| subagent_status_is_active(&entry.info.status))
    {
        return None;
    }
    map.values()
        .filter_map(|entry| entry.exited_at.map(|exited_at| exited_at + grace))
        .min()
}

impl TrackedSubagent {
    pub(super) fn new(info: crate::infrastructure::client::SubagentInfoEvent) -> Self {
        let now = tokio::time::Instant::now();
        let active = subagent_status_is_active(&info.status);
        let exited_at = (info.status == STATUS_EXITED).then_some(now);
        Self {
            info,
            started_at: now,
            stopped_at: if active { None } else { Some(now) },
            exited_at,
            optimistic: false,
            roster_source: None,
        }
    }

    /// Seconds the agent was actively running, frozen once it goes idle/exits.
    pub(super) fn elapsed_secs(&self, now: tokio::time::Instant) -> u64 {
        let end = self.stopped_at.unwrap_or(now);
        end.saturating_duration_since(self.started_at).as_secs()
    }

    /// Update the info, freezing the timer when the agent stops being active and
    /// recording exited_at on transition to "exited".
    pub(super) fn update_info(
        &mut self,
        mut new_info: crate::infrastructure::client::SubagentInfoEvent,
    ) {
        // Preserve last-known workflow + parent_id when an update omits them
        // (get_subagents carries neither, and would otherwise erase the n/n).
        if new_info.workflow.is_none() {
            new_info.workflow = self.info.workflow.clone();
        }
        if new_info.parent_id.is_none() {
            new_info.parent_id = self.info.parent_id.clone();
        }
        let now = tokio::time::Instant::now();
        if subagent_status_is_active(&new_info.status) {
            // Resumed work — let the timer run again.
            self.stopped_at = None;
        } else if self.stopped_at.is_none() {
            // First transition into a stopped state — freeze the timer here.
            self.stopped_at = Some(now);
        }
        if new_info.status == STATUS_EXITED && self.exited_at.is_none() {
            self.exited_at = Some(now);
        } else if new_info.status != STATUS_EXITED {
            self.exited_at = None;
        }
        self.info = new_info;
    }
}

/// Remove exited subagents whose grace period has elapsed (#540). Returns `true`
/// if any entries were removed. While any sibling is still active, finished
/// agents are kept on screen so the panel doesn't shrink mid-batch and jolt the
/// chat above it — reclamation waits until the whole batch is quiescent.
pub(super) fn gc_exited_subagents(
    map: &mut std::collections::BTreeMap<String, TrackedSubagent>,
    now: tokio::time::Instant,
    grace: Duration,
) -> bool {
    if map
        .values()
        .any(|entry| subagent_status_is_active(&entry.info.status))
    {
        return false;
    }
    let mut removed = false;
    map.retain(|_, entry| match entry.exited_at {
        Some(exited_at) => {
            let keep = now.saturating_duration_since(exited_at) < grace;
            if !keep {
                removed = true;
            }
            keep
        }
        None => true,
    });
    removed
}
