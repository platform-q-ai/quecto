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
    ///
    /// Ids with an in-flight attach are treated as occupied even after the
    /// map entry is gone only via generation rejection; here we still skip
    /// currently present keys (#1465 F2).
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

    /// Mint a new attach generation for the next spawn/reattach of `tab`.
    pub(crate) fn bump_attach_generation(&mut self, tab: TabId) -> u64 {
        let generation = self.next_attach_generation;
        self.next_attach_generation = self.next_attach_generation.saturating_add(1);
        if let Some(c) = self.conn_mut(tab) {
            c.attach_generation = generation;
        }
        generation
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
    ///
    /// A leftover `pending_session_resume` latch alone is not "still connecting":
    /// treating it that way queued prompts forever after a failed attach.
    pub(crate) fn tab_has_pending_attach(&self, tab: TabId) -> bool {
        self.conn_for(tab).is_some_and(|c| c.pending_attach)
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
    pub(crate) fn spawn_tab_agent_attach(&mut self, tab: TabId, resume_session: Option<String>) {
        let Some(tx) = self.tab_attach_tx.clone() else {
            return;
        };
        let generation = self.bump_attach_generation(tab);
        // Prefer reattach only when the tab already knows a *live* socket.
        // Stale registry paths must fall through to spawn+resume (AC6 / F5).
        let existing_socket = self
            .conn_for(tab)
            .and_then(|c| c.socket_path.clone())
            .filter(|p| crate::shell::tab_registry::socket_path_is_live(p));
        let policy = self.tab_spawn_policy.clone().unwrap_or_default();
        // Unit tests may call lifecycle helpers without a running runtime; the
        // pending_attach flag is still set by the caller for AC1/AC2 coverage.
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        tokio::spawn(async move {
            let mut outcome = match existing_socket {
                Some(path) => {
                    let out = attach_existing_socket(tab, path).await;
                    // Dead/stale socket: Tier-2 spawn + resume_session (AC6).
                    if out.result.is_err() {
                        spawn_and_attach_new_agent(tab, resume_session, policy).await
                    } else {
                        out
                    }
                }
                None => spawn_and_attach_new_agent(tab, resume_session, policy).await,
            };
            outcome.generation = generation;
            let _ = tx.send(outcome).await;
        });
    }

    /// Apply a successful/failed tab attach on the event loop (AC2).
    pub(crate) fn apply_tab_attach_outcome(&mut self, outcome: TabAttachOutcome) {
        let tab = outcome.tab;
        let stale = match self.conn_for(tab) {
            None => true,
            Some(c) => c.attach_generation != outcome.generation,
        };
        if stale {
            // Tab closed or recycled while spawning — terminate any owned child.
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
                // Keep deferred resume on the tab; attach_connection_to_tab
                // applies it once via try_apply_pending_session_resume (F6).
                if let Some(session) = ready.resume_session.clone() {
                    if let Some(c) = self.conn_mut(tab) {
                        if c.pending_session_resume.is_none() {
                            c.pending_session_resume = Some(session);
                        }
                    }
                }
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
                // Per-tab startup (GetState + roster + history), not master-only (F4).
                let _ = self.with_routing_tab(tab, |app| {
                    app.send_startup_requests();
                });
                // Flush prompts queued while connecting (F7) via submit helper.
                let _ = self.with_routing_tab(tab, |app| {
                    app.flush_queued_prompts();
                });
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
                    // Final failure: drop the resume latch so later prompts are
                    // not treated as still-connecting (#1481 review).
                    c.pending_session_resume = None;
                    c.queued_prompts.clear();
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
        // #1466 decision 1: durability is keyed by this TUI's UUID identity,
        // never a shared literal, so two TUIs can never clobber each other.
        let workspace_id = self.workspace_id.clone();
        self.persist_durability_snapshot(&workspace_id, &reg, &man);
    }

    /// Focus `tab` if present. Returns whether the switch happened.
    pub(crate) fn switch_tab(&mut self, tab: TabId) -> bool {
        if !self.tabs.contains_key(&tab) {
            return false;
        }
        if self.active_tab == tab {
            return true;
        }
        // Park the active editor draft on the leaving tab; restore the target's (F11).
        let leaving = self.active_tab;
        let draft = self.editor.text();
        if let Some(c) = self.conn_mut(leaving) {
            c.editor_draft = draft;
        }
        self.close_tab_switch_overlays();
        self.active_tab = tab;
        // Viewing the tab consumes its unread dot (#1466 decision 4).
        if let Some(c) = self.conn_mut(tab) {
            c.unread_output = false;
        }
        let restore = self
            .conn_for(tab)
            .map(|c| c.editor_draft.clone())
            .unwrap_or_default();
        self.editor.set_text(&restore);
        // Panel cursor/key live on App-global SubagentUi; resync to the newly
        // focused tab's roster so a prior tab's selection cannot stick (review).
        self.subagents.panel_nav_key = None;
        self.subagents.panel_nav.set_selected(0);
        self.sync_panel_selection_to_active();
        true
    }

    /// Focus the `ordinal`-th tab (1-based, tab-bar order). Ordinals past the
    /// open tab count no-op (#1466 decision 5).
    pub(crate) fn focus_tab_ordinal(&mut self, ordinal: usize) -> bool {
        let Some(tab) = self
            .ordered_tab_ids()
            .get(ordinal.saturating_sub(1))
            .copied()
        else {
            return false;
        };
        self.switch_tab(tab)
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
        // Invalidate in-flight attach outcomes before remove so a recycled id
        // cannot accept a stale spawn (F2). Feed abort happens via Connection Drop (F1).
        if let Some(c) = self.conn_mut(tab) {
            c.attach_generation = 0;
            c.pending_attach = false;
            c.transport.abort_feed();
        }
        let mut state = self.tabs.remove(&tab).expect("tab present");
        let watch = state.child_exit_watch.take();
        if self.active_tab == tab {
            // Restore fallback draft without parking closed-tab text onto it.
            self.editor.set_text(
                &self
                    .conn_for(fallback)
                    .map(|c| c.editor_draft.clone())
                    .unwrap_or_default(),
            );
            self.active_tab = fallback;
            self.subagents.panel_nav_key = None;
            self.subagents.panel_nav.set_selected(0);
            self.sync_panel_selection_to_active();
            self.inference.model_registry.open_pending = false;
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
        // Abort prior feed before overwrite (Drop also aborts; explicit is clearer).
        state.transport.abort_feed();
        state.transport = transport;
        state.child_exit_watch = child_watch;
        state.agent_connected = true;
        state.agent_ever_connected = true;
        state.pending_attach = false;
        state.started_at = tokio::time::Instant::now();
        // Do not steal focus if the user already navigated away (F9).
        // AC6: if restore deferred a session key while offline, apply it once.
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
                    // #1466 fix pass item 3: persist the tab's last user
                    // message so `/resume` rows are recognizable by content.
                    summary: state
                        .master_session
                        .chat
                        .last_user_text()
                        .map(|t| snippet_of(t, TAB_SUMMARY_MAX_CHARS)),
                })
            })
            .collect();
        WorkspaceManifest {
            workspace_id: workspace_id.to_string(),
            // Own workspace: stamp our auto-generated label (#1466 decision 1).
            // Foreign ids leave it empty; persist preserves the stored label.
            label: if workspace_id == self.workspace_id {
                self.workspace_label.clone()
            } else {
                String::new()
            },
            last_active_unix_s: unix_now_s(),
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
        // Merge the open-tab snapshot into the on-disk document. A full replace
        // would drop detached-but-live rows (AC3b / AC6): later load/refresh/gc
        // and workspace restore would never see them.
        let open_snapshot = self.registry_snapshot(Some(workspace_id));
        let mut on_disk = TabAgentRegistry::load(registry_path);
        for rec in open_snapshot.agents {
            on_disk.upsert(rec);
        }
        let open: std::collections::HashSet<u32> = self.tabs.keys().map(|t| t.0).collect();
        // A row is "ours" only when both the tab id is open AND the row's
        // workspace matches: matching on tab_id alone would flip dead foreign
        // rows Live (tab 0 is open in every TUI) and make them immortal,
        // which in turn pins their session-less manifests past gc_orphaned.
        let is_open_own = |rec: &crate::shell::tab_registry::TabAgentRecord| {
            open.contains(&rec.tab_id) && rec.workspace_id.as_deref() == Some(workspace_id)
        };
        on_disk.refresh_status(|rec| {
            if is_open_own(rec) {
                true
            } else {
                crate::shell::tab_registry::default_liveness_probe(rec)
            }
        });
        on_disk.gc_dead(|rec| {
            is_open_own(rec) || crate::shell::tab_registry::default_liveness_probe(rec)
        });
        let _ = on_disk.store(registry_path);
        let mut store = WorkspaceManifestStore::load(manifest_path);
        let mut manifest = self.workspace_manifest_snapshot(workspace_id);
        // Preserve a stored (possibly renamed) label the snapshot doesn't know.
        if manifest.label.trim().is_empty() {
            if let Some(prev) = store.get(workspace_id) {
                manifest.label = prev.label.clone();
            }
        }
        store.upsert(manifest);
        // Orphaned-workspace GC (#1466): rows with no resumable session key
        // and no registry record are dead weight in `/resume` — drop them.
        let _ = store.gc_orphaned(&on_disk);
        let _ = store.store(manifest_path);
    }
}

/// Result delivered from a background tab spawn/reattach task.
pub(crate) struct TabAttachOutcome {
    pub(crate) tab: TabId,
    /// Must match the tab's `attach_generation` or the outcome is rejected (F2).
    pub(crate) generation: u64,
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
                        generation: 0,
                        result: Err(format!("reattach {path:?}: {e}; legacy: {e2}")),
                        child_watch: None,
                    };
                }
            }
        }
    };
    TabAttachOutcome {
        tab,
        generation: 0,
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
    policy: crate::shell::cli::TabSpawnPolicy,
) -> TabAttachOutcome {
    let flags = crate::shell::cli::tab_spawn_flags_from_policy(&policy, resume_session.clone());
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
                    generation: 0,
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
                        generation: 0,
                        result: Err(format!("connect after spawn: {e}")),
                        child_watch: None,
                    }
                }
            }
        }
        Err(e) => TabAttachOutcome {
            tab,
            generation: 0,
            result: Err(e),
            child_watch: None,
        },
    }
}

/// Max characters persisted for a tab's `/resume` snippet (#1466 item 3).
const TAB_SUMMARY_MAX_CHARS: usize = 60;

/// First line of `text`, sanitized of control/escape bytes and truncated to
/// `max` chars behind an ellipsis.
///
/// Sanitization (PR #1485 review): pasted input can carry raw ESC/BEL bytes
/// (OSC hyperlinks, SGR); the snippet is persisted to the manifest and later
/// replayed verbatim into the /resume selector, so control bytes must never
/// reach disk — mirroring how tab names are sanitized.
fn snippet_of(text: &str, max: usize) -> String {
    let raw = text.lines().next().unwrap_or("");
    let (line, _) = crate::components::ansi::sanitize_control_truncated(raw, usize::MAX);
    let line = line.trim();
    // Truncate through the shared sanitizer's max_chars/truncated contract
    // (PR #1485 review) instead of hand-rolling chars().take(): if the shared
    // truncation rule ever tightens, persisted snippets follow it. The trim
    // must happen before counting, so sanitize the trimmed line again.
    let (kept, truncated) = crate::components::ansi::sanitize_control_truncated(line, max);
    if !truncated {
        kept
    } else {
        let (head, _) =
            crate::components::ansi::sanitize_control_truncated(line, max.saturating_sub(1));
        format!("{head}…")
    }
}

#[cfg(test)]
#[path = "tab_lifecycle_tests.rs"]
mod tab_lifecycle_tests;
