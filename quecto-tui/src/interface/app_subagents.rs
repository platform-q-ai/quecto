use super::*;

pub(crate) fn usable_socket_path(path: Option<&str>) -> bool {
    path.is_some_and(|p| {
        let p = p.trim();
        !p.is_empty() && std::path::Path::new(p).is_absolute()
    })
}

fn has_incoming_root_ancestor(
    id: &str,
    incoming: &std::collections::BTreeMap<String, crate::infrastructure::client::SubagentInfoEvent>,
    map: &std::collections::BTreeMap<String, TrackedSubagent>,
) -> bool {
    let mut current = map
        .get(id)
        .and_then(|entry| entry.info.parent_id.as_deref());
    let mut guard = 0usize;
    while let Some(parent) = current {
        if incoming
            .get(parent)
            .is_some_and(|entry| entry.parent_id.is_none())
        {
            return true;
        }
        guard += 1;
        if guard > map.len() {
            return false;
        }
        current = map
            .get(parent)
            .and_then(|entry| entry.info.parent_id.as_deref());
    }
    false
}

fn is_descendant_of(
    id: &str,
    ancestor: &str,
    map: &std::collections::BTreeMap<String, TrackedSubagent>,
) -> bool {
    let mut current = map
        .get(id)
        .and_then(|entry| entry.info.parent_id.as_deref());
    let mut guard = 0usize;
    while let Some(parent) = current {
        if parent == ancestor {
            return true;
        }
        guard += 1;
        if guard > map.len() {
            return false;
        }
        current = map
            .get(parent)
            .and_then(|entry| entry.info.parent_id.as_deref());
    }
    false
}

/// Grace window during which an unconfirmed optimistic subagent entry (created
/// from the spawn ToolStart, before the kernel registers the child) survives a
/// snapshot that omits it. Generous enough to bridge slow socket readiness, yet
/// bounded so a never-confirmed spawn (failed launch) can't linger forever (#866).
const OPTIMISTIC_SUBAGENT_GRACE: Duration = Duration::from_secs(30);

impl App {
    /// Update a session's OWN footer (context-window / cost / model) from a
    /// forwarded sub-agent event, mirroring the master footer path (#805):
    /// `get_state` carries the model and window, `turn_end` the live context
    /// usage, and `get_session_stats` the cumulative cost (plus usage fallback).
    pub(super) fn update_session_footer(session: &mut SessionView, ev: &Event) {
        use crate::application::session_payloads;
        match ev {
            Event::Response {
                command,
                data: Some(data),
                success: true,
                ..
            } if command == "get_state" => {
                session.footer.apply_get_state(data);
            }
            Event::Response {
                command,
                data: Some(data),
                success: true,
                ..
            } if command == "get_session_stats" => {
                let stats = session_payloads::parse_session_stats(data);
                session.footer.apply_session_stats(&stats);
            }
            Event::TurnEnd { message } => {
                let used = message.get("contextTokens").and_then(|v| v.as_u64());
                let window = message
                    .get("maxContextTokens")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                if let (Some(used), Some(window)) = (used, window) {
                    session.footer.update_context_usage(used, window);
                }
            }
            _ => {}
        }
    }

    pub(super) fn update_subagent_bar(
        &mut self,
        subagents: Vec<crate::infrastructure::client::SubagentInfoEvent>,
    ) {
        self.update_subagent_bar_from_source(None, subagents);
    }

    pub(super) fn update_subagent_bar_from_source(
        &mut self,
        source_agent_id: Option<&str>,
        subagents: Vec<crate::infrastructure::client::SubagentInfoEvent>,
    ) {
        let source_agent_id = source_agent_id.map(sanitize_agent_id);
        let mut candidates = std::collections::BTreeMap::new();
        for mut s in subagents {
            if !usable_socket_path(s.socket_path.as_deref()) {
                s.socket_path = None;
            }
            candidates.insert(sanitize_agent_id(&s.agent_id), s);
        }
        let mut incoming = std::collections::BTreeMap::new();
        if let Some(source) = source_agent_id.as_deref() {
            // Accept the source's existing subtree plus descendants introduced in
            // this same snapshot. Existing IDs outside that subtree remain owned
            // by their current authority and cannot be hijacked or cycled.
            loop {
                let before = incoming.len();
                candidates.retain(|id, s| {
                    let existing_owned = !self.subagents.tracked.contains_key(id)
                        || is_descendant_of(id, source, &self.subagents.tracked);
                    let parent_owned = s.parent_id.as_deref() == Some(source)
                        || s.parent_id.as_deref().is_some_and(|parent| {
                            incoming.contains_key(parent)
                                || is_descendant_of(parent, source, &self.subagents.tracked)
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

        let mut new_map = self.subagents.tracked.clone();
        match source_agent_id.as_deref() {
            None => {
                new_map.retain(|id, _entry| {
                    incoming.contains_key(id)
                        || has_incoming_root_ancestor(id, &incoming, &self.subagents.tracked)
                });
            }
            Some(source) => {
                new_map.retain(|id, _entry| {
                    incoming.contains_key(id)
                        || !is_descendant_of(id, source, &self.subagents.tracked)
                });
            }
        }

        for (id, s) in incoming {
            if let Some(mut existing) = new_map.remove(&id) {
                existing.optimistic = false;
                if source_agent_id.is_some() || existing.roster_source.is_none() {
                    existing.update_info(s);
                }
                if source_agent_id.is_some() {
                    existing.roster_source = source_agent_id.clone();
                }
                new_map.insert(id, existing);
            } else if let Some(mut existing) = self.subagents.tracked.get(&id).cloned() {
                existing.optimistic = false;
                if source_agent_id.is_some() || existing.roster_source.is_none() {
                    existing.update_info(s);
                }
                if source_agent_id.is_some() {
                    existing.roster_source = source_agent_id.clone();
                }
                new_map.insert(id, existing);
            } else {
                let mut entry = TrackedSubagent::new(s);
                entry.roster_source = source_agent_id.clone();
                new_map.insert(id, entry);
            }
        }

        let leftover = std::mem::take(&mut self.subagents.tracked);
        let mut pending: Vec<String> = new_map
            .values()
            .filter_map(|t| t.info.parent_id.clone())
            .collect();
        while let Some(pid) = pending.pop() {
            if new_map.contains_key(&pid) {
                continue;
            }
            if let Some(entry) = leftover.get(&pid) {
                if let Some(grandparent) = entry.info.parent_id.clone() {
                    pending.push(grandparent);
                }
                new_map.insert(pid, entry.clone());
            }
        }

        let now = tokio::time::Instant::now();
        for (id, entry) in leftover {
            if new_map.contains_key(&id)
                || source_agent_id
                    .as_deref()
                    .is_some_and(|source| is_descendant_of(&id, source, &new_map))
            {
                continue;
            }
            if let Some(exited_at) = entry.exited_at {
                if now.saturating_duration_since(exited_at) < EXITED_SUBAGENT_GRACE {
                    new_map.entry(id).or_insert(entry);
                }
            } else if entry.optimistic
                && now.saturating_duration_since(entry.started_at) < OPTIMISTIC_SUBAGENT_GRACE
            {
                new_map.entry(id).or_insert(entry);
            }
        }
        self.subagents.tracked = new_map;
        self.clamp_panel_selection();
    }

    /// Advance the subagent spinner animation. Returns `true` if a re-render is
    /// needed (i.e. at least one agent is active). Driven by the spinner tick so
    /// running agents animate and their elapsed-time clocks stay current.
    pub(super) fn tick_subagent_animation(&mut self) -> bool {
        // Advance while ANY tracked child is active OR the selected sub-agent is
        // mid-turn on its own connect-on-select stream (#820): the selected
        // session's `running` flag is the source for its working spinner, and the
        // master's `subagent_local` status may still read idle for it.
        let any_active =
            self.active_subagent_running() || self.subagents.tracked_active_count() > 0;
        if !any_active {
            return false;
        }
        self.subagents.frame = self.subagents.frame.wrapping_add(1);
        // The sub-agent-first panel reads live state directly, so the only
        // consumer of `subagent_frame` is the "N working" activity line (#820
        // review). The frame bump above is all this tick needs.
        true
    }

    /// GC exited subagent bars whose grace period has elapsed (#540).
    /// Returns `true` if the bar was modified.
    pub(super) fn gc_exited_subagents(&mut self) -> bool {
        if self.subagents.tracked.is_empty() {
            return false;
        }
        gc_exited_subagents(
            &mut self.subagents.tracked,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
        )
    }
}
