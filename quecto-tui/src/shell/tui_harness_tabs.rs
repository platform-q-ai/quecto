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

    /// Whether `tab` carries the unread dot (output since last viewed).
    pub fn tab_unread(&self, tab: u32) -> bool {
        self.app.tab_unread(TabId(tab))
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
}

// Keep `App` referenced so the import stays honest if helpers shift.
const _: fn(&App) = |_| {};
