//! Workspace-aware `/resume` (#1465 P4 / AC5–AC6).
//!
//! Workspaces are listed above bare sessions. Selecting a workspace restores
//! its tab set (placeholders + per-tab session resume where keys exist).
//! Bare session selection and `/resume <key>` keep current-tab behaviour.

use crate::components::notification::NotifyLevel;
use crate::components::select_list::{SelectItem, SelectList};
use crate::shell::connection::TabId;
use crate::shell::tab_registry::unix_now_s;
#[cfg(any(test, feature = "test-harness"))]
use crate::shell::workspace_manifest::WorkspaceTabEntry;
use crate::shell::workspace_manifest::{
    WorkspaceManifest, WorkspaceManifestStore, default_manifest_path,
};

/// Value prefix for workspace rows in the resume selector.
pub(crate) const WORKSPACE_RESUME_PREFIX: &str = "workspace:";
/// Value prefix for bare session rows (optional; bare keys still accepted).
pub(crate) const SESSION_RESUME_PREFIX: &str = "session:";

impl super::App {
    /// Open the session-only `/resume` selector.
    pub(super) fn open_resume_selector_with_sessions(
        &mut self,
        session_items: Vec<SelectItem>,
        empty_status: Option<&str>,
    ) {
        let items = session_items;
        if items.is_empty() {
            self.ac_mut().master_session.chat.add_entry(
                crate::components::chat::ChatEntry::Status {
                    text: empty_status
                        .unwrap_or("No persisted sessions found.")
                        .to_string(),
                },
            );
            return;
        }
        self.ac_mut().sessions.resume_selector = Some(SelectList::new(items, 12));
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn open_resume_selector_with_workspaces(
        &mut self,
        session_items: Vec<SelectItem>,
        _manifest_path: &std::path::Path,
        empty_status: Option<&str>,
    ) {
        self.open_resume_selector_with_sessions(session_items, empty_status);
    }

    /// Dispatch a resume-selector choice: workspace restore or current-tab session.
    pub(super) fn apply_resume_selection(&mut self, raw: &str) {
        if let Some(ws) = raw.strip_prefix(WORKSPACE_RESUME_PREFIX) {
            self.restore_workspace(ws.trim());
            return;
        }
        let session = raw
            .strip_prefix(SESSION_RESUME_PREFIX)
            .unwrap_or(raw)
            .trim();
        if session.is_empty() {
            self.send_list_sessions();
        } else {
            // Latch when the tab is still connecting / disconnected (AC5).
            self.queue_or_send_session_resume(session);
        }
    }

    /// Restore a workspace tab set from the durable manifest (AC5/AC6 foundation).
    ///
    /// - Ensures one tab slot per manifest entry (reuses MASTER for index 0).
    /// - Sets names; queues session resume on tabs that have keys (rehydrate path).
    /// - Per-tab failures notify without aborting the rest of the restore.
    pub(crate) fn restore_workspace(&mut self, workspace_id: &str) {
        self.restore_workspace_from_path(workspace_id, &default_manifest_path());
    }

    pub(crate) fn restore_workspace_from_path(
        &mut self,
        workspace_id: &str,
        manifest_path: &std::path::Path,
    ) {
        let store = WorkspaceManifestStore::load(manifest_path);
        let Some(manifest) = store.get(workspace_id).cloned() else {
            self.notify(
                &format!("Unknown workspace '{workspace_id}'"),
                NotifyLevel::Warning,
            );
            return;
        };
        self.apply_workspace_manifest(&manifest);
    }

    /// Apply an already-loaded workspace manifest to the tab collection.
    pub(crate) fn apply_workspace_manifest(&mut self, manifest: &WorkspaceManifest) {
        self.apply_workspace_manifest_with_registry(
            manifest,
            &crate::shell::tab_registry::default_registry_path(),
        );
    }

    /// Apply a workspace manifest, consulting `registry_path` for live reattach (AC6).
    pub(crate) fn apply_workspace_manifest_with_registry(
        &mut self,
        manifest: &WorkspaceManifest,
        registry_path: &std::path::Path,
    ) {
        if manifest.tabs.is_empty() {
            self.notify("Workspace has no tabs", NotifyLevel::Warning);
            return;
        }

        // Adopt the resumed workspace's identity: subsequent persists must
        // update the existing row instead of forking a duplicate under the
        // constructor-minted UUID (which would also strand the old row and
        // discard its label).
        self.workspace_id = manifest.workspace_id.clone();
        if !manifest.label.trim().is_empty() {
            self.workspace_label = manifest.label.clone();
        }

        // Ensure tab slots exist matching manifest order, recording the ACTUAL
        // tab id each entry occupies: stored ids can be stale (a previous
        // run's numbering), and entry 0 reuses the MASTER slot — the resume
        // plan below must target the real slots, not the stored ids, or the
        // first tab silently loses its session (#1466 fix pass item 4).
        let mut entry_tabs: Vec<TabId> = Vec::with_capacity(manifest.tabs.len());
        for (idx, entry) in manifest.tabs.iter().enumerate() {
            let tab = TabId(entry.tab_id);
            if idx == 0 && !self.tabs.contains_key(&tab) {
                // Fall back to MASTER if ids don't match stored id 0.
                let master = TabId::MASTER;
                if let Some(state) = self.tabs.get_mut(&master) {
                    state.name = entry.name.clone();
                }
                entry_tabs.push(master);
                continue;
            }
            entry_tabs.push(tab);
            if !self.tabs.contains_key(&tab) {
                let opened = self.open_placeholder_tab(entry.name.clone());
                // open_placeholder allocates next id; if it doesn't match desired,
                // rename map entry when possible.
                if opened != tab {
                    if let Some(mut state) = self.tabs.remove(&opened) {
                        // Keep transport tab id aligned with map key.
                        state.transport.set_tab(tab);
                        state.name = entry.name.clone();
                        self.tabs.insert(tab, state);
                        if self.active_tab == opened {
                            self.active_tab = tab;
                        }
                    }
                } else if let Some(state) = self.tabs.get_mut(&tab) {
                    state.name = entry.name.clone();
                }
            } else if let Some(state) = self.tabs.get_mut(&tab) {
                state.name = entry.name.clone();
            }
        }

        // Focus stored active tab when present (mapped, not stored, id).
        let active_tab_id = entry_tabs
            .get(manifest.active_index)
            .copied()
            .filter(|t| self.tabs.contains_key(t))
            .unwrap_or(TabId::MASTER);
        let _ = self.switch_tab(active_tab_id);

        // Queue per-tab session resumes without aborting on individual
        // failure — against the MAPPED slots (fix pass item 4). The STORED
        // tab id rides along: registry rows were persisted under the previous
        // run's numbering, so live-reattach lookups must use it.
        let resume_plan: Vec<(TabId, u32, Option<String>)> = manifest
            .tabs
            .iter()
            .zip(entry_tabs.iter())
            .map(|(e, tab)| (*tab, e.tab_id, e.session_key.clone()))
            .collect();
        let mut failures = 0usize;
        let mut resumed = 0usize;
        for (tab, stored_tab_id, key) in resume_plan {
            if !self.tabs.contains_key(&tab) {
                failures += 1;
                self.notify(
                    &format!("workspace restore: missing tab {}", tab.0),
                    NotifyLevel::Warning,
                );
                continue;
            }
            let _ = self.switch_tab(tab);
            match key {
                Some(session) if !session.trim().is_empty() => {
                    let live_socket =
                        live_registry_socket_for_tab(registry_path, stored_tab_id, &session);
                    if let Some(socket) = live_socket {
                        // AC6: prefer the live detached owner even when this TUI
                        // already spawned a fresh connected master. Resuming into
                        // the new agent would contend for the session lock.
                        self.ac_mut().socket_path = Some(socket);
                        self.ac_mut().session_key = Some(session.clone());
                        // Tier-1 live reattach reconnects to the already-running
                        // owner; do not send resume_session back into it after
                        // attach completes. Tier-2 stale/dead sockets still pass
                        // `session` to spawn_tab_agent_attach so fallback spawn
                        // resumes the persistent session.
                        self.ac_mut().pending_session_resume = None;
                        self.ac_mut().pending_attach = true;
                        self.ac_mut().agent_connected = false;
                        self.spawn_tab_agent_attach(tab, Some(session.clone()));
                        failures += 1; // counted as deferred until attach completes
                        self.ac_mut().master_session.chat.add_entry(
                            crate::components::chat::ChatEntry::Status {
                                text: format!(
                                    "Tab {}: reattaching live agent for '{session}'",
                                    tab.0
                                ),
                            },
                        );
                    } else if self.ac().agent_connected {
                        self.queue_or_send_session_resume(&session);
                        resumed += 1;
                    } else {
                        // AC6: retain deferred resume and kick a live attach/spawn so
                        // resume can retry once the tab connects (not a silent drop).
                        self.ac_mut().session_key = Some(session.clone());
                        self.ac_mut().pending_session_resume = Some(session.clone());
                        self.ac_mut().pending_attach = true;
                        self.spawn_tab_agent_attach(tab, Some(session.clone()));
                        failures += 1; // counted as deferred until attach completes
                        self.ac_mut().master_session.chat.add_entry(
                            crate::components::chat::ChatEntry::Status {
                                text: format!(
                                    "Tab {}: deferred resume of '{session}' (connecting…)",
                                    tab.0
                                ),
                            },
                        );
                    }
                }
                _ => {
                    // No session key: leave placeholder / live tab as-is.
                }
            }
        }

        // Restore focus to workspace active tab after per-tab switches.
        let _ = self.switch_tab(active_tab_id);
        self.notify(
            &format!(
                "Workspace '{}' restored ({} resumed, {} deferred/failed)",
                manifest.workspace_id, resumed, failures
            ),
            NotifyLevel::Info,
        );
        let _ = unix_now_s();
    }

    /// Queue a session resume for the active tab; apply immediately if connected.
    pub(crate) fn queue_or_send_session_resume(&mut self, session: &str) {
        let session = session.trim();
        if session.is_empty() {
            return;
        }
        if self.ac().agent_connected {
            self.ac_mut().pending_session_resume = None;
            self.ac_mut().session_key = Some(session.to_string());
            self.send_resume_session(session);
        } else {
            self.ac_mut().pending_session_resume = Some(session.to_string());
            self.ac_mut().session_key = Some(session.to_string());
        }
    }

    /// Apply deferred `/resume` once the active connection is live (AC6 / F6).
    ///
    /// Single send path: clears the pending latch before enqueue so attach
    /// startup and this helper cannot double-fire `resume_session`.
    pub(crate) fn try_apply_pending_session_resume(&mut self) {
        let Some(session) = self.ac_mut().pending_session_resume.take() else {
            return;
        };
        if !self.ac().agent_connected {
            self.ac_mut().pending_session_resume = Some(session);
            return;
        }
        self.ac_mut().session_key = Some(session.clone());
        self.send_resume_session(&session);
        self.persist_default_durability();
    }

    /// Test/helper: build a minimal in-memory workspace manifest.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn test_workspace_manifest(
        workspace_id: &str,
        tabs: Vec<WorkspaceTabEntry>,
        active_index: usize,
    ) -> WorkspaceManifest {
        WorkspaceManifest {
            workspace_id: workspace_id.into(),
            label: String::new(),
            last_active_unix_s: 0,
            active_index,
            tabs,
            updated_unix_s: unix_now_s(),
        }
    }
}

/// Live registry socket for the STORED `tab_id` / `session` if the process
/// still owns it. Registry rows carry the persisting run's tab numbering, so
/// callers pass the manifest's stored id, not the mapped local slot.
fn live_registry_socket_for_tab(
    registry_path: &std::path::Path,
    tab_id: u32,
    session: &str,
) -> Option<std::path::PathBuf> {
    let reg = crate::shell::tab_registry::TabAgentRegistry::load(registry_path);
    let rec = reg
        .agents
        .iter()
        .find(|a| a.tab_id == tab_id && a.session_key.as_deref() == Some(session))?;
    if rec.socket_path.as_os_str().is_empty() {
        return None;
    }
    if crate::shell::tab_registry::socket_path_is_live(&rec.socket_path) {
        Some(rec.socket_path.clone())
    } else {
        None
    }
}

#[cfg(test)]
#[path = "workspace_resume_tests.rs"]
mod workspace_resume_tests;
