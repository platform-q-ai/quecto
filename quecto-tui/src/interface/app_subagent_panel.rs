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

    /// The chat buffer for the active session: the master's own `self.chat`
    /// when no sub-agent is selected, otherwise the selected agent's session
    /// (created lazily so a selection always has a body to render).
    pub(super) fn active_chat_mut(&mut self) -> &mut Chat {
        match &self.active_agent_id {
            None => &mut self.chat,
            Some(id) => {
                let id = id.clone();
                let git_branch = self.git_branch.clone();
                &mut self
                    .sessions
                    .entry(id.clone())
                    .or_insert_with(|| {
                        Self::remember_session(&mut self.session_order, &id);
                        SessionView::new(git_branch)
                    })
                    .chat
            }
        }
    }

    /// The active session's workflow bar: the master's own when no sub-agent is
    /// selected, otherwise the selected agent's (#802). Falls back to the master
    /// bar if the session is somehow missing.
    pub(super) fn active_workflow_bar(&self) -> &workflow_bar::WorkflowBarState {
        match &self.active_agent_id {
            None => &self.workflow_bar,
            Some(id) => self
                .sessions
                .get(id)
                .map(|s| &s.workflow_bar)
                .unwrap_or(&self.workflow_bar),
        }
    }

    /// Render the active session's footer: the master's own `self.footer` when
    /// no sub-agent is selected, otherwise the selected agent's per-session
    /// footer (#805). Falls back to the master footer if the session is missing.
    pub(super) fn active_footer_render(&mut self, width: usize) -> Vec<String> {
        // Borrow-split the disjoint fields (`active_agent_id`, `sessions`,
        // `footer`) so we read the id by reference — no per-frame allocation on
        // the render hot path while a sub-agent is selected.
        match self.active_agent_id.as_deref() {
            Some(id) if self.sessions.contains_key(id) => {
                self.sessions.get_mut(id).unwrap().footer.render(width)
            }
            _ => self.footer.render(width),
        }
    }

    /// Whether the active sub-agent is mid-turn (drives the per-session working
    /// indicator). Always `false` for the master, whose spinner is separate.
    pub(super) fn active_subagent_running(&self) -> bool {
        match &self.active_agent_id {
            None => false,
            Some(id) => self.sessions.get(id).is_some_and(|s| s.running),
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
    fn ensure_session(&mut self, id: &str) {
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
            // Backfill history; the live stream flows immediately while a busy
            // child's dispatch loop answers this (issue: ≤5s backfill).
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

    // ── Routing the sub-agent stream into its session ──────────────────

    /// Route one event from a sub-agent's direct connection into that agent's
    /// `SessionView`, mirroring the master render path so the body is visibly
    /// equivalent to how the master renders (#800).
    pub(super) fn route_subagent_event(&mut self, agent_id: &str, ev: Event) {
        // Tearing down a connection mid-stream can leave already-queued
        // `(old_id, ev)` items in `subagent_event_rx`. Drop events for agents
        // that are neither still tracked nor have a retained session, so a
        // stale frame cannot resurrect a session `evict_retained_sessions`
        // just dropped (#800 review).
        if !self.sessions.contains_key(agent_id) && !self.subagent_local.contains_key(agent_id) {
            return;
        }
        self.ensure_session(agent_id);
        let Some(session) = self.sessions.get_mut(agent_id) else {
            return;
        };
        // Per-session workflow bar so a selected sub-agent renders its OWN
        // workflow/phase bar, footer-equivalent chrome and running state (#802).
        if let Event::WorkflowState {
            agent_id: _,
            steps,
            progress,
            active_issue,
            mode,
            active_template,
            available_templates,
        } = &ev
        {
            session.workflow_bar = super::app_events::build_workflow_state(
                steps,
                progress,
                active_issue,
                mode,
                active_template,
                available_templates,
            );
            return;
        }
        match &ev {
            Event::AgentStart | Event::TurnStart => {
                session.running = true;
                session.footer.set_streaming(true);
            }
            Event::AgentEnd { .. } | Event::TurnEnd { .. } => {
                session.running = false;
                session.footer.set_streaming(false);
            }
            _ => {}
        }
        // Per-session FOOTER: feed the child's OWN context-window / cost / model
        // gauges from its forwarded events so a selected sub-agent shows ITS
        // usage, not the master's (#805).
        Self::update_session_footer(session, &ev);
        let chat = &mut session.chat;
        match ev {
            Event::Token { token } => chat.append_token(&token),
            Event::AgentEnd { .. } | Event::TurnEnd { .. } => chat.finalize_assistant(),
            Event::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let args_str = if args.is_object() || args.is_array() {
                    serde_json::to_string(&args).unwrap_or_default()
                } else {
                    args.to_string()
                };
                if !super::app_events::suppress_tool_box(&tool_name, &args) {
                    chat.start_tool(tool_call_id, tool_name, args_str);
                }
            }
            Event::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                ..
            } => {
                let text = crate::infrastructure::client::extract_result_text(&result);
                chat.complete_tool(&tool_call_id, &text, is_error, None);
            }
            Event::Response {
                command,
                data: Some(data),
                ..
            } if command == "get_messages" => {
                Self::replace_session_chat(chat, &data);
            }
            _ => {}
        }
    }

    /// Update a session's OWN footer (context-window / cost / model) from a
    /// forwarded sub-agent event, mirroring the master footer path (#805):
    /// `get_state` carries the model and window, `turn_end` the live context
    /// usage, and `get_session_stats` the cumulative cost (plus usage fallback).
    fn update_session_footer(session: &mut SessionView, ev: &Event) {
        use crate::application::session_payloads;
        match ev {
            Event::Response {
                command,
                data: Some(data),
                success: true,
                ..
            } if command == "get_state" => {
                session.footer.apply_get_state(data);
            }
            Event::Response {
                command,
                data: Some(data),
                success: true,
                ..
            } if command == "get_session_stats" => {
                let stats = session_payloads::parse_session_stats(data);
                session.footer.apply_session_stats(&stats);
            }
            Event::TurnEnd { message, .. } => {
                let used = message.get("contextTokens").and_then(|v| v.as_u64());
                let window = message
                    .get("maxContextTokens")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                if let (Some(used), Some(window)) = (used, window) {
                    session.footer.update_context_usage(used, window);
                }
            }
            _ => {}
        }
    }

    /// Replace a session's chat from a `get_messages` payload (history backfill).
    fn replace_session_chat(chat: &mut Chat, data: &serde_json::Value) {
        use crate::application::session_payloads::{self, ResumedChatMessage};
        let Ok(messages) = session_payloads::parse_resumed_messages(data) else {
            return;
        };
        chat.clear();
        for message in messages {
            match message {
                ResumedChatMessage::User(text) => chat.add_entry(ChatEntry::User { text }),
                ResumedChatMessage::Assistant(text) => chat.add_entry(ChatEntry::Assistant {
                    text,
                    streaming: false,
                }),
            }
        }
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
            depth: 0,
            label: "Master Agent".to_string(),
            status: self.master_status().to_string(),
        }];
        for (id, depth) in self.subagent_tree_order() {
            let info = self.subagent_local.get(&id).map(|t| &t.info);
            rows.push(PanelRow {
                label: id.clone(),
                status: info.map(|i| i.status.clone()).unwrap_or_default(),
                id: Some(id),
                depth,
            });
        }
        rows
    }

    /// Depth-first `(agent_id, depth)` listing of the sub-agent tree. Root
    /// sub-agents (no in-map parent) sit at depth 1 under the master; their
    /// children at depth 2, etc. Sibling order follows the sorted map keys.
    fn subagent_tree_order(&self) -> Vec<(String, usize)> {
        use std::collections::BTreeMap;
        let mut children: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
        for (id, tracked) in &self.subagent_local {
            let parent = tracked.info.parent_id.clone().filter(|p| {
                // Treat an unknown parent as a root so its subtree is not lost.
                self.subagent_local.contains_key(p)
            });
            children.entry(parent).or_default().push(id.clone());
        }
        let mut out = Vec::new();
        // Explicit stack DFS; push siblings reversed so popping preserves order.
        let mut stack: Vec<(String, usize)> = children
            .get(&None)
            .into_iter()
            .flatten()
            .rev()
            .map(|id| (id.clone(), 1))
            .collect();
        while let Some((id, depth)) = stack.pop() {
            out.push((id.clone(), depth));
            if let Some(kids) = children.get(&Some(id.clone())) {
                for kid in kids.iter().rev() {
                    stack.push((kid.clone(), depth + 1));
                }
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
            let indent = " ".repeat(row.depth * 2);
            let label = status_colored_name(&row.status, &sanitize_panel_label(&row.label));
            let elapsed = theme::dim(&self.panel_row_elapsed(row.id.as_deref(), now));
            let text = format!("{caret}{indent}{label} {elapsed}");
            let cell = if i == selected {
                pad_cell(&theme::reverse(&text), width)
            } else {
                pad_cell(&text, width)
            };
            lines.push(cell);
        }
        // A separator column on the right edge keeps the panel visually distinct
        // from the body; pad/truncate every row to exactly `width` already does
        // the alignment, so just fill the remaining height with blanks.
        while lines.len() < height {
            lines.push(" ".repeat(width));
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
    depth: usize,
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
