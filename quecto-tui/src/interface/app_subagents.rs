use super::*;

impl App {
    pub(super) fn update_subagent_bar(
        &mut self,
        subagents: Vec<crate::infrastructure::client::SubagentInfoEvent>,
    ) {
        // Merge server data with existing local state to preserve exited_at
        // timestamps. New entries are inserted; entries absent from the
        // server push are removed unless they have an active grace period.
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
        // Preserve locally-tracked exited entries whose grace period
        // hasn't elapsed yet (server may stop reporting them immediately).
        let now = tokio::time::Instant::now();
        for (id, entry) in std::mem::take(&mut self.subagent_local) {
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
