//! Connection-collection helpers for ordinary exit (#1465): the exit persist
//! fan-out and child-watch collection. The single-connection collapse is #2044 PR 2.

use crate::shell::connection::TabId;

impl super::App {
    /// Explicit phase-3 ordinary-exit persist request fan-out: enqueue a current
    /// roster/session persist on every visible tab before any later teardown
    /// removes children. Phase 4 owns exit-path wiring and any ack/wait policy.
    pub fn enqueue_ordinary_exit_snapshot_persists(
        &mut self,
    ) -> Result<Vec<String>, (Vec<String>, crate::protocol::client::ClientError)> {
        let mut ids = Vec::new();
        let mut first_err = None;
        for tab in self.ordered_tab_ids() {
            let id = self.conn_for(tab).map(|c| c.namespaced_id("persist-exit"));
            if let (Some(conn), Some(id)) = (self.conn_for(tab), id) {
                if let Err(err) = conn.transport.clone_sender().try_send_exit_durability(
                    &crate::protocol::client::Command::PersistSession {
                        id: Some(id.clone()),
                        restore_reason: (self.exit_policy.kill_owned
                            && self
                                .tabs
                                .get(&tab)
                                .is_some_and(|state| state.child_exit_watch.is_some()))
                        .then(|| "ordinary_tui_exit_stopped".to_string()),
                    },
                ) {
                    first_err.get_or_insert(err);
                } else {
                    ids.push(id);
                }
            }
        }
        if let Some(err) = first_err {
            Err((ids, err))
        } else {
            Ok(ids)
        }
    }

    pub(crate) fn ordered_tab_ids(&self) -> Vec<TabId> {
        let mut ids: Vec<_> = self.tabs.keys().copied().collect();
        ids.sort_by_key(|t| t.0);
        ids
    }

    /// Detach every per-tab `ChildWatch` so ordinary-exit cleanup can terminate them (AC3c).
    /// Production exit goes through the roster-aware variant below; this
    /// roster-less form serves the headless harness and tests only.
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn take_all_child_exit_watches(
        &mut self,
    ) -> Vec<crate::shell::child_watch::ChildWatch> {
        self.take_all_child_exit_watches_with_rosters()
            .into_iter()
            .map(|(watch, _)| watch)
            .collect()
    }

    /// Like [`Self::take_all_child_exit_watches`], each watch paired with the
    /// number of subagents its tab's roster last showed (`None` for a spawn
    /// still in flight, which has no roster yet) — the input to the
    /// fleet-derived leader budget (#1956).
    pub(crate) fn take_all_child_exit_watches_with_rosters(
        &mut self,
    ) -> Vec<(crate::shell::child_watch::ChildWatch, Option<usize>)> {
        let mut out = Vec::new();
        for state in self.tabs.values_mut() {
            if let Some(w) = state.child_exit_watch.take() {
                out.push((w, Some(state.roster.tracked.len())));
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "tab_lifecycle_tests.rs"]
mod tab_lifecycle_tests;
