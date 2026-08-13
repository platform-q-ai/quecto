use super::*;
use crate::components::theme;

#[path = "controller_subagent_panel_helpers.rs"]
pub(crate) mod controller_subagent_panel_helpers;
use controller_subagent_panel_helpers::{
    fmt_mss, pad_cell, panel_bar_line, sanitize_panel_label, status_colored_name,
};

use super::app_subagent_panel_rows::PanelRow;

const MAX_WARM_AGENT_FEEDS: usize = MAX_RETAINED_SESSIONS;

impl App {
    // ── Visibility / active session ────────────────────────────────────

    /// Whether the persistent left panel is shown. Sub-agent-first default
    /// (#820): ALWAYS on once connected — the Master is pinned as the top row
    /// even with no sub-agents, so the panel is not gated on the tree. It
    /// also survives an agent disconnect (#1047): the user keeps the session
    /// tree context needed to diagnose why the agent went away.
    pub(super) fn subagent_panel_visible(&self) -> bool {
        self.ac().agent_ever_connected
    }

    /// The agent whose session is currently shown in the body. `None` = master.
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn active_agent_id(&self) -> Option<&str> {
        self.ac().roster.active_agent_id.as_deref()
    }

    /// Ids of all retained sub-agent sessions (live or exited but still
    /// viewable per the retention policy).
    #[cfg(test)]
    pub(super) fn retained_session_ids(&self) -> Vec<String> {
        self.ac().roster.sessions.keys().cloned().collect()
    }

    /// The socket path the child-feed connection would dial for `id`, as
    /// surfaced by the kernel (#800). `None` when unknown.
    #[cfg(test)]
    pub(super) fn subagent_socket_path(&self, id: &str) -> Option<String> {
        let tracked = self.ac().roster.tracked.get(id)?;
        tracked.info.socket_path.clone()
    }

    /// The active session, master or selected sub-agent.
    pub(super) fn active_session(&self) -> &SessionView {
        let ui = &self.ac().roster;
        match ui.active_agent_id.as_deref() {
            None => &self.ac().master_session,
            // Fall back to the master session if a selected session is somehow
            // missing, mirroring `active_session_mut`'s lazy-create contract.
            Some(id) => ui.sessions.get(id).unwrap_or(&self.ac().master_session),
        }
    }

    /// Mutable counterpart to [`active_session`]. Lazily creates the selected
    /// sub-agent's session so a selection always has a body to render; the
    /// master session always exists.
    pub(super) fn active_session_mut(&mut self) -> &mut SessionView {
        let Some(id) = self.ac().roster.active_agent_id.clone() else {
            return &mut self.ac_mut().master_session;
        };
        if !self.ac().roster.sessions.contains_key(&id) {
            // Cold path only: clone git_branch and build the session here, so the
            // common already-exists render path allocates nothing extra (#827 perf).
            let git_branch = self.workspace.git_branch.clone();
            Self::remember_session(&mut self.ac_mut().roster.session_order, &id);
            self.ac_mut()
                .roster
                .sessions
                .insert(id.clone(), SessionView::new(git_branch));
        }
        self.ac_mut().roster.sessions.get_mut(&id).unwrap()
    }

    /// The chat buffer for the active session (master or selected sub-agent).
    pub(super) fn active_chat_mut(&mut self) -> &mut Chat {
        &mut self.active_session_mut().chat
    }

    /// Test-only: number of chat entries in a sub-agent session (for asserting
    /// the deferred-note buffer cap independently of the rendered viewport).
    #[cfg(test)]
    pub(crate) fn session_chat_entry_count(&self, agent_id: &str) -> Option<usize> {
        let session = self.ac().roster.sessions.get(agent_id)?;
        Some(session.chat.entry_count())
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
        // No stream state observed yet: the child feed may have joined MID-TURN
        // and missed `agent_start`, so `session.running` reads a false
        // negative. Fall back to the active tab's tracked status (`subagent_local`)
        // so Esc still cancels a busy sub-agent instead of navigating to master.
        let ui = &self.ac().roster;
        match &ui.active_agent_id {
            Some(id) => ui
                .tracked
                .get(id)
                .is_some_and(|t| subagent_status_is_active(&t.info.status)),
            None => false,
        }
    }

    /// The focus region currently holding keyboard input (#802, tests).
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn focus_region(&self) -> Focus {
        self.subagents.focus
    }

    /// The 0-based panel highlight index (#802, tests).
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn panel_highlight_index(&self) -> usize {
        self.subagents.panel_nav.selected()
    }

    /// Send a command to the active sub-agent connection.
    pub(super) fn send_to_active_subagent(&mut self, cmd: Command) -> bool {
        let Some(id) = self.ac().roster.active_agent_id.clone() else {
            return false;
        };
        let Some(feed) = self.ac().roster.feeds.get(&id) else {
            return false;
        };
        if feed.inspection_only
            && !crate::protocol::inspection_routing::is_inspection_routable(&cmd)
        {
            return false;
        }
        // Exact active-agent-id lookup is the command-routing guard: callers
        // cannot accidentally route through a stale non-active feed entry.
        feed.cmd_tx.try_send(cmd).is_ok()
    }

    pub(super) fn select_agent(&mut self, agent_id: Option<&str>) {
        // Focusing a concrete agent always dismisses environment-detail chrome
        // (#1369 slice 4); selecting the master keeps it (environment rows
        // render their details over the master body).
        if agent_id.is_some() {
            self.ac_mut().roster.selected_environment = None;
        }
        let new_active = agent_id.map(str::to_string);
        self.close_tab_switch_overlays();
        if new_active == self.ac().roster.active_agent_id {
            return;
        }
        self.ac_mut().roster.active_agent_id = new_active.clone();
        self.subagents.panel_nav_key = new_active
            .as_deref()
            .map(|id| format!("agent:{id}"))
            .or_else(|| Some("master".to_string()));
        self.sync_panel_selection_to_active();
        let Some(id) = new_active else {
            // Restore model/effort markers from master footer (#1085).
            self.ac_mut().inference.current_model = self
                .ac()
                .master_session
                .footer
                .known_model()
                .map(str::to_string);
            self.ac_mut().inference.current_effort =
                self.ac().master_session.footer.effort().map(str::to_string);
            self.ac_mut().inference.effort_levels.clear();
            self.send_state_resync();
            return;
        };
        self.ensure_session(&id);
        let (model, effort) = self
            .ac()
            .roster
            .sessions
            .get(&id)
            .map(|s| {
                (
                    s.footer.known_model().map(str::to_string),
                    s.footer.effort().map(str::to_string),
                )
            })
            .unwrap_or((None, None));
        self.ac_mut().inference.current_model = model;
        self.ac_mut().inference.current_effort = effort;
        self.ac_mut().inference.effort_levels.clear();
        self.seed_session_bar_from_snapshot(&id);
        self.ensure_synced_subagent_feed(&id);
        // Merge committed ledger + retained in-flight live tail so focusing a
        // busy child mid-turn shows the full transcript so far (#1259).
        self.reproject_child_chat_with_live(&id);
        self.refresh_synced_feed_for_focus(&id);
    }

    /// Project the child's session chat from its authoritative ledger transcript
    /// plus any retained in-flight live buffer (#1259).
    pub(super) fn reproject_child_chat_with_live(&mut self, id: &str) {
        let Some(entries) = self
            .ac()
            .roster
            .feeds
            .get(id)
            .filter(|f| f.authority == crate::agents::feed::FeedAuthority::SyncedAuthoritative)
            .map(|f| f.transcript.entries())
        else {
            return;
        };
        let Some(session) = self.ac_mut().roster.sessions.get_mut(id) else {
            return;
        };
        // Focus always attaches the retained buffer; never clear it here.
        session.project_ledger_with_live(entries, true, false);
    }

    fn reconcile_active_agent(&mut self) {
        if let Some(active) = self.ac().roster.active_agent_id.clone() {
            if !self.ac().roster.tracked.contains_key(&active) {
                self.select_agent(None);
            }
        }
    }

    /// Create the session for `id` if missing, recording retention order and
    /// evicting the oldest non-active session beyond the cap.
    pub(super) fn ensure_session(&mut self, id: &str) {
        if !self.ac().roster.sessions.contains_key(id) {
            let git_branch = self.workspace.git_branch.clone();
            self.ac_mut()
                .roster
                .sessions
                .insert(id.to_string(), SessionView::new(git_branch));
            Self::remember_session(&mut self.ac_mut().roster.session_order, id);
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
        let subs = &mut self.ac_mut().roster;
        while subs.sessions.len() > MAX_RETAINED_SESSIONS {
            let order = &subs.session_order;
            let Some(pos) = order
                .iter()
                .position(|id| Some(id) != subs.active_agent_id.as_ref())
            else {
                break;
            };
            let victim = subs.session_order.remove(pos);
            subs.sessions.remove(&victim);
            if let Some(feed) = subs.feeds.remove(&victim) {
                feed.handle.abort();
            }
        }
    }

    fn refresh_synced_feed_for_focus(&mut self, id: &str) {
        let stale = self.ac().roster.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::agents::feed::FeedAuthority::SyncedAuthoritative
                && feed
                    .last_fresh_at
                    .is_none_or(|fresh| fresh.elapsed().as_secs() > 0)
        });
        if stale {
            let sync_id = self.ac().namespaced_id("subagent-sync");
            if let Some(feed) = self.ac_mut().roster.feeds.get_mut(id) {
                let _ = feed.cmd_tx.try_send(Command::Sync {
                    agent_id: None,
                    id: Some(sync_id),
                    epoch: feed.epoch,
                    since_rev: feed.rev,
                });
            }
        }
    }

    pub(super) fn is_synced_authoritative_feed(&self, id: &str) -> bool {
        self.ac().roster.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::agents::feed::FeedAuthority::SyncedAuthoritative
        })
    }

    /// Whether this feed should retain a mid-turn live buffer (#1259).
    /// Includes warm-sync feeds before the first authoritative sync response so
    /// connect races do not drop the in-flight prefix.
    pub(super) fn retains_live_inflight_feed(&self, id: &str) -> bool {
        self.ac().roster.feeds.get(id).is_some_and(|feed| {
            feed.supports_sync
                || matches!(
                    feed.authority,
                    crate::agents::feed::FeedAuthority::WarmSync
                        | crate::agents::feed::FeedAuthority::SyncedAuthoritative
                )
        })
    }

    pub(super) fn enforce_warm_feed_cap(&mut self) {
        while self.ac().roster.feeds.len() > MAX_WARM_AGENT_FEEDS {
            let Some(victim) = self
                .ac()
                .roster
                .session_order
                .iter()
                .filter(|id| Some(*id) != self.ac().roster.active_agent_id.as_ref())
                .find(|id| self.ac().roster.feeds.contains_key(id.as_str()))
                .cloned()
            else {
                break;
            };
            if let Some(feed) = self.ac_mut().roster.feeds.remove(&victim) {
                feed.handle.abort();
            }
        }
    }

    // ── Panel keyboard navigation (#802 focus model) ──────────────────

    fn panel_row_key(row: &PanelRow) -> String {
        if let Some(env_key) = row.env_key.as_deref() {
            format!("env:{env_key}")
        } else if let Some(id) = row.id.as_deref() {
            format!("agent:{id}")
        } else {
            "master".to_string()
        }
    }

    fn remember_panel_nav_key_from_rows(&mut self, rows: &[PanelRow]) {
        self.subagents.panel_nav_key = rows
            .get(self.subagents.panel_nav.selected())
            .map(Self::panel_row_key);
    }

    /// Move the panel highlight up (toward the master) WITHOUT switching the
    /// active session — commit happens only on Enter/Tab (#802).
    pub(super) fn panel_highlight_previous(&mut self) {
        let rows = self.panel_rows();
        self.subagents.panel_nav.move_previous(rows.len());
        self.remember_panel_nav_key_from_rows(&rows);
    }

    /// Move the panel highlight down WITHOUT switching the active session.
    pub(super) fn panel_highlight_next(&mut self) {
        let rows = self.panel_rows();
        self.subagents.panel_nav.move_next(rows.len());
        self.remember_panel_nav_key_from_rows(&rows);
    }

    /// Move the panel highlight down by multiple rows (mouse wheel/page-like
    /// panel scrolling) without switching the active session. Unlike arrow-key
    /// navigation, scroll/page input is clamped at the list edge so a wheel tick
    /// at the bottom never wraps back to Master.
    pub(super) fn panel_highlight_next_by(&mut self, rows: usize) {
        let len = self.panel_rows().len();
        if len == 0 {
            self.subagents.panel_nav.set_selected(0);
            return;
        }
        let selected = self.subagents.panel_nav.selected();
        self.subagents
            .panel_nav
            .set_selected(selected.saturating_add(rows).min(len - 1));
        let rows = self.panel_rows();
        self.remember_panel_nav_key_from_rows(&rows);
    }

    /// Move the panel highlight up by multiple rows (mouse wheel/page-like
    /// panel scrolling) without switching the active session. Unlike arrow-key
    /// navigation, scroll/page input is clamped at the list edge so a wheel tick
    /// at the top never wraps to the bottom.
    pub(super) fn panel_highlight_previous_by(&mut self, rows: usize) {
        let len = self.panel_rows().len();
        if len == 0 {
            self.subagents.panel_nav.set_selected(0);
            return;
        }
        let selected = self.subagents.panel_nav.selected();
        self.subagents
            .panel_nav
            .set_selected(selected.saturating_sub(rows));
        let rows = self.panel_rows();
        self.remember_panel_nav_key_from_rows(&rows);
    }

    /// Jump the panel highlight to a 1-based row number (digits 1–9). Row 1 is
    /// the master; row N+1 is the Nth listed sub-agent. No-op past the end.
    pub(super) fn panel_highlight_row(&mut self, one_based: usize) {
        let len = self.panel_rows().len();
        if one_based >= 1 && one_based <= len {
            self.subagents.panel_nav.set_selected(one_based - 1);
            let rows = self.panel_rows();
            self.remember_panel_nav_key_from_rows(&rows);
        }
    }

    /// Commit the highlighted panel row: switch the active session to that agent
    /// (the master when row 1 is highlighted) and open its connection (#802).
    pub(super) fn commit_panel_selection(&mut self) {
        let rows = self.panel_rows();
        let Some(row) = rows.get(self.subagents.panel_nav.selected()) else {
            return;
        };
        if row.is_environment() {
            // Selecting an environment row shows its details in the main-pane
            // chrome over the master body (#1369 slice 4).
            self.ac_mut().roster.selected_environment = row.env_key.clone();
            self.subagents.panel_nav_key = row.env_key.as_deref().map(|key| format!("env:{key}"));
            self.select_agent(None);
            return;
        }
        let target = row.id.clone();
        self.ac_mut().roster.selected_environment = None;
        self.subagents.panel_nav_key = target
            .as_deref()
            .map(|id| format!("agent:{id}"))
            .or_else(|| Some("master".to_string()));
        self.select_agent(target.as_deref());
    }

    /// Keep the panel cursor pointing at the active agent (or the master row)
    /// after the underlying list changes.
    pub(super) fn clamp_panel_selection(&mut self) {
        // If the active agent dropped out of the live list, fall back to the
        // master before reconciling the cursor (#800 review).
        self.reconcile_active_agent();
        let rows = self.panel_rows();
        self.subagents.panel_nav.clamp(rows.len());
        self.sync_panel_selection_to_active_with(&rows);
    }

    pub(super) fn sync_panel_selection_to_active(&mut self) {
        let rows = self.panel_rows();
        self.sync_panel_selection_to_active_with(&rows);
    }

    fn sync_panel_selection_to_active_with(&mut self, rows: &[PanelRow]) {
        if matches!(self.subagents.focus, Focus::Panel) {
            // While the user is navigating the panel, cursor identity owns the
            // highlight. A committed environment still owns main-pane chrome,
            // but it must not snap the focused panel away from the user's row.
            // This also lets committing Master overwrite a stale env key even
            // when the active session was already Master.
            if let Some(key) = self.subagents.panel_nav_key.as_deref() {
                if let Some(idx) = rows.iter().position(|r| Self::panel_row_key(r) == key) {
                    self.subagents.panel_nav.set_selected(idx);
                    return;
                }
            }
            self.subagents.panel_nav.clamp(rows.len());
            self.remember_panel_nav_key_from_rows(rows);
            return;
        }
        // A committed environment selection owns the cursor: with an
        // environment selected `active_agent_id` is `None`, which would
        // otherwise match the Master row and silently snap the cursor away
        // from the environment whose chrome is showing (review #1392).
        if let Some(env_key) = self.ac().roster.selected_environment.as_deref() {
            if let Some(idx) = rows
                .iter()
                .position(|r| r.env_key.as_deref() == Some(env_key))
            {
                self.subagents.panel_nav.set_selected(idx);
                return;
            }
            // The group dissolved (member exited/refreshed away): drop the
            // stale selection so the chrome and cursor fall back together.
            self.ac_mut().roster.selected_environment = None;
        }
        if let Some(idx) = rows
            .iter()
            .position(|r| r.id.as_deref() == self.ac().roster.active_agent_id.as_deref())
        {
            self.subagents.panel_nav.set_selected(idx);
        }
    }

    // ── Panel rendering ────────────────────────────────────────────────

    /// Flattened panel rows: the master pinned at the top, then the sub-agent
    /// tree depth-ordered by `parent_id` (grandchildren under their parent).
    /// Master's live status: `running` while processing, else `idle` (#820).
    pub(super) fn master_status_for(conn: &connection_state::ConnectionState) -> &'static str {
        if conn.agent_state.is_running() {
            "running"
        } else {
            "idle"
        }
    }

    pub(super) fn render_subagent_panel(
        &self,
        width: usize,
        height: usize,
        now: tokio::time::Instant,
    ) -> Vec<String> {
        let rows = self.panel_rows();
        let selected = self.subagents.panel_nav.selected();
        let active = self.ac().roster.active_agent_id.as_deref();
        let focused = matches!(self.subagents.focus, Focus::Panel);

        let blocks: Vec<Vec<String>> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let sel = i == selected;
                // Environment rows share `id: None` with the master row; they
                // are never the active agent (#1369 slice 4).
                let is_active = row.id.as_deref() == active && !row.is_environment();
                let mut block =
                    vec![self.panel_name_line(row, sel && focused, is_active, width, now)];
                if let Some((done, total)) = row.workflow {
                    block.push(panel_bar_line(&row.prefix, done, total, width));
                }
                block
            })
            .collect();

        // Viewport: scroll so the selected agent stays visible (variable heights).
        let content = height.saturating_sub(1); // footer only; no header
        let mut start = 0usize;
        if selected < blocks.len() {
            while start < selected
                && blocks[start..=selected].iter().map(Vec::len).sum::<usize>() > content
            {
                start += 1;
            }
        }

        let mut lines: Vec<String> = Vec::with_capacity(height);
        let mut used = 0usize;
        for block in &blocks[start.min(blocks.len())..] {
            if used + block.len() > content {
                break;
            }
            lines.extend(block.iter().cloned());
            used += block.len();
        }
        let hint = pad_cell(&theme::dim("⇥ pane  ↑↓ move  ⏎ open"), width);
        while lines.len() + 1 < height {
            lines.push(" ".repeat(width));
        }
        if lines.len() < height {
            lines.push(hint);
        }
        lines.truncate(height);
        lines
    }

    /// The agent name row: `<sel-bar><stalk><name>…<timer>`; read-only
    /// sub-agents show the observer marker after the name (#966).
    fn panel_name_line(
        &self,
        row: &PanelRow,
        show_bar: bool,
        active: bool,
        width: usize,
        now: tokio::time::Instant,
    ) -> String {
        use crate::components::utils::{truncate_to_width, visible_width};
        // `show_bar` = selected AND panel-focused: the ▌ bar is the panel's
        // cursor, mirroring the editor's cursor which hides when focus is here.
        let selbar = if show_bar {
            theme::accent("▌")
        } else {
            " ".to_string()
        };
        let stalk_vis = visible_width(&row.prefix);
        // Environment rows have no per-agent timer; `id: None` must not fall
        // back to the master uptime (#1369 slice 4).
        let timer = if row.is_environment() {
            String::new()
        } else {
            self.panel_row_timer(row.id.as_deref(), now)
        };
        let observer = self.panel_row_observer(row.id.as_deref()).unwrap_or("");
        let observer_vis = visible_width(observer);
        let usable = width.saturating_sub(1);
        let name_avail = usable.saturating_sub(1 + stalk_vis + 1 + observer_vis + timer.len());
        let name = truncate_to_width(&sanitize_panel_label(&row.label), name_avail, Some("…"));
        let name_vis = visible_width(&name);
        let mut name = status_colored_name(&row.status, &name);
        if active {
            name = theme::bold(&name);
        }
        let pad = usable.saturating_sub(1 + stalk_vis + name_vis + observer_vis + timer.len());
        let line = format!(
            "{selbar}{}{name}{observer}{}{} ",
            theme::dim(&row.prefix),
            " ".repeat(pad),
            theme::dim(&timer),
        );
        pad_cell(&line, width)
    }

    fn panel_row_observer(&self, id: Option<&str>) -> Option<&'static str> {
        let id = id?;
        let entry = self.ac().roster.tracked.get(id)?;
        if entry.info.read_only {
            Some(theme::OBSERVER_MARKER)
        } else {
            None
        }
    }

    fn panel_row_timer(&self, id: Option<&str>, now: tokio::time::Instant) -> String {
        match id {
            None => fmt_mss(
                now.saturating_duration_since(self.ac().started_at)
                    .as_secs(),
            ),
            Some(id) => {
                let t = self.ac().roster.tracked.get(id);
                t.map(|t| fmt_mss(t.elapsed_secs(now))).unwrap_or_default()
            }
        }
    }

    /// The per-row elapsed label for the panel (#820): the Master row shows the
    /// session uptime; a sub-agent row shows its running/idle/frozen timer.
    pub(super) fn panel_row_elapsed(&self, id: Option<&str>, now: tokio::time::Instant) -> String {
        let Some(id) = id else {
            // Master row → session uptime.
            return fmt_mss(
                now.saturating_duration_since(self.ac().started_at)
                    .as_secs(),
            );
        };
        let Some(t) = self.ac().roster.tracked.get(id) else {
            return String::new();
        };
        // Per-row timer (#838): non-running agents show a FROZEN value
        // (`elapsed_secs` is `now`-independent once `stopped_at` is set).
        let mss = fmt_mss(t.elapsed_secs(now));
        if t.info.status == "idle" {
            // `elapsed_secs` is the frozen *run* duration (start→stopped_at), not a
            // time-since-idle clock, so label it as such (`ran`) to avoid reading
            // `idle 5:00` as "idle for 5:00" when it means "ran 5:00" (#838 review).
            format!("idle (ran {mss})")
        } else {
            mss
        }
    }
}
