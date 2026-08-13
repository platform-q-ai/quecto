//! Multi-tab lifecycle helpers (#1465 P3): allocate/switch/close tabs and
//! snapshot registry/manifest durability.

use std::path::Path;

use super::connection_state::ConnectionState;
use crate::agents::view::SessionView;
use crate::components::footer::Footer;
use crate::components::notification::NotifyLevel;
use crate::shell::connection::{Connection, TabId};
use crate::shell::tab_registry::{TabAgentRecord, TabAgentRegistry, TabAgentStatus, unix_now_s};
use crate::shell::workspace_manifest::{
    WorkspaceManifest, WorkspaceManifestStore, WorkspaceTabEntry,
};

impl super::App {
    /// Stable iteration order: ascending `TabId` raw value.
    pub(crate) fn ordered_tab_ids(&self) -> Vec<TabId> {
        let mut ids: Vec<TabId> = self.tabs.keys().copied().collect();
        ids.sort_by_key(|t| t.0);
        ids
    }

    /// Next free tab id (max existing + 1).
    pub(crate) fn allocate_tab_id(&self) -> TabId {
        let next = self
            .tabs
            .keys()
            .map(|t| t.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        TabId(next)
    }

    /// Focus `tab` if present; clears tab-switch overlays. Returns false if unknown.
    pub(crate) fn switch_tab(&mut self, tab: TabId) -> bool {
        if !self.tabs.contains_key(&tab) {
            return false;
        }
        if self.active_tab == tab {
            return true;
        }
        self.active_tab = tab;
        self.close_tab_switch_overlays();
        true
    }

    /// Cycle focus forward (ascending ids, wrap).
    pub(crate) fn switch_tab_next(&mut self) -> TabId {
        let ids = self.ordered_tab_ids();
        if ids.is_empty() {
            return self.active_tab;
        }
        let pos = ids.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let next = ids[(pos + 1) % ids.len()];
        let _ = self.switch_tab(next);
        next
    }

    /// Cycle focus backward.
    pub(crate) fn switch_tab_prev(&mut self) -> TabId {
        let ids = self.ordered_tab_ids();
        if ids.is_empty() {
            return self.active_tab;
        }
        let pos = ids.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let prev = ids[(pos + ids.len() - 1) % ids.len()];
        let _ = self.switch_tab(prev);
        prev
    }

    /// Insert a connecting placeholder tab and focus it (AC1 partial).
    pub(crate) fn open_placeholder_tab(&mut self, name: Option<String>) -> TabId {
        let tab = self.allocate_tab_id();
        let transport = Connection::placeholder(tab);
        // Insert disconnected skeleton first so attach can upgrade the slot.
        let mut footer = Footer::new();
        footer.set_git_branch(self.workspace.git_branch.clone());
        let mut state = ConnectionState::new(
            Connection::placeholder(tab),
            SessionView::with_footer(footer),
        );
        state.agent_connected = false;
        state.name = name;
        self.tabs.insert(tab, state);
        // Re-enter via attach so the production path stays live for spawn-connect.
        self.attach_connection_to_tab(tab, transport, None);
        // Placeholder must remain not-connected until a live client is attached.
        if let Some(s) = self.tabs.get_mut(&tab) {
            s.agent_connected = false;
        }
        tab
    }

    /// Attach a live `Connection` into an existing placeholder (or new id).
    pub(crate) fn attach_connection_to_tab(
        &mut self,
        tab: TabId,
        connection: Connection,
        child_watch: Option<crate::shell::child_watch::ChildWatch>,
    ) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            #[cfg(any(test, feature = "test-harness"))]
            state.transport.abort_feed();
            state.transport = connection;
            state.agent_connected = true;
            state.child_exit_watch = child_watch;
        } else {
            let mut footer = Footer::new();
            footer.set_git_branch(self.workspace.git_branch.clone());
            let mut state = ConnectionState::new(connection, SessionView::with_footer(footer));
            state.agent_connected = true;
            state.child_exit_watch = child_watch;
            self.tabs.insert(tab, state);
        }
        let _ = self.switch_tab(tab);
        self.maybe_persist_default_durability();
    }

    /// Close a tab. Detaches by default (drops connection/watch without
    /// terminate). When `kill_agent` is true, returns that tab's ChildWatch for
    /// the caller to terminate (AC2/AC3). Refuses to close the last tab.
    pub(crate) fn close_tab(
        &mut self,
        tab: TabId,
        kill_agent: bool,
    ) -> Result<Option<crate::shell::child_watch::ChildWatch>, &'static str> {
        if self.tabs.len() <= 1 {
            return Err("cannot close the last tab");
        }
        if !self.tabs.contains_key(&tab) {
            return Err("unknown tab");
        }
        let mut state = self.tabs.remove(&tab).expect("checked");
        #[cfg(any(test, feature = "test-harness"))]
        state.transport.abort_feed();
        let watch = state.child_exit_watch.take();
        if self.active_tab == tab {
            let ids = self.ordered_tab_ids();
            let new_active = ids
                .iter()
                .copied()
                .rev()
                .find(|t| t.0 < tab.0)
                .unwrap_or(ids[0]);
            self.active_tab = new_active;
            self.close_tab_switch_overlays();
        }
        self.maybe_persist_default_durability();
        if kill_agent { Ok(watch) } else { Ok(None) }
    }

    /// Best-effort default-path durability write after lifecycle mutations.
    fn maybe_persist_default_durability(&mut self) {
        let workspace_id = "default";
        let registry_path = crate::shell::tab_registry::default_registry_path();
        let manifest_path = crate::shell::workspace_manifest::default_manifest_path();
        self.persist_durability_snapshot(workspace_id, &registry_path, &manifest_path);
    }

    /// Registry snapshot from current tabs.
    pub(crate) fn registry_snapshot(&self, workspace_id: Option<&str>) -> TabAgentRegistry {
        let mut reg = TabAgentRegistry::new();
        let now = unix_now_s();
        for tab in self.ordered_tab_ids() {
            let Some(state) = self.tabs.get(&tab) else {
                continue;
            };
            let status = if state.agent_connected {
                TabAgentStatus::Live
            } else {
                TabAgentStatus::Unknown
            };
            reg.upsert(TabAgentRecord {
                tab_id: tab.0,
                pid: None,
                socket_path: std::path::PathBuf::new(),
                session_key: None,
                tab_name: state.name.clone(),
                workspace_id: workspace_id.map(str::to_string),
                updated_unix_s: now,
                status,
            });
        }
        reg
    }

    /// Workspace manifest snapshot for durability (AC4/AC5 prep).
    pub(crate) fn workspace_manifest_snapshot(&self, workspace_id: &str) -> WorkspaceManifest {
        let tabs: Vec<WorkspaceTabEntry> = self
            .ordered_tab_ids()
            .into_iter()
            .map(|tab| {
                let name = self.tabs.get(&tab).and_then(|s| s.name.clone());
                WorkspaceTabEntry {
                    tab_id: tab.0,
                    session_key: None,
                    name,
                }
            })
            .collect();
        let active_index = tabs
            .iter()
            .position(|t| t.tab_id == self.active_tab.0)
            .unwrap_or(0);
        WorkspaceManifest {
            workspace_id: workspace_id.to_string(),
            active_index,
            tabs,
            updated_unix_s: unix_now_s(),
        }
    }

    /// Persist registry + workspace manifest (best-effort).
    pub(crate) fn persist_durability_snapshot(
        &mut self,
        workspace_id: &str,
        registry_path: &Path,
        manifest_path: &Path,
    ) {
        let reg = self.registry_snapshot(Some(workspace_id));
        if let Err(e) = reg.store(registry_path) {
            self.notify(
                &format!("failed to write tab registry: {e}"),
                NotifyLevel::Warning,
            );
        }
        let mut store = WorkspaceManifestStore::load(manifest_path);
        store.upsert(self.workspace_manifest_snapshot(workspace_id));
        if let Err(e) = store.store(manifest_path) {
            self.notify(
                &format!("failed to write workspace manifest: {e}"),
                NotifyLevel::Warning,
            );
        }
    }
}

#[cfg(test)]
#[path = "tab_lifecycle_tests.rs"]
mod tab_lifecycle_tests;
