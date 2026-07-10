use super::*;

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
            Event::TurnEnd { message, .. } => {
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
        // Merge server data with existing local state to preserve exited_at
        // timestamps. New entries are inserted; entries absent from the
        // server push are removed unless they have an active grace period OR
        // they are an ancestor of a surviving entry (see below).
        let mut new_map = std::collections::BTreeMap::new();
        for s in subagents {
            let id = sanitize_agent_id(&s.agent_id);
            if let Some(mut existing) = self.subagents.tracked.remove(&id) {
                // The kernel reported this agent — it is now confirmed, so it is
                // no longer an unconfirmed local guess and reverts to normal
                // full-replace reconciliation thereafter (#866).
                existing.optimistic = false;
                existing.update_info(s);
                new_map.insert(id, existing);
            } else {
                new_map.insert(id, TrackedSubagent::new(s));
            }
        }
        // The remaining (un-pushed) entries from the previous roster. Kept by
        // reference so we can both grace-preserve exited ones AND pull in any
        // that are still referenced as ancestors of a surviving entry.
        let leftover = std::mem::take(&mut self.subagents.tracked);

        // Ancestor preservation (grandchild-nesting bug): a `subagent_state_changed`
        // is treated as a FULL replace, but a forwarded child's-eye-view push
        // lists only a sub-tree (a child's OWN children) and omits the
        // intermediate parent itself. A naive full-replace would then EVICT that
        // parent, so `subagent_tree_order` can no longer resolve a grandchild's
        // `parent_id` in the map and re-roots it to depth 1 (sometimes ABOVE its
        // parent). Walk every surviving entry's parent chain and carry over any
        // ancestor the push omitted, so intermediate parents are never dropped
        // and nesting depth is preserved regardless of which source pushed.
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

        // Preserve locally-tracked exited entries whose grace period hasn't
        // elapsed yet (server may stop reporting them immediately), AND
        // unconfirmed optimistic entries within their own grace window: a child
        // can be omitted from a snapshot taken before the kernel registered it,
        // and dropping it would make a long-first-turn agent invisible until it
        // finishes (#866). The grace bounds a never-confirmed spawn (e.g. a
        // failed launch) so it can't linger forever.
        let now = tokio::time::Instant::now();
        for (id, entry) in leftover {
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
        // Keep the panel cursor in bounds after the list changes (#800).
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
