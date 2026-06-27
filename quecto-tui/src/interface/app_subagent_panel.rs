//! Sub-agent-first persistent left panel + multi-session switching (#800).
//!
//! Replaces the bolt-on inspector (#795/#796/#798). A persistent left column
//! lists the **Master Agent** and the sub-agent tree (indented by `parent_id`).
//! Selecting an agent switches the main body to that agent's own `SessionView`,
//! rendering its full live session via a direct **connect-on-select** UDS
//! connection to the sub-agent's own socket. Esc returns to the master.

use super::*;
use crate::interface::theme;

impl App {
    // ── Visibility / active session ────────────────────────────────────

    /// Whether the persistent left panel is shown. Sub-agent-first default
    /// (#820): ALWAYS on once connected — the Master is pinned as the top row
    /// even with no sub-agents, so the panel is not gated on the tree.
    pub(super) fn subagent_panel_visible(&self) -> bool {
        self.agent_connected
    }

    /// The agent whose session is currently shown in the body. `None` = master.
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn active_agent_id(&self) -> Option<&str> {
        self.active_agent_id.as_deref()
    }

    /// Ids of all retained sub-agent sessions (live or exited but still
    /// viewable per the retention policy).
    #[cfg(test)]
    pub(super) fn retained_session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// The socket path the connect-on-select connection would dial for `id`, as
    /// surfaced by the kernel (#800). `None` when unknown.
    #[cfg(test)]
    pub(super) fn subagent_socket_path(&self, id: &str) -> Option<String> {
        self.subagent_local
            .get(id)
            .and_then(|t| t.info.socket_path.clone())
    }

    /// The active session — the master (`active_agent_id == None`) or the
    /// selected sub-agent — modeled identically as a [`SessionView`] (#828).
    /// This is the SINGLE place that maps the active id to its session; every
    /// render/input accessor below delegates here so there is no master-vs-
    /// sub-agent branching scattered across the render path.
    pub(super) fn active_session(&self) -> &SessionView {
        match self.active_agent_id.as_deref() {
            None => &self.master_session,
            // Fall back to the master session if a selected session is somehow
            // missing, mirroring `active_session_mut`'s lazy-create contract.
            Some(id) => self.sessions.get(id).unwrap_or(&self.master_session),
        }
    }

    /// Mutable counterpart to [`active_session`]. Lazily creates the selected
    /// sub-agent's session so a selection always has a body to render; the
    /// master session always exists.
    pub(super) fn active_session_mut(&mut self) -> &mut SessionView {
        let Some(id) = self.active_agent_id.clone() else {
            return &mut self.master_session;
        };
        if !self.sessions.contains_key(&id) {
            // Cold path only: clone git_branch and build the session here, so the
            // common already-exists render path allocates nothing extra (#827 perf).
            let git_branch = self.git_branch.clone();
            Self::remember_session(&mut self.session_order, &id);
            self.sessions
                .insert(id.clone(), SessionView::new(git_branch));
        }
        self.sessions.get_mut(&id).unwrap()
    }

    /// The chat buffer for the active session (master or selected sub-agent).
    pub(super) fn active_chat_mut(&mut self) -> &mut Chat {
        &mut self.active_session_mut().chat
    }

    /// Test-only: number of chat entries in a sub-agent session (for asserting
    /// the deferred-note buffer cap independently of the rendered viewport).
    #[cfg(test)]
    pub(crate) fn session_chat_entry_count(&self, agent_id: &str) -> Option<usize> {
        self.sessions.get(agent_id).map(|s| s.chat.entry_count())
    }

    /// The active session's workflow/phase bar (master or selected sub-agent).
    pub(super) fn active_workflow_bar(&self) -> &workflow_bar::WorkflowBarState {
        &self.active_session().workflow_bar
    }

    /// Render the active session's status footer (master or selected sub-agent).
    pub(super) fn active_footer_render(&mut self, width: usize) -> Vec<String> {
        self.active_session_mut().footer.render(width)
    }

    /// Whether the active session is mid-turn. For a selected sub-agent this is
    /// its forwarded `running` flag; for the master it mirrors `agent_state`
    /// (kept in sync on start/end), so the working indicator is driven by ONE
    /// per-session flag for both. The master additionally owns the richer tool
    /// `spinner` telemetry, layered on top by the render path.
    pub(super) fn active_subagent_running(&self) -> bool {
        let session = self.active_session();
        if session.running {
            return true;
        }
        // Once this session's OWN stream has reported a run-state event,
        // `running` is authoritative — don't let the (lagging) master-tracked
        // status override an accurate idle (which would leave the spinner up or
        // make Esc abort instead of navigate, #834 review).
        if session.observed_run_state {
            return false;
        }
        // No stream state observed yet: connect-on-select may have joined
        // MID-TURN and missed `agent_start`, so `session.running` reads a false
        // negative. Fall back to the master's tracked status (`subagent_local`)
        // so Esc still cancels a busy sub-agent instead of navigating to master.
        match &self.active_agent_id {
            Some(id) => self
                .subagent_local
                .get(id)
                .is_some_and(|t| subagent_status_is_active(&t.info.status)),
            None => false,
        }
    }

    /// The focus region currently holding keyboard input (#802, tests).
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn focus_region(&self) -> Focus {
        self.focus
    }

    /// The 0-based panel highlight index (#802, tests).
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn panel_highlight_index(&self) -> usize {
        self.panel_nav.selected()
    }

    /// Send a command to the active sub-agent's own connection (#802). Returns
    /// `true` only if it was actually enqueued onto a live matching sender.
    ///
    /// Enqueues synchronously with `try_send` rather than spawning a task per
    /// call: independent tasks gave no ordering guarantee, so a routed `Abort`
    /// could overtake the `Prompt` it was meant to cancel (#804 review). A
    /// single bounded channel drained by the connection task now preserves
    /// submit order. A missing/mismatched sender (older kernel, or the
    /// connection task failed `Client::connect`) or a full channel returns
    /// `false` so the caller can surface the dropped command rather than
    /// reporting a phantom success.
    pub(super) fn send_to_active_subagent(&mut self, cmd: Command) -> bool {
        let Some(id) = self.active_agent_id.clone() else {
            return false;
        };
        let Some((conn_id, tx)) = &self.active_subagent_cmd_tx else {
            return false;
        };
        if conn_id != &id {
            return false;
        }
        tx.try_send(cmd).is_ok()
    }

    // ── Selection / switching ──────────────────────────────────────────

    /// Switch the active session. `None` selects the master; `Some(id)` selects
    /// that sub-agent, creating/retaining its `SessionView` and opening a direct
    /// connect-on-select connection to its live stream.
    pub(super) fn select_agent(&mut self, agent_id: Option<&str>) {
        let new_active = agent_id.map(str::to_string);
        if new_active == self.active_agent_id {
            return;
        }
        // Tear down the previous sub-agent connection, if any.
        self.teardown_active_connection();
        self.active_agent_id = new_active.clone();
        self.sync_panel_selection_to_active();

        let Some(id) = new_active else {
            // Master selected: nothing to dial.
            return;
        };
        // Ensure a session exists (retained for later viewing).
        self.ensure_session(&id);
        // Connect-on-commit: the selection only changes on an explicit commit
        // (Enter/Tab/digit-jump) now that highlight movement is decoupled from
        // selection (#802), so the old debounce is gone — open immediately.
        self.open_subagent_connection(&id);
    }

    /// Reconcile the active/pending session when the viewed sub-agent leaves
    /// the live list (e.g. it exited and its grace period elapsed). The panel
    /// only lists tracked agents, so an `active_agent_id` that is no longer
    /// tracked would leave the body rendering a session with no matching panel
    /// row — and the panel itself may have vanished. Fall back to the master so
    /// body and panel always agree (#800 review).
    fn reconcile_active_agent(&mut self) {
        if let Some(active) = self.active_agent_id.clone() {
            if !self.subagent_local.contains_key(&active) {
                self.select_agent(None);
            }
        }
    }

    /// Create the session for `id` if missing, recording retention order and
    /// evicting the oldest non-active session beyond the cap.
    pub(super) fn ensure_session(&mut self, id: &str) {
        if !self.sessions.contains_key(id) {
            self.sessions
                .insert(id.to_string(), SessionView::new(self.git_branch.clone()));
            Self::remember_session(&mut self.session_order, id);
            self.evict_retained_sessions();
        }
    }

    fn remember_session(order: &mut Vec<String>, id: &str) {
        if !order.iter().any(|o| o == id) {
            order.push(id.to_string());
        }
    }

    /// Bound retained-session memory: drop the oldest sessions (never the
    /// active one) once the retained count exceeds `MAX_RETAINED_SESSIONS`.
    fn evict_retained_sessions(&mut self) {
        while self.sessions.len() > MAX_RETAINED_SESSIONS {
            let Some(pos) = self
                .session_order
                .iter()
                .position(|id| Some(id) != self.active_agent_id.as_ref())
            else {
                break;
            };
            let victim = self.session_order.remove(pos);
            self.sessions.remove(&victim);
        }
    }

    // ── Panel keyboard navigation (#802 focus model) ──────────────────

    /// Move the panel highlight up (toward the master) WITHOUT switching the
    /// active session — commit happens only on Enter/Tab (#802).
    pub(super) fn panel_highlight_previous(&mut self) {
        let len = self.panel_rows().len();
        self.panel_nav.move_previous(len);
    }

    /// Move the panel highlight down WITHOUT switching the active session.
    pub(super) fn panel_highlight_next(&mut self) {
        let len = self.panel_rows().len();
        self.panel_nav.move_next(len);
    }

    /// Jump the panel highlight to a 1-based row number (digits 1–9). Row 1 is
    /// the master; row N+1 is the Nth listed sub-agent. No-op past the end.
    pub(super) fn panel_highlight_row(&mut self, one_based: usize) {
        let len = self.panel_rows().len();
        if one_based >= 1 && one_based <= len {
            self.panel_nav.set_selected(one_based - 1);
        }
    }

    /// Commit the highlighted panel row: switch the active session to that agent
    /// (the master when row 1 is highlighted) and open its connection (#802).
    pub(super) fn commit_panel_selection(&mut self) {
        let target = self
            .panel_rows()
            .get(self.panel_nav.selected())
            .and_then(|r| r.id.clone());
        self.select_agent(target.as_deref());
    }

    /// Keep the panel cursor pointing at the active agent (or the master row)
    /// after the underlying list changes.
    pub(super) fn clamp_panel_selection(&mut self) {
        // If the active agent dropped out of the live list, fall back to the
        // master before reconciling the cursor (#800 review).
        self.reconcile_active_agent();
        let rows = self.panel_rows();
        self.panel_nav.clamp(rows.len());
        self.sync_panel_selection_to_active_with(&rows);
    }

    pub(super) fn sync_panel_selection_to_active(&mut self) {
        let rows = self.panel_rows();
        self.sync_panel_selection_to_active_with(&rows);
    }

    fn sync_panel_selection_to_active_with(&mut self, rows: &[PanelRow]) {
        if let Some(idx) = rows
            .iter()
            .position(|r| r.id.as_deref() == self.active_agent_id.as_deref())
        {
            self.panel_nav.set_selected(idx);
        }
    }

    // ── Connect-on-select ──────────────────────────────────────────────

    /// Open a direct UDS connection to `id`'s own socket and fan its live
    /// stream into the shared `subagent_event_rx`, tagged with the agent id.
    /// No-op when the socket path is unknown (older kernel / non-local agent).
    fn open_subagent_connection(&mut self, id: &str) {
        let Some(socket) = self
            .subagent_local
            .get(id)
            .and_then(|t| t.info.socket_path.clone())
        else {
            return;
        };
        let tx = self.subagent_event_tx.clone();
        let agent_id = id.to_string();
        let path = std::path::PathBuf::from(socket);
        // Outbound channel so the editor's send/abort path can steer this child
        // (#802); the connection task forwards queued commands onto its socket.
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);
        let handle = tokio::spawn(async move {
            let Ok(mut client) = Client::connect(&path).await else {
                return;
            };
            // The kernel sends a connect-time get_messages snapshot of the
            // pre-turn conversation immediately on connect (#828) — served by
            // the accept loop, independent of the child's (possibly busy)
            // dispatch loop — so prior history shows at once for a BUSY child,
            // not just an idle one. This explicit get_messages is a follow-up
            // refresh; the TUI reconciles both via `Chat::prepend_history`.
            let _ = client
                .send(&Command::GetMessages {
                    id: Some("subagent-history".into()),
                })
                .await;
            loop {
                tokio::select! {
                    ev = client.recv() => match ev {
                        Some(ev) => {
                            if tx.send((agent_id.clone(), ev)).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    },
                    cmd = cmd_rx.recv() => match cmd {
                        // Steer/abort routed from the editor; the child's single
                        // dispatch loop queues a prompt until its turn ends.
                        Some(cmd) => {
                            let _ = client.send(&cmd).await;
                        }
                        None => break,
                    },
                }
            }
        });
        self.active_conn = Some((id.to_string(), handle));
        self.active_subagent_cmd_tx = Some((id.to_string(), cmd_tx));
    }

    /// Abort the active sub-agent connection's forwarding task, if any.
    fn teardown_active_connection(&mut self) {
        if let Some((_, handle)) = self.active_conn.take() {
            handle.abort();
        }
        self.active_subagent_cmd_tx = None;
    }

    // ── Panel rendering ────────────────────────────────────────────────

    /// Flattened panel rows: the master pinned at the top, then the sub-agent
    /// tree depth-ordered by `parent_id` (grandchildren under their parent).
    /// Master's live status: `running` while processing, else `idle` (#820).
    fn master_status(&self) -> &'static str {
        if self.agent_state.is_running() {
            "running"
        } else {
            "idle"
        }
    }

    fn panel_rows(&self) -> Vec<PanelRow> {
        let mut rows = vec![PanelRow {
            id: None,
            prefix: String::new(),
            label: "Master Agent".to_string(),
            status: self.master_status().to_string(),
        }];
        for (id, prefix) in self.subagent_tree_order() {
            let info = self.subagent_local.get(&id).map(|t| &t.info);
            rows.push(PanelRow {
                label: id.clone(),
                status: info.map(|i| i.status.clone()).unwrap_or_default(),
                id: Some(id),
                prefix,
            });
        }
        rows
    }

    /// Depth-first `(agent_id, tree_prefix)` listing of the sub-agent tree. Root
    /// sub-agents (no in-map parent) sit under the master; `tree_prefix` is the
    /// connector stalk (`├ `/`└ ` with `│ `/`  ` ancestor continuation) so the
    /// panel draws tree lines back up to each parent. Order follows sorted ids.
    fn subagent_tree_order(&self) -> Vec<(String, String)> {
        use std::collections::BTreeMap;
        let mut children: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
        for (id, tracked) in &self.subagent_local {
            // Treat an unknown parent as a root so its subtree is not lost.
            let parent = tracked
                .info
                .parent_id
                .clone()
                .filter(|p| self.subagent_local.contains_key(p));
            children.entry(parent).or_default().push(id.clone());
        }
        // Push siblings reversed (with their connector) so popping preserves order.
        // Stack item: (id, own_prefix, descendant_continuation_prefix).
        let push_children =
            |stack: &mut Vec<(String, String, String)>, kids: &[String], cont: &str| {
                let n = kids.len();
                for (i, kid) in kids.iter().enumerate().rev() {
                    let last = i == n - 1;
                    stack.push((
                        kid.clone(),
                        format!("{cont}{}", if last { "└ " } else { "├ " }),
                        format!("{cont}{}", if last { "  " } else { "│ " }),
                    ));
                }
            };
        let mut out = Vec::new();
        let mut stack: Vec<(String, String, String)> = Vec::new();
        if let Some(roots) = children.get(&None) {
            push_children(&mut stack, roots, "");
        }
        while let Some((id, own_prefix, cont)) = stack.pop() {
            out.push((id.clone(), own_prefix));
            if let Some(kids) = children.get(&Some(id.clone())) {
                push_children(&mut stack, kids, &cont);
            }
        }
        out
    }

    /// Render the persistent left panel into exactly `height` rows, each padded
    /// to `width` visible columns. Row 0 is a panel header; the selected row is
    /// highlighted. Width-aware so the horizontal split joins cleanly (#800).
    pub(super) fn render_subagent_panel(
        &self,
        width: usize,
        height: usize,
        now: tokio::time::Instant,
    ) -> Vec<String> {
        let rows = self.panel_rows();
        let selected = self.panel_nav.selected();
        let active = self.active_agent_id.as_deref();
        // Header doubles as the panel's `Agents` summary, with the live count
        // (replacing the removed bottom sub-agent bar's count line, #820).
        let mut lines: Vec<String> = Vec::with_capacity(height);
        lines.push(pad_cell(
            &format!(
                "  {} {}",
                theme::accent(&theme::bold("Agents")),
                theme::dim(&format!("{}", self.subagent_local.len() + 1))
            ),
            width,
        ));
        for (i, row) in rows.iter().enumerate() {
            // Sub-agent-first row (#820): `<caret><indent> name <elapsed>` — NO
            // status dot/glyph; the NAME TEXT carries the status colour and the
            // `▸` caret marks the active session shown in the main pane.
            let is_active = row.id.as_deref() == active;
            let caret = if is_active {
                theme::accent("▸ ")
            } else {
                "  ".to_string()
            };
            let stalk = theme::dim(&row.prefix); // tree connector to parent (#820)
            let label = status_colored_name(&row.status, &sanitize_panel_label(&row.label));
            let elapsed = theme::dim(&self.panel_row_elapsed(row.id.as_deref(), now));
            let text = format!("{caret}{stalk}{label} {elapsed}");
            let cell = if i == selected {
                pad_cell(&theme::reverse(&text), width)
            } else {
                pad_cell(&text, width)
            };
            lines.push(cell);
        }
        // Footer hint pinned to the bottom row (#820): pane navigation + actions.
        let hint = pad_cell(&theme::dim("Tab ⇄ · ↑↓ · ⏎ open"), width);
        while lines.len() + 1 < height {
            lines.push(" ".repeat(width));
        }
        if lines.len() < height {
            lines.push(hint);
        }
        lines.truncate(height);
        lines
    }

    /// The per-row elapsed label for the panel (#820): the Master row shows the
    /// session uptime; a sub-agent row shows its running/idle/frozen timer.
    fn panel_row_elapsed(&self, id: Option<&str>, now: tokio::time::Instant) -> String {
        let Some(id) = id else {
            // Master row → session uptime.
            return fmt_mss(now.saturating_duration_since(self.started_at).as_secs());
        };
        let Some(t) = self.subagent_local.get(id) else {
            return String::new();
        };
        // Per-row timer: idle → `idle m:ss` (time since it went idle); running →
        // live `m:ss`; errored/exited → frozen running `m:ss` (#820).
        if t.info.status == "idle" {
            let since = t
                .stopped_at
                .map(|s| now.saturating_duration_since(s).as_secs())
                .unwrap_or(0);
            format!("idle {}", fmt_mss(since))
        } else {
            fmt_mss(t.elapsed_secs(now))
        }
    }

    // ── Main-pane workflow indicator (selected agent) ──────────────────

    /// The main pane's top chrome for the selected agent (#820): a title line
    /// (`agent · status · elapsed · #issue workflow`) followed by the EXISTING
    /// yellow workflow bar rendered BOXED as a single content line
    /// (`progress-bar PHASE n/total`) — dropping the phase-pills and hints lines.
    /// Title always renders; the boxed bar is appended only when a workflow exists.
    pub(super) fn render_main_pane_workflow(
        &self,
        width: usize,
        now: tokio::time::Instant,
    ) -> Vec<String> {
        if width < 4 {
            return Vec::new();
        }
        let state = self.active_workflow_bar();
        // Title ALWAYS renders; the boxed bar is conditional on a workflow (#820).
        let mut out = vec![pad_cell(&self.main_pane_title(state, now), width)];
        if let Some(content) = workflow_bar::render_compact_line(state) {
            // Box the single workflow line for breathing space (#820).
            let inner = width - 2;
            out.push(theme::dim(&format!("┌{}┐", "─".repeat(inner))));
            out.push(format!(
                "{}{}{}",
                theme::dim("│"),
                boxed_inner(&content, inner),
                theme::dim("│")
            ));
            out.push(theme::dim(&format!("└{}┘", "─".repeat(inner))));
        }
        out
    }

    /// Build the main-pane title line for the active agent (#820).
    fn main_pane_title(
        &self,
        state: &workflow_bar::WorkflowBarState,
        now: tokio::time::Instant,
    ) -> String {
        let (name, status) = match self.active_agent_id.as_deref() {
            None => ("Master".to_string(), self.master_status().to_string()),
            Some(id) => (
                id.to_string(),
                self.subagent_local
                    .get(id)
                    .map(|t| t.info.status.clone())
                    .unwrap_or_default(),
            ),
        };
        let elapsed = self.panel_row_elapsed(self.active_agent_id.as_deref(), now);
        let mut title = format!(
            "{} {} {} {}",
            theme::bold(&sanitize_panel_label(&name)),
            theme::dim("·"),
            status_colored_name(&status, &sanitize_panel_label(&status)),
            theme::dim(&elapsed),
        );
        if let Some(n) = state.issue_number {
            title.push_str(&format!(
                " {} {} {}",
                theme::dim("·"),
                theme::accent(&theme::bold(&format!("#{n}"))),
                theme::dim("workflow"),
            ));
        }
        title
    }
}

/// Format an elapsed duration as `m:ss` (or `h:mm:ss` past an hour) for the
/// sub-agent-first panel's per-row timers (#820).
fn fmt_mss(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// Pad (ANSI-aware) a boxed workflow line's content to exactly `inner` columns.
fn boxed_inner(content: &str, inner: usize) -> String {
    let visible = crate::interface::utils::visible_width(content);
    if visible >= inner {
        crate::interface::utils::truncate_to_width(content, inner, None)
    } else {
        format!("{}{}", content, " ".repeat(inner - visible))
    }
}

/// One flattened entry in the left panel: the master (`id == None`) or a
/// sub-agent, with its tree depth and last-known status.
struct PanelRow {
    id: Option<String>,
    /// Tree connector stalk drawn before the name (`├ `/`└ ` + ancestor `│ `).
    prefix: String,
    label: String,
    status: String,
}

/// Colour a panel row's NAME by status (#820): green = running, orange/yellow =
/// idle, red = errored. Exited names dim out; unknown states stay uncoloured.
/// No glyph is emitted — the colour alone conveys the state.
fn status_colored_name(status: &str, name: &str) -> String {
    match status {
        "running" | "starting" => theme::green(name),
        "idle" => theme::yellow(name),
        "error" | "errored" => theme::red(name),
        "exited" => theme::dim(name),
        _ => name.to_string(),
    }
}

/// Strip terminal control sequences from a panel label.
fn sanitize_panel_label(s: &str) -> String {
    crate::interface::components::sanitize::strip_terminal_control(s)
}

/// Pad (or truncate) a cell to exactly `width` visible columns.
fn pad_cell(text: &str, width: usize) -> String {
    let visible = crate::interface::utils::visible_width(text);
    if visible > width {
        crate::interface::utils::truncate_to_width(text, width, None)
    } else {
        format!("{}{}", text, " ".repeat(width - visible))
    }
}
