//! Test/harness API for multi-tab connection collection (#1465).

use super::*;

#[cfg(any(test, feature = "test-harness"))]
impl App {
    /// Arm exact-pending attach correlation for a synthetic response delivery.
    pub fn test_arm_attach_backfill(&mut self, id: &str) {
        self.ac_mut().pending_attach_backfill_id = Some(id.to_string());
    }

    /// Arm exact-pending resume correlation for a synthetic response delivery.
    pub fn test_arm_resume_messages(&mut self, id: &str) {
        self.ac_mut().pending_resume_messages_id = Some(id.to_string());
    }

    /// Arm exact-pending rewind-refresh correlation for a synthetic response delivery.
    pub fn test_arm_rewind_refresh(&mut self, id: &str) {
        self.ac_mut().pending_rewind_refresh_id = Some(id.to_string());
    }

    /// Pending attach id (test inspection / capture after real mint).
    pub fn test_pending_attach_backfill_id(&self) -> Option<&str> {
        self.ac().pending_attach_backfill_id.as_deref()
    }

    /// Pending resume id (test inspection / capture after real mint).
    pub fn test_pending_resume_messages_id(&self) -> Option<&str> {
        self.ac().pending_resume_messages_id.as_deref()
    }

    /// Pending rewind-refresh id (test inspection / capture after real mint).
    pub fn test_pending_rewind_refresh_id(&self) -> Option<&str> {
        self.ac().pending_rewind_refresh_id.as_deref()
    }

    /// Re-key the master connection to another tab id, so tests can pin that
    /// minted-id namespaces derive from the connection's tab rather than a
    /// hard-coded `tab0:` literal (#1463 review).
    ///
    /// Also re-keys the tab map entry so `route_sourced` / `with_routing_tab`
    /// still address this slot (#1465).
    pub fn test_set_master_tab(&mut self, tab: u32) {
        let new_tab = crate::shell::connection::TabId(tab);
        let old = self.active_tab;
        if old == new_tab {
            self.ac_mut().transport.set_tab_for_tests(new_tab);
            return;
        }
        let Some(mut state) = self.tabs.remove(&old) else {
            panic!("active tab missing from map during test re-key");
        };
        state.transport.set_tab_for_tests(new_tab);
        self.tabs.insert(new_tab, state);
        self.active_tab = new_tab;
        if self.routing_tab_override == Some(old) {
            self.routing_tab_override = Some(new_tab);
        }
    }

    /// Insert a second (or Nth) disconnected tab for multi-tab isolation tests
    /// (#1465). Panics if `tab` is already present.
    pub fn test_insert_disconnected_tab(&mut self, tab: u32) {
        let tab = crate::shell::connection::TabId(tab);
        assert!(!self.tabs.contains_key(&tab), "tab {tab:?} already present");
        let mut transport = crate::shell::connection::Connection::disconnected_for_tests();
        transport.set_tab_for_tests(tab);
        let mut footer = crate::components::footer::Footer::new();
        footer.set_git_branch(self.workspace.git_branch.clone());
        let mut state = super::connection_state::ConnectionState::new(
            transport,
            crate::agents::view::SessionView::with_footer(footer),
        );
        // Disconnected stub: still treat as connected for event routing tests
        // unless a case explicitly tears it down.
        state.agent_connected = true;
        self.tabs.insert(tab, state);
    }

    /// Focus a different tab without tearing connections (#1465 test seam).
    pub fn test_set_active_tab(&mut self, tab: u32) {
        let tab = crate::shell::connection::TabId(tab);
        assert!(
            self.tabs.contains_key(&tab),
            "cannot activate missing tab {tab:?}"
        );
        self.active_tab = tab;
    }

    /// Snapshot lens for AC7 dual-state assertions (#1465).
    pub fn test_tab_ac7_snapshot(
        &self,
        tab: u32,
    ) -> (usize, bool, bool, u64, usize, Option<String>) {
        let tab = crate::shell::connection::TabId(tab);
        let c = self.conn_for(tab).expect("tab present for snapshot");
        (
            c.master_session.chat.entry_count(),
            c.agent_connected,
            c.disconnect_diag_pending,
            c.surfaced_oversized_drops,
            c.roster.tracked.len(),
            c.pending_resume_messages_id.clone(),
        )
    }

    /// Chat entry count for a specific tab.
    pub fn test_tab_chat_entry_count(&self, tab: u32) -> usize {
        self.conn_for(crate::shell::connection::TabId(tab))
            .expect("tab")
            .master_session
            .chat
            .entry_count()
    }

    /// Whether a tab's chat text contains `needle` (any entry Display).
    pub fn test_tab_chat_contains(&self, tab: u32, needle: &str) -> bool {
        let c = self
            .conn_for(crate::shell::connection::TabId(tab))
            .expect("tab");
        c.master_session
            .chat
            .entries()
            .iter()
            .any(|e| format!("{e:?}").contains(needle) || e_text_contains(e, needle))
    }
}

#[cfg(any(test, feature = "test-harness"))]
fn e_text_contains(entry: &crate::components::chat::ChatEntry, needle: &str) -> bool {
    match entry {
        crate::components::chat::ChatEntry::Assistant { text, .. }
        | crate::components::chat::ChatEntry::User { text, .. }
        | crate::components::chat::ChatEntry::Stub { text, .. }
        | crate::components::chat::ChatEntry::Status { text, .. } => text.contains(needle),
        crate::components::chat::ChatEntry::ToolExecution { args, result, .. } => {
            args.contains(needle) || result.as_ref().is_some_and(|r| r.contains(needle))
        }
    }
}
