use super::*;

impl App {
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
            if let Some(mut existing) = self.subagent_local.remove(&id) {
                existing.update_info(s);
                new_map.insert(id, existing);
            } else {
                new_map.insert(id, TrackedSubagent::new(s));
            }
        }
        // The remaining (un-pushed) entries from the previous roster. Kept by
        // reference so we can both grace-preserve exited ones AND pull in any
        // that are still referenced as ancestors of a surviving entry.
        let leftover = std::mem::take(&mut self.subagent_local);

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

        // Preserve locally-tracked exited entries whose grace period
        // hasn't elapsed yet (server may stop reporting them immediately).
        let now = tokio::time::Instant::now();
        for (id, entry) in leftover {
            if let Some(exited_at) = entry.exited_at {
                if now.saturating_duration_since(exited_at) < EXITED_SUBAGENT_GRACE {
                    new_map.entry(id).or_insert(entry);
                }
            }
        }
        self.subagent_local = new_map;
        // Keep the panel cursor in bounds after the list changes (#800).
        self.clamp_panel_selection();
    }

    /// Advance the subagent spinner animation. Returns `true` if a re-render is
    /// needed (i.e. at least one agent is active). Driven by the spinner tick so
    /// running agents animate and their elapsed-time clocks stay current.
    pub(super) fn tick_subagent_animation(&mut self) -> bool {
        let any_active = self
            .subagent_local
            .values()
            .any(|t| subagent_status_is_active(&t.info.status));
        if !any_active {
            return false;
        }
        self.subagent_frame = self.subagent_frame.wrapping_add(1);
        // The sub-agent-first panel reads live state directly, so the only
        // consumer of `subagent_frame` is the "N working" activity line (#820
        // review). The frame bump above is all this tick needs.
        true
    }

    /// GC exited subagent bars whose grace period has elapsed (#540).
    /// Returns `true` if the bar was modified.
    pub(super) fn gc_exited_subagents(&mut self) -> bool {
        if self.subagent_local.is_empty() {
            return false;
        }
        gc_exited_subagents(
            &mut self.subagent_local,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
        )
    }
}
