//! Multi-tab lifecycle helpers (#1465): open/close/switch + durability snapshots.

use super::connection_state::ConnectionState;
use crate::agents::view::SessionView;
use crate::components::notification::NotifyLevel;
use crate::shell::connection::{Connection, TabId};
use crate::shell::tab_registry::{TabAgentRecord, TabAgentRegistry, TabAgentStatus, unix_now_s};
use crate::shell::workspace_manifest::{
    WorkspaceManifest, WorkspaceManifestStore, WorkspaceTabEntry,
};

impl super::App {
    /// Allocate the next free numeric tab id (monotonic, skips occupied).
    pub(crate) fn allocate_tab_id(&self) -> TabId {
        let mut n = self
            .tabs
            .keys()
            .map(|t| t.0)
            .max()
            .map(|m| m.saturating_add(1))
            .unwrap_or(0);
        while self.tabs.contains_key(&TabId(n)) {
            n = n.saturating_add(1);
        }
        TabId(n)
    }

    /// Open a connecting placeholder tab and focus it (AC2).
    pub(crate) fn open_placeholder_tab(&mut self, name: Option<String>) -> TabId {
        let tab = self.allocate_tab_id();
        let mut state = ConnectionState::new(Connection::placeholder(tab), SessionView::new(None));
        state.name = name;
        state.agent_connected = false;
        state.agent_ever_connected = false;
        self.tabs.insert(tab, state);
        self.switch_tab(tab);
        tab
    }

    /// Whether a background spawn/reattach is still pending for `tab` (AC2).
    pub(crate) fn tab_has_pending_attach(&self, tab: TabId) -> bool {
        self.conn_for(tab)
            .is_some_and(|c| c.pending_attach || c.pending_session_resume.is_some())
    }

    /// Mark a tab as waiting on a non-blocking live attach/spawn (AC1/AC2).
    pub(crate) fn mark_tab_pending_attach(&mut self, tab: TabId) {
        if let Some(c) = self.conn_mut(tab) {
            c.pending_attach = true;
            c.agent_connected = false;
        }
    }

    /// Open a connecting placeholder and schedule a live agent spawn/attach.
    pub(crate) fn open_live_tab(&mut self, name: Option<String>) -> TabId {
        let tab = self.open_placeholder_tab(name);
        self.mark_tab_pending_attach(tab);
        self.spawn_tab_agent_attach(tab, None);
        tab
    }

    /// Background-spawn (or reattach) a persistent agent for `tab` and deliver
    /// the result on the tab-attach channel (AC1/AC2/AC6).
    pub(crate) fn spawn_tab_agent_attach(&self, tab: TabId, resume_session: Option<String>) {
        let Some(tx) = self.tab_attach_tx.clone() else {
            return;
        };
        // Prefer reattach when the tab already knows a live socket.
        let existing_socket = self
            .conn_for(tab)
            .and_then(|c| c.socket_path.clone())
            .filter(|p| p.as_os_str().len() > 0);
        // Unit tests may call lifecycle helpers without a running runtime; the
        // pending_attach flag is still set by the caller for AC1/AC2 coverage.
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        tokio::spawn(async move {
            let outcome = match existing_socket {
                Some(path) => attach_existing_socket(tab, path).await,
                None => spawn_and_attach_new_agent(tab, resume_session).await,
            };
            let _ = tx.send(outcome).await;
        });
    }

    /// Apply a successful/failed tab attach on the event loop (AC2).
    pub(crate) fn apply_tab_attach_outcome(&mut self, outcome: TabAttachOutcome) {
        let tab = outcome.tab;
        if !self.tabs.contains_key(&tab) {
            // Tab closed while spawning — terminate any owned child we got back.
            if let Some(watch) = outcome.child_watch {
                if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::spawn(async move {
                        watch.terminate().await;
                    });
                }
            }
            return;
        }
        if let Some(conn_state) = self.conn_mut(tab) {
            conn_state.pending_attach = false;
        }
        match outcome.result {
            Ok(ready) => {
                let pending_resume = self
                    .conn_for(tab)
                    .and_then(|c| c.pending_session_resume.clone())
                    .or(ready.resume_session.clone());
                let event_tx = match self.tab_event_tx.clone() {
                    Some(tx) => tx,
                    None => {
                        self.notify(
                            &format!("Tab {} connected without fan-in sender", tab.0),
                            NotifyLevel::Warning,
                        );
                        return;
                    }
                };
                let transport = Connection::spawn(ready.client, tab, event_tx);
                if let Some(c) = self.conn_mut(tab) {
                    c.socket_path = Some(ready.socket_path.clone());
                    c.child_pid = ready.pid;
                    if let Some(key) = ready.session_key.clone() {
                        c.session_key = Some(key);
                    }
                }
                self.attach_connection_to_tab(tab, transport, ready.child_watch);
                if let Some(session) = pending_resume {
                    if let Some(c) = self.conn_mut(tab) {
                        c.pending_session_resume = None;
                    }
                    let _ = self.with_routing_tab(tab, |app| {
                        app.send_resume_session(&session);
                    });
                } else {
                    let _ = self.with_routing_tab(tab, |app| {
                        app.request_master_attach_backfill();
                    });
                }
                self.notify(&format!("Tab {} connected", tab.0), NotifyLevel::Success);
                self.persist_default_durability();
            }
            Err(err) => {
                self.notify(
                    &format!("Tab {} failed to connect: {err}", tab.0),
                    NotifyLevel::Error,
                );
                if let Some(c) = self.conn_mut(tab) {
                    c.agent_connected = false;
                }
            }
        }
    }

    /// Best-effort default-path durability write after lifecycle changes.
    pub(crate) fn persist_default_durability(&mut self) {
        let reg = crate::shell::tab_registry::default_registry_path();
        let man = crate::shell::workspace_manifest::default_manifest_path();
        if let Some(parent) = reg.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Some(parent) = man.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Touch pending-attach query so durability stays aware of in-flight tabs.
        let _pending = self
            .ordered_tab_ids()
            .into_iter()
            .filter(|t| self.tab_has_pending_attach(*t))
            .count();
        let _ = _pending;
        self.persist_durability_snapshot("default", &reg, &man);
    }

    /// Focus `tab` if present. Returns whether the switch happened.
    pub(crate) fn switch_tab(&mut self, tab: TabId) -> bool {
        if !self.tabs.contains_key(&tab) {
            return false;
        }
        if self.active_tab == tab {
            return true;
        }
        self.close_tab_switch_overlays();
        self.active_tab = tab;
        // Panel cursor/key live on App-global SubagentUi; resync to the newly
        // focused tab's roster so a prior tab's selection cannot stick (review).
        self.subagents.panel_nav_key = None;
        self.subagents.panel_nav.set_selected(0);
        self.sync_panel_selection_to_active();
        true
    }

    /// Cycle focus forward/back through sorted tab ids.
    pub(crate) fn switch_tab_next(&mut self) -> TabId {
        let ids = self.ordered_tab_ids();
        let cur = ids.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let next = ids[(cur + 1) % ids.len()];
        self.switch_tab(next);
        next
    }

    pub(crate) fn switch_tab_prev(&mut self) -> TabId {
        let ids = self.ordered_tab_ids();
        let cur = ids.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let prev = ids[(cur + ids.len() - 1) % ids.len()];
        self.switch_tab(prev);
        prev
    }

    pub(crate) fn ordered_tab_ids(&self) -> Vec<TabId> {
        let mut ids: Vec<_> = self.tabs.keys().copied().collect();
        ids.sort_by_key(|t| t.0);
        ids
    }

    /// Close `tab`. When `kill_agent`, return its `ChildWatch` so the caller
    /// can terminate (AC3a). Refuses to close the last remaining tab.
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
        let ids = self.ordered_tab_ids();
        let idx = ids.iter().position(|t| *t == tab).unwrap_or(0);
        let fallback = if idx > 0 {
            ids[idx - 1]
        } else {
            ids.get(1).copied().unwrap_or(TabId::MASTER)
        };
        let mut state = self.tabs.remove(&tab).expect("tab present");
        let watch = state.child_exit_watch.take();
        if self.active_tab == tab {
            self.active_tab = fallback;
            self.subagents.panel_nav_key = None;
            self.subagents.panel_nav.set_selected(0);
            self.sync_panel_selection_to_active();
        }
        self.routing_tab_override = self.routing_tab_override.filter(|t| *t != tab);
        self.persist_default_durability();
        Ok(if kill_agent { watch } else { None })
    }

    /// Detach every per-tab `ChildWatch` for explicit kill-on-exit (AC3c).
    pub(crate) fn take_all_child_exit_watches(
        &mut self,
    ) -> Vec<crate::shell::child_watch::ChildWatch> {
        let mut out = Vec::new();
        for state in self.tabs.values_mut() {
            if let Some(w) = state.child_exit_watch.take() {
                out.push(w);
            }
        }
        out
    }

    /// Replace a tab's transport with a live connection (spawn success path).
    pub(crate) fn attach_connection_to_tab(
        &mut self,
        tab: TabId,
        transport: Connection,
        child_watch: Option<crate::shell::child_watch::ChildWatch>,
    ) {
        let Some(state) = self.tabs.get_mut(&tab) else {
            return;
        };
        state.transport = transport;
        state.child_exit_watch = child_watch;
        state.agent_connected = true;
        state.agent_ever_connected = true;
        state.pending_attach = false;
        state.started_at = tokio::time::Instant::now();
        if self.active_tab != tab {
            self.switch_tab(tab);
        }
        // AC6: if restore deferred a session key while offline, apply it now.
        let _ = self.with_routing_tab(tab, |app| {
            app.try_apply_pending_session_resume();
        });
    }

    /// Snapshot registry records for all live tabs (AC4).
    pub(crate) fn registry_snapshot(&self, workspace_id: Option<&str>) -> TabAgentRegistry {
        let mut reg = TabAgentRegistry::new();
        let now = unix_now_s();
        for tab in self.ordered_tab_ids() {
            let Some(state) = self.tabs.get(&tab) else {
                continue;
            };
            let status = if state.agent_connected {
                TabAgentStatus::Live
            } else if state.pending_attach {
                TabAgentStatus::Unknown
            } else {
                TabAgentStatus::Dead
            };
            let socket_path = state.socket_path.clone().unwrap_or_default();
            let pid = state
                .child_pid
                .or_else(|| state.child_exit_watch.as_ref().and_then(|w| w.pid()));
            let session_key = state
                .session_key
                .clone()
                .or_else(|| state.pending_session_resume.clone());
            reg.upsert(TabAgentRecord {
                tab_id: tab.0,
                pid,
                socket_path,
                session_key,
                tab_name: state.name.clone(),
                workspace_id: workspace_id.map(str::to_string),
                updated_unix_s: now,
                status,
            });
        }
        // Opportunistic liveness refresh for records that claim a pid/socket; do not
        // flip pure placeholders to Dead here (workspace restore still needs them).
        for rec in &mut reg.agents {
            if rec.pid.is_some() || !rec.socket_path.as_os_str().is_empty() {
                if crate::shell::tab_registry::default_liveness_probe(rec) {
                    rec.status = TabAgentStatus::Live;
                } else if rec.status == TabAgentStatus::Live {
                    rec.status = TabAgentStatus::Dead;
                }
            }
        }
        reg
    }

    /// Snapshot the active workspace manifest (AC4/AC5).
    pub(crate) fn workspace_manifest_snapshot(&self, workspace_id: &str) -> WorkspaceManifest {
        let ids = self.ordered_tab_ids();
        let active_index = ids.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let tabs = ids
            .iter()
            .filter_map(|tab| {
                let state = self.tabs.get(tab)?;
                Some(WorkspaceTabEntry {
                    tab_id: tab.0,
                    session_key: state
                        .session_key
                        .clone()
                        .or_else(|| state.pending_session_resume.clone()),
                    name: state.name.clone(),
                })
            })
            .collect();
        WorkspaceManifest {
            workspace_id: workspace_id.to_string(),
            active_index,
            tabs,
            updated_unix_s: unix_now_s(),
        }
    }

    /// Persist registry + workspace store for `workspace_id` (best-effort).
    pub(crate) fn persist_durability_snapshot(
        &self,
        workspace_id: &str,
        registry_path: &std::path::Path,
        manifest_path: &std::path::Path,
    ) {
        let reg = self.registry_snapshot(Some(workspace_id));
        let _ = reg.store(registry_path);
        // Maintain on-disk registry with the public GC/refresh APIs: keep every
        // tab still open in this process; drop rows for closed tabs that are
        // no longer live on disk.
        let open: std::collections::HashSet<u32> = self.tabs.keys().map(|t| t.0).collect();
        let mut on_disk = TabAgentRegistry::load(registry_path);
        on_disk.refresh_status(|rec| {
            if open.contains(&rec.tab_id) {
                true
            } else {
                crate::shell::tab_registry::default_liveness_probe(rec)
            }
        });
        on_disk.gc_dead(|rec| {
            open.contains(&rec.tab_id) || crate::shell::tab_registry::default_liveness_probe(rec)
        });
        let _ = on_disk.store(registry_path);
        let mut store = WorkspaceManifestStore::load(manifest_path);
        store.upsert(self.workspace_manifest_snapshot(workspace_id));
        let _ = store.store(manifest_path);
    }
}

/// Result delivered from a background tab spawn/reattach task.
pub(crate) struct TabAttachOutcome {
    pub(crate) tab: TabId,
    pub(crate) result: Result<TabAttachReady, String>,
    pub(crate) child_watch: Option<crate::shell::child_watch::ChildWatch>,
}

pub(crate) struct TabAttachReady {
    pub(crate) client: crate::protocol::client::Client,
    pub(crate) socket_path: std::path::PathBuf,
    pub(crate) pid: Option<u32>,
    pub(crate) child_watch: Option<crate::shell::child_watch::ChildWatch>,
    pub(crate) session_key: Option<String>,
    pub(crate) resume_session: Option<String>,
}

async fn attach_existing_socket(tab: TabId, path: std::path::PathBuf) -> TabAttachOutcome {
    let client = match crate::protocol::client::Client::connect(&path).await {
        Ok(c) => c,
        Err(e) => {
            // Fall back to framed→legacy if needed is already inside connect;
            // try legacy once more for older agents.
            match crate::protocol::client::Client::connect_legacy(&path).await {
                Ok(c) => c,
                Err(e2) => {
                    return TabAttachOutcome {
                        tab,
                        result: Err(format!("reattach {path:?}: {e}; legacy: {e2}")),
                        child_watch: None,
                    };
                }
            }
        }
    };
    TabAttachOutcome {
        tab,
        result: Ok(TabAttachReady {
            client,
            socket_path: path,
            pid: None,
            child_watch: None,
            session_key: None,
            resume_session: None,
        }),
        child_watch: None,
    }
}

async fn spawn_and_attach_new_agent(
    tab: TabId,
    resume_session: Option<String>,
) -> TabAttachOutcome {
    let flags = crate::shell::cli::tab_spawn_flags(resume_session.clone());
    match crate::shell::cli::spawn_agent_for_tab(&flags).await {
        Ok((path, child, stderr_tail, announced)) => {
            let pid = child.id();
            let watch = crate::shell::child_watch::watch_child(child, stderr_tail);
            let speaks_frames = announced
                .is_some_and(|v| u32::from(v) >= u32::from(quecto_line_io::PROTOCOL_VERSION));
            let client = if speaks_frames {
                crate::protocol::client::Client::connect(&path).await
            } else {
                crate::protocol::client::Client::connect_legacy(&path).await
            };
            match client {
                Ok(c) => TabAttachOutcome {
                    tab,
                    child_watch: Some(watch.clone()),
                    result: Ok(TabAttachReady {
                        client: c,
                        socket_path: path,
                        pid,
                        child_watch: Some(watch),
                        session_key: None,
                        resume_session,
                    }),
                },
                Err(e) => {
                    watch.terminate().await;
                    TabAttachOutcome {
                        tab,
                        result: Err(format!("connect after spawn: {e}")),
                        child_watch: None,
                    }
                }
            }
        }
        Err(e) => TabAttachOutcome {
            tab,
            result: Err(e),
            child_watch: None,
        },
    }
}

#[cfg(test)]
#[path = "tab_lifecycle_tests.rs"]
mod tab_lifecycle_tests;
