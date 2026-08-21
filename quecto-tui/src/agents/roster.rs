use std::collections::BTreeMap;
use std::time::Duration;

const STATUS_EXITED: &str = "exited";

/// Minimal policy-facing view of a roster entry. Implementations adapt concrete
/// transport payloads outside this pure module.
pub(crate) trait RosterInfo: Clone {
    fn status(&self) -> &str;
    fn parent_id(&self) -> Option<&str>;
    fn agent_uuid(&self) -> Option<&str>;
    fn display_label(&self) -> &str;

    /// Preserve sticky metadata when lossy roster polls omit it.
    fn merge_sticky_fields(&mut self, previous: &Self);
}

/// Subagent entry with optional expiry timestamp (#540).
#[derive(Debug, Clone)]
pub(crate) struct TrackedSubagent<I: RosterInfo> {
    pub(crate) info: I,
    /// When the subagent was first observed (for the elapsed-time display).
    pub(crate) started_at: tokio::time::Instant,
    /// When the subagent last stopped being active (idle/error/exited), used to
    /// freeze the elapsed-time display. `None` while active.
    pub(crate) stopped_at: Option<tokio::time::Instant>,
    /// When the subagent entered the "exited" state. `None` if not exited. GC
    /// grace counts from exit, while the timer freezes from first going idle.
    pub(crate) exited_at: Option<tokio::time::Instant>,
    /// `true` while this is a local optimistic guess (from the spawn ToolStart)
    /// the kernel has not yet confirmed. Such an entry is not dropped just
    /// because a snapshot predating its registration omits it (#866); cleared
    /// once any payload includes the agent.
    pub(crate) optimistic: bool,
    /// Direct feed whose roster is authoritative for this entry. Master snapshots
    /// may preserve membership, but must not overwrite fresher direct metadata.
    pub(crate) roster_source: Option<String>,
}

/// Whether a subagent status counts as "actively running" for the timer.
pub(crate) fn subagent_status_is_active(status: &str) -> bool {
    matches!(status, "starting" | "running")
}

/// Whether a subagent status is terminal — the child process is gone and can
/// never receive a send. The status vocabulary lives here beside
/// [`subagent_status_is_active`] so callers don't string-match it themselves
/// (PR #1485 review).
pub(crate) fn subagent_status_is_terminal(status: &str) -> bool {
    matches!(status, "dead" | "exited")
}

pub(crate) fn next_exited_subagent_gc_deadline<I: RosterInfo>(
    map: &BTreeMap<String, TrackedSubagent<I>>,
    grace: Duration,
) -> Option<tokio::time::Instant> {
    if map
        .values()
        .any(|entry| subagent_status_is_active(entry.info.status()))
    {
        return None;
    }
    map.values()
        .filter_map(|entry| entry.exited_at.map(|exited_at| exited_at + grace))
        .min()
}

impl<I: RosterInfo> TrackedSubagent<I> {
    pub(crate) fn new(info: I) -> Self {
        Self::new_at(info, tokio::time::Instant::now())
    }

    /// Clock-injected constructor used by snapshot application so lifecycle
    /// timestamps come from the caller's single `now` reading.
    pub(crate) fn new_at(info: I, now: tokio::time::Instant) -> Self {
        let active = subagent_status_is_active(info.status());
        let exited_at = (info.status() == STATUS_EXITED).then_some(now);
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
    pub(crate) fn elapsed_secs(&self, now: tokio::time::Instant) -> u64 {
        let end = self.stopped_at.unwrap_or(now);
        end.saturating_duration_since(self.started_at).as_secs()
    }

    /// Update the info, freezing the timer when the agent stops being active and
    /// recording exited_at on transition to "exited".
    #[cfg(test)]
    pub(crate) fn update_info(&mut self, new_info: I) {
        self.update_info_at(new_info, tokio::time::Instant::now());
    }

    /// Clock-injected update used by snapshot application.
    pub(crate) fn update_info_at(&mut self, mut new_info: I, now: tokio::time::Instant) {
        new_info.merge_sticky_fields(&self.info);
        if subagent_status_is_active(new_info.status()) {
            // Resumed work — let the timer run again.
            self.stopped_at = None;
        } else if self.stopped_at.is_none() {
            // First transition into a stopped state — freeze the timer here.
            self.stopped_at = Some(now);
        }
        if new_info.status() == STATUS_EXITED && self.exited_at.is_none() {
            self.exited_at = Some(now);
        } else if new_info.status() != STATUS_EXITED {
            self.exited_at = None;
        }
        self.info = new_info;
    }
}

/// Remove exited subagents whose grace period has elapsed (#540). Returns `true`
/// if any entries were removed. While any sibling is still active, finished
/// agents are kept on screen so the panel doesn't shrink mid-batch and jolt the
/// chat above it — reclamation waits until the whole batch is quiescent.
pub(crate) fn gc_exited_subagents<I: RosterInfo>(
    map: &mut BTreeMap<String, TrackedSubagent<I>>,
    now: tokio::time::Instant,
    grace: Duration,
) -> bool {
    if map
        .values()
        .any(|entry| subagent_status_is_active(entry.info.status()))
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

pub(crate) fn apply_roster_snapshot<I: RosterInfo>(
    tracked: &mut BTreeMap<String, TrackedSubagent<I>>,
    source_agent_id: Option<&str>,
    candidates: BTreeMap<String, I>,
    now: tokio::time::Instant,
    exited_grace: Duration,
    optimistic_grace: Duration,
) {
    let mut incoming = BTreeMap::new();
    if let Some(source) = source_agent_id {
        // Accept the source's existing subtree plus descendants introduced in
        // this same snapshot. Existing IDs outside that subtree remain owned
        // by their current authority and cannot be hijacked or cycled.
        let mut candidates = candidates;
        loop {
            let before = incoming.len();
            candidates.retain(|id, s| {
                let existing_owned =
                    !tracked.contains_key(id) || is_descendant_of(id, source, tracked);
                let parent_owned = s.parent_id() == Some(source)
                    || s.parent_id().is_some_and(|parent| {
                        incoming.contains_key(parent) || is_descendant_of(parent, source, tracked)
                    });
                if id != source && existing_owned && parent_owned {
                    incoming.insert(id.clone(), s.clone());
                    false
                } else {
                    true
                }
            });
            if incoming.len() == before {
                break;
            }
        }
    } else {
        incoming = candidates;
    }

    let mut new_map = tracked.clone();
    match source_agent_id {
        None => {
            new_map.retain(|id, _entry| {
                incoming.contains_key(id) || has_incoming_root_ancestor(id, &incoming, tracked)
            });
        }
        Some(source) => {
            new_map.retain(|id, _entry| {
                incoming.contains_key(id) || !is_descendant_of(id, source, tracked)
            });
        }
    }

    for (id, info) in incoming {
        if let Some(mut existing) = new_map.remove(&id) {
            existing.optimistic = false;
            if source_agent_id.is_some() || existing.roster_source.is_none() {
                existing.update_info_at(info, now);
            }
            if source_agent_id.is_some() {
                existing.roster_source = source_agent_id.map(str::to_string);
            }
            new_map.insert(id, existing);
        } else if let Some(mut existing) = tracked.get(&id).cloned() {
            existing.optimistic = false;
            if source_agent_id.is_some() || existing.roster_source.is_none() {
                existing.update_info_at(info, now);
            }
            if source_agent_id.is_some() {
                existing.roster_source = source_agent_id.map(str::to_string);
            }
            new_map.insert(id, existing);
        } else {
            let mut entry = TrackedSubagent::new_at(info, now);
            entry.roster_source = source_agent_id.map(str::to_string);
            new_map.insert(id, entry);
        }
    }

    let leftover = std::mem::take(tracked);
    let mut pending: Vec<String> = new_map
        .values()
        .filter_map(|t| t.info.parent_id().map(str::to_string))
        .collect();
    while let Some(pid) = pending.pop() {
        if new_map.contains_key(&pid) {
            continue;
        }
        if let Some(entry) = leftover.get(&pid) {
            if let Some(grandparent) = entry.info.parent_id().map(str::to_string) {
                pending.push(grandparent);
            }
            new_map.insert(pid, entry.clone());
        }
    }

    for (id, entry) in leftover {
        if new_map.contains_key(&id)
            || source_agent_id.is_some_and(|source| is_descendant_of(&id, source, &new_map))
        {
            continue;
        }
        if let Some(exited_at) = entry.exited_at {
            if now.saturating_duration_since(exited_at) < exited_grace {
                new_map.entry(id).or_insert(entry);
            }
        } else if entry.optimistic
            && now.saturating_duration_since(entry.started_at) < optimistic_grace
        {
            new_map.entry(id).or_insert(entry);
        }
    }
    *tracked = new_map;
}

fn has_incoming_root_ancestor<I: RosterInfo>(
    id: &str,
    incoming: &BTreeMap<String, I>,
    map: &BTreeMap<String, TrackedSubagent<I>>,
) -> bool {
    let mut current = map.get(id).and_then(|entry| entry.info.parent_id());
    let mut guard = 0usize;
    while let Some(parent) = current {
        if incoming
            .get(parent)
            .is_some_and(|entry| entry.parent_id().is_none())
        {
            return true;
        }
        guard += 1;
        if guard > map.len() {
            return false;
        }
        current = map.get(parent).and_then(|entry| entry.info.parent_id());
    }
    false
}

fn is_descendant_of<I: RosterInfo>(
    id: &str,
    ancestor: &str,
    map: &BTreeMap<String, TrackedSubagent<I>>,
) -> bool {
    let mut current = map.get(id).and_then(|entry| entry.info.parent_id());
    let mut guard = 0usize;
    while let Some(parent) = current {
        if parent == ancestor {
            return true;
        }
        guard += 1;
        if guard > map.len() {
            return false;
        }
        current = map.get(parent).and_then(|entry| entry.info.parent_id());
    }
    false
}
