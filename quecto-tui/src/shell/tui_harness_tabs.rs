//! #1466 harness drivers: background-tab streaming through the production
//! render decision, tab activity indicators, retention, and workspace resume.

use super::TuiHarness;
use crate::protocol::client::Event;
use crate::shell::app::App;
use crate::shell::connection::{SourcedEvent, TabId};
use crate::shell::tab_registry::TabAgentRegistry;
use crate::shell::workspace_manifest::{
    WorkspaceManifest, WorkspaceManifestStore, WorkspaceTabEntry,
};

impl TuiHarness {
    /// Insert a disconnected background tab (id 1) without changing focus.
    pub fn open_background_tab(&mut self) -> &mut Self {
        self.app.test_insert_disconnected_tab(1);
        self
    }

    /// Deliver one event owned by `tab` through the production fan-in routing
    /// AND the production event-loop paint decision (`route_sourced` +
    /// `apply_sourced_render`), sharing the harness coalescer.
    pub fn sourced_stream_event_for_tab(&mut self, tab: u32, ev: Event) -> &mut Self {
        self.app.suppress_paint = true;
        let render = self.app.route_sourced(SourcedEvent::Tab(TabId(tab), ev));
        let mut coalescer = std::mem::take(&mut self.stream_coalescer);
        self.app.apply_sourced_render(render, &mut coalescer);
        self.stream_coalescer = coalescer;
        self
    }

    /// Whether `tab` shows the activity spinner (turn in flight).
    pub fn tab_spinner(&self, tab: u32) -> bool {
        self.app.tab_spinner_active(TabId(tab))
    }

    /// Whether `tab` carries unread output (output since last viewed).
    pub fn tab_unread(&self, tab: u32) -> bool {
        self.app
            .conn_for(TabId(tab))
            .is_some_and(|c| c.unread_output)
    }

    /// Switch focus to `tab` through the PRODUCTION key path: the Alt+digit
    /// primary goes through `handle_key`, then the event loop's stdin-arm
    /// paint policy (`render_and_note`) runs exactly as `run()` would — the
    /// harness adds no paint of its own beyond that policy.
    pub fn switch_to_tab(&mut self, tab: u32) -> &mut Self {
        let digit = char::from_digit(tab + 1, 10).expect("tab ordinal digit");
        self.app.suppress_paint = true;
        self.app.handle_key(crate::shell::keys::Key::Alt(digit));
        let mut coalescer = std::mem::take(&mut self.stream_coalescer);
        self.app.render_and_note(&mut coalescer);
        self.stream_coalescer = coalescer;
        self
    }

    /// Drain any deferred stream paint the way the loop's deadline arm would:
    /// if a coalesced paint is pending, treat its deadline as reached and
    /// paint. Lets tests assert "no frame even after the loop settles"
    /// without reaching into the coalescer's internals.
    pub fn settle_deferred_paints(&mut self) -> &mut Self {
        if let Some(deadline) = self.stream_coalescer.pending_deadline() {
            if self.stream_coalescer.render_due(deadline) {
                self.app.render();
            }
        }
        self
    }

    /// The focused tab's numeric id.
    pub fn active_tab_index(&self) -> u32 {
        self.app.active_tab.0
    }

    /// Whether the loop would keep scheduling animation ticks.
    pub fn animation_tick_needed(&self) -> bool {
        self.app.needs_animation_tick(false)
    }

    /// Start `n` sub-agent sessions on `tab` through the production
    /// `ensure_session` path (which enforces the retention cap), restoring
    /// the previously focused tab afterwards.
    pub fn start_sessions_on_tab(&mut self, tab: u32, n: usize) -> &mut Self {
        let prev = self.app.active_tab;
        assert!(self.app.switch_tab(TabId(tab)), "tab {tab} must exist");
        for i in 0..n {
            self.app.ensure_session(&format!("tab{tab}-agent-{i:03}"));
        }
        assert!(self.app.switch_tab(prev), "restore focus");
        self
    }

    /// Retained sub-agent session count for `tab`.
    pub fn retained_sessions_for_tab(&self, tab: u32) -> usize {
        self.app
            .conn_for(TabId(tab))
            .map(|c| c.roster.sessions.len())
            .unwrap_or(0)
    }

    /// Seed a durable workspace manifest at `path` (#1466 identity metadata).
    pub fn seed_workspace_manifest(
        path: &std::path::Path,
        workspace_id: &str,
        label: &str,
        last_active_unix_s: u64,
        with_session_key: bool,
    ) {
        let mut store = WorkspaceManifestStore::load(path);
        store.upsert(WorkspaceManifest {
            workspace_id: workspace_id.to_string(),
            label: label.to_string(),
            last_active_unix_s,
            active_index: 0,
            tabs: vec![WorkspaceTabEntry {
                tab_id: 0,
                session_key: with_session_key.then(|| "sess-1".to_string()),
                name: None,
                summary: None,
            }],
            updated_unix_s: last_active_unix_s,
        });
        store.store(path).expect("store manifest");
    }

    /// Open the resume selector against the manifest at `path` and return the
    /// workspace rows as `(label, description)` pairs.
    pub fn open_resume_with_manifest(&mut self, path: &std::path::Path) -> Vec<(String, String)> {
        self.app
            .open_resume_selector_with_workspaces(Vec::new(), path, None);
        self.app
            .ac()
            .sessions
            .resume_selector
            .as_ref()
            .map(|sel| {
                sel.items_for_tests()
                    .iter()
                    .map(|i| (i.label.clone(), i.description.clone().unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Run orphaned-workspace GC over the manifest at `path` with an EMPTY
    /// tab-agent registry; returns removed workspace ids.
    pub fn gc_orphaned_workspaces(path: &std::path::Path) -> Vec<String> {
        let mut store = WorkspaceManifestStore::load(path);
        let removed = store.gc_orphaned(&TabAgentRegistry::new());
        store.store(path).expect("store manifest");
        removed
    }

    /// Mint a workspace id through the production generator.
    pub fn generate_workspace_id() -> String {
        crate::shell::workspace_manifest::generate_workspace_id()
    }

    /// Feed one raw key byte-sequence through the production parser and key
    /// handler (kitty alias verification, #1466 decision 5).
    pub fn press_raw(&mut self, bytes: &[u8]) -> &mut Self {
        let (key, _) = crate::shell::keys::parse_key(bytes).expect("parseable key sequence");
        self.app.suppress_paint = true;
        self.app.handle_key(key);
        self
    }

    /// Route a background-tab end-of-turn (`AgentEnd`). Note this is the
    /// generic completion event; the client-driven abort path
    /// (`handle_abort` → `agent_state.abort()`) is covered by unit tests in
    /// `app_multi_tab_polish_tests.rs` since abort can only target the
    /// focused tab.
    pub fn end_background_turn(&mut self, tab: u32) -> &mut Self {
        self.sourced_stream_event_for_tab(
            tab,
            Event::AgentEnd {
                messages: Vec::new(),
                message_refs: Vec::new(),
            },
        )
    }

    /// Direct access for unit tests: mark a tab unread (pre-seeding clears).
    pub fn force_tab_unread(&mut self, tab: u32) {
        if let Some(c) = self.app.conn_mut(TabId(tab)) {
            c.unread_output = true;
        }
    }

    // ── #1466 fix-pass accessors (PR #1485 regressions) ──────────────────

    /// The rendered tab-bar line (empty because tabs were removed).
    pub fn tab_bar_line(&mut self, _width: usize) -> String {
        String::new()
    }

    /// Left-click at absolute terminal cell (col, row) through the
    /// production key path (press + release, no drag).
    pub fn click(&mut self, col: u16, row: u16) -> &mut Self {
        self.app.suppress_paint = true;
        self.app
            .handle_key(crate::shell::keys::Key::MousePress(col, row));
        self.app
            .handle_key(crate::shell::keys::Key::MouseRelease(col, row));
        self
    }

    /// Number of open tabs.
    pub fn tab_count(&self) -> usize {
        self.app.tabs.len()
    }

    /// Click the old tab-block location. Tab UI was removed, so this is inert.
    pub fn click_tab_block(&mut self, _ordinal: usize) -> &mut Self {
        self.click(0, 0)
    }

    /// Click the old new-tab button location. Tab UI was removed, so this is inert.
    pub fn click_new_tab_button(&mut self) -> &mut Self {
        self.click(0, 0)
    }

    /// Click old tab-bar dead space. Tab UI was removed, so this is an ordinary click.
    pub fn click_past_tab_bar(&mut self) -> &mut Self {
        self.click(0, 0)
    }

    /// Track a sub-agent roster entry with `status` on the active tab.
    pub fn track_subagent(&mut self, id: &str, status: &str) -> &mut Self {
        self.app
            .update_subagent_bar(vec![crate::protocol::client::SubagentInfoEvent {
                agent_uuid: None,
                display_name: None,
                agent_id: id.to_string(),
                status: status.to_string(),
                last_tool: None,
                last_error: None,
                compact: false,
                pid: 0,
                socket_path: None,
                parent_id: None,
                workflow: None,
                read_only: false,
                execution_backend: None,
                environment: None,
            }]);
        self
    }

    /// The active session's most recent Status chat line, if any.
    pub fn last_status_line(&self) -> Option<String> {
        self.app
            .active_session()
            .chat
            .last_status_text()
            .map(str::to_string)
    }

    /// Number of raised notifications.
    pub fn notification_count(&self) -> usize {
        self.app.notifications.messages().len()
    }

    /// The most recent notification's message text, if any.
    pub fn last_notification(&self) -> Option<String> {
        self.app.notifications.messages().last().cloned()
    }

    /// Insert a SECOND disconnected background tab (id 2) — three tabs total,
    /// so cycle-direction assertions are falsifiable.
    pub fn open_second_background_tab(&mut self) -> &mut Self {
        self.app.test_insert_disconnected_tab(2);
        self
    }

    /// Current spinner frame index for `tab`'s master turn, if spinning.
    pub fn tab_spinner_frame(&self, tab: u32) -> Option<usize> {
        self.app
            .conn_for(TabId(tab))
            .and_then(|c| c.spinner.as_ref())
            .map(|s| s.frame_index())
    }

    /// Apply a two-entry workspace manifest whose STORED tab ids (7/8) do not
    /// exist in this fresh TUI (#1466 fix-pass item 4). Returns whether each
    /// entry's session ended up carried by some tab (latched or resumed).
    pub fn apply_manifest_with_stale_tab_ids(&mut self) -> (bool, bool) {
        self.app.ac_mut().agent_connected = true;
        let entry = |tab_id: u32, key: &str| WorkspaceTabEntry {
            tab_id,
            session_key: Some(key.to_string()),
            name: None,
            summary: None,
        };
        let manifest = App::test_workspace_manifest(
            "ws-stale-ids",
            vec![entry(7, "sess-a"), entry(8, "sess-b")],
            0,
        );
        self.app.apply_workspace_manifest(&manifest);
        let carries = |key: &str| {
            self.app.tabs.values().any(|c| {
                c.session_key.as_deref() == Some(key)
                    || c.pending_session_resume.as_deref() == Some(key)
            })
        };
        (carries("sess-a"), carries("sess-b"))
    }

    /// Seed one workspace row with full #1466 fix-pass metadata.
    pub fn seed_workspace_row(
        path: &std::path::Path,
        workspace_id: &str,
        label: &str,
        last_active_unix_s: u64,
        session_key: Option<&str>,
        summary: Option<&str>,
    ) {
        let mut store = WorkspaceManifestStore::load(path);
        store.upsert(WorkspaceManifest {
            workspace_id: workspace_id.to_string(),
            label: label.to_string(),
            last_active_unix_s,
            active_index: 0,
            tabs: vec![WorkspaceTabEntry {
                tab_id: 0,
                session_key: session_key.map(str::to_string),
                name: None,
                summary: summary.map(str::to_string),
            }],
            updated_unix_s: last_active_unix_s,
        });
        store.store(path).expect("store manifest");
    }
}

// ── #1466 round-2 fix-pass drivers/probes ────────────────────────────────

impl TuiHarness {
    /// Compose the full frame through the production path (ANSI intact).
    pub fn frame_lines(&mut self) -> Vec<String> {
        self.app.compose_frame()
    }

    /// The harness terminal's height in rows.
    pub fn terminal_height(&self) -> usize {
        self.app.terminal.height
    }

    /// Whether the tool-policy selector has been requested (its catalogue
    /// fetch is in flight).
    pub fn tool_policy_selector_requested(&self) -> bool {
        self.app.tool_policy_modal_pending_catalogue_id.is_some()
    }

    /// Focus a running sub-agent restored by workspace resume: focused BEFORE
    /// its live socket was known, whose socket then becomes reachable — the
    /// #1466 round-2 field state behind "not attached" send failures.
    pub fn focus_restored_running_subagent(&mut self, id: &str) {
        self.track_subagent(id, "running");
        self.app.select_agent(Some(id));
        let socket = super::events::spawn_subagent_socket(id);
        self.app
            .ac_mut()
            .roster
            .tracked
            .get_mut(id)
            .expect("tracked sub-agent")
            .info
            .socket_path = Some(socket.to_string_lossy().into_owned());
    }

    /// Focus a sub-agent still marked detached whose registry socket is live.
    pub fn focus_detached_reachable_subagent(&mut self, id: &str) {
        let socket = super::events::spawn_subagent_socket(id);
        self.app
            .update_subagent_bar(vec![super::events::subagent_with_socket(
                id,
                "detached",
                None,
                Some(socket),
            )]);
        self.app.select_agent(Some(id));
    }

    /// Focus a dead sub-agent (no reachable socket).
    pub fn focus_dead_subagent(&mut self, id: &str) {
        self.track_subagent(id, "dead");
        self.app.select_agent(Some(id));
    }

    /// User entries in the ACTIVE session's transcript.
    pub fn active_user_entries(&self) -> Vec<String> {
        self.app
            .active_session()
            .chat
            .entries()
            .iter()
            .filter_map(|e| match e {
                crate::components::chat::ChatEntry::User { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }
}

// Keep `App` referenced so the import stays honest if helpers shift.
const _: fn(&App) = |_| {};
