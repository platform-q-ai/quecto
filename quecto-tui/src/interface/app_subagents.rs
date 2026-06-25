use super::*;

impl App {
    pub(super) fn update_subagent_bar(
        &mut self,
        subagents: Vec<crate::infrastructure::client::SubagentInfoEvent>,
    ) {
        // Snapshot prior statuses so we can detect active→idle transitions for a
        // one-shot inspector backfill once the merge below is applied (#797).
        let prev_statuses: std::collections::BTreeMap<String, String> = self
            .subagent_local
            .iter()
            .map(|(id, t)| (id.clone(), t.info.status.clone()))
            .collect();
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
        self.rebuild_subagent_bar();
        self.reconcile_inspector_on_idle(&prev_statuses);
    }

    /// When the inspector is open and its selected agent has just transitioned
    /// from active to a terminal/idle state, fire a single one-shot tail fetch
    /// to backfill the final turn (which the per-turn push path does not emit).
    /// Cheap drop-reconciliation; no recurring timer (#797).
    fn reconcile_inspector_on_idle(
        &mut self,
        prev_statuses: &std::collections::BTreeMap<String, String>,
    ) {
        if !self.inspector_open() {
            return;
        }
        let Some(selected) = self
            .subagent_inspector
            .as_ref()
            .and_then(|i| i.selected_agent_id())
        else {
            return;
        };
        let was_active = prev_statuses
            .get(&selected)
            .is_some_and(|s| subagent_status_is_active(s));
        let now_inactive = self
            .subagent_local
            .get(&selected)
            .is_some_and(|t| !subagent_status_is_active(&t.info.status));
        if was_active && now_inactive {
            self.request_inspector_tail(&selected);
        }
    }

    /// Rebuild the widget from local state.
    pub(super) fn rebuild_subagent_bar(&mut self) {
        if self.subagent_local.is_empty() {
            self.widgets_above.clear("subagents");
        } else {
            let now = tokio::time::Instant::now();
            let rows: Vec<SubagentRow> = self
                .subagent_local
                .values()
                .map(|t| SubagentRow::new(t.info.clone(), t.elapsed_secs(now)))
                .collect();
            let mut bar = SubagentBar::new();
            bar.update(rows, self.subagent_frame);
            bar.set_awaited(self.awaited_agent_id.clone());
            self.widgets_above.set("subagents", Box::new(bar));
        }
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
        self.rebuild_subagent_bar();
        true
    }

    /// GC exited subagent bars whose grace period has elapsed (#540).
    /// Returns `true` if the bar was modified.
    pub(super) fn gc_exited_subagents(&mut self) -> bool {
        if self.subagent_local.is_empty() {
            return false;
        }
        let removed = gc_exited_subagents(
            &mut self.subagent_local,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
        );
        if removed {
            self.rebuild_subagent_bar();
        }
        removed
    }
}
