use super::*;
use crate::interface::theme;
const MAX_WARM_AGENT_FEEDS: usize = MAX_RETAINED_SESSIONS;

impl App {
    // ── Visibility / active session ────────────────────────────────────

    /// Whether the persistent left panel is shown. Sub-agent-first default
    /// (#820): ALWAYS on once connected — the Master is pinned as the top row
    /// even with no sub-agents, so the panel is not gated on the tree. It
    /// also survives an agent disconnect (#1047): the user keeps the session
    /// tree context needed to diagnose why the agent went away.
    pub(super) fn subagent_panel_visible(&self) -> bool {
        self.agent_ever_connected
    }

    /// The agent whose session is currently shown in the body. `None` = master.
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn active_agent_id(&self) -> Option<&str> {
        self.subagents.active_agent_id.as_deref()
    }

    /// Ids of all retained sub-agent sessions (live or exited but still
    /// viewable per the retention policy).
    #[cfg(test)]
    pub(super) fn retained_session_ids(&self) -> Vec<String> {
        self.subagents.sessions.keys().cloned().collect()
    }

    /// The socket path the connect-on-select connection would dial for `id`, as
    /// surfaced by the kernel (#800). `None` when unknown.
    #[cfg(test)]
    pub(super) fn subagent_socket_path(&self, id: &str) -> Option<String> {
        let tracked = self.subagents.tracked.get(id)?;
        tracked.info.socket_path.clone()
    }

    /// The active session, master or selected sub-agent.
    pub(super) fn active_session(&self) -> &SessionView {
        let ui = &self.subagents;
        match ui.active_agent_id.as_deref() {
            None => &self.master_session,
            // Fall back to the master session if a selected session is somehow
            // missing, mirroring `active_session_mut`'s lazy-create contract.
            Some(id) => ui.sessions.get(id).unwrap_or(&self.master_session),
        }
    }

    /// Mutable counterpart to [`active_session`]. Lazily creates the selected
    /// sub-agent's session so a selection always has a body to render; the
    /// master session always exists.
    pub(super) fn active_session_mut(&mut self) -> &mut SessionView {
        let Some(id) = self.subagents.active_agent_id.clone() else {
            return &mut self.master_session;
        };
        if !self.subagents.sessions.contains_key(&id) {
            // Cold path only: clone git_branch and build the session here, so the
            // common already-exists render path allocates nothing extra (#827 perf).
            let git_branch = self.git_branch.clone();
            Self::remember_session(&mut self.subagents.session_order, &id);
            self.subagents
                .sessions
                .insert(id.clone(), SessionView::new(git_branch));
        }
        self.subagents.sessions.get_mut(&id).unwrap()
    }

    /// The chat buffer for the active session (master or selected sub-agent).
    pub(super) fn active_chat_mut(&mut self) -> &mut Chat {
        &mut self.active_session_mut().chat
    }

    /// Test-only: number of chat entries in a sub-agent session (for asserting
    /// the deferred-note buffer cap independently of the rendered viewport).
    #[cfg(test)]
    pub(crate) fn session_chat_entry_count(&self, agent_id: &str) -> Option<usize> {
        let session = self.subagents.sessions.get(agent_id)?;
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
        // No stream state observed yet: connect-on-select may have joined
        // MID-TURN and missed `agent_start`, so `session.running` reads a false
        // negative. Fall back to the master's tracked status (`subagent_local`)
        // so Esc still cancels a busy sub-agent instead of navigating to master.
        let ui = &self.subagents;
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
        let Some(id) = self.subagents.active_agent_id.clone() else {
            return false;
        };
        let Some(feed) = self.subagents.feeds.get(&id) else {
            return false;
        };
        // Exact active-agent-id lookup is the command-routing guard: callers
        // cannot accidentally route through a stale non-active feed entry.
        feed.cmd_tx.try_send(cmd).is_ok()
    }

    pub(super) fn select_agent(&mut self, agent_id: Option<&str>) {
        let new_active = agent_id.map(str::to_string);
        if new_active == self.subagents.active_agent_id {
            return;
        }
        let old_active = self.subagents.active_agent_id.clone();
        self.subagents.active_agent_id = new_active.clone();
        self.sync_panel_selection_to_active();
        let Some(id) = new_active else {
            // Restore model/effort markers from master footer (#1085).
            self.current_model = self.master_session.footer.known_model().map(str::to_string);
            self.current_effort = self.master_session.footer.effort().map(str::to_string);
            self.effort_levels.clear();
            if let Some(old) = old_active {
                let drop_legacy = self.is_legacy_selected_feed(&old);
                if drop_legacy {
                    if let Some(feed) = self.subagents.feeds.remove(&old) {
                        feed.handle.abort();
                    }
                }
            }
            self.send_state_resync();
            return;
        };
        self.ensure_session(&id);
        let f = self.subagents.sessions.get(&id).map(|s| &s.footer);
        self.current_model = f.and_then(|f| f.known_model()).map(str::to_string);
        self.current_effort = f.and_then(|f| f.effort()).map(str::to_string);
        self.effort_levels.clear();
        self.seed_session_bar_from_snapshot(&id);
        self.upgrade_warm_feed_for_selection(&id);
        self.refresh_synced_feed_for_focus(&id);
        if !self.subagents.feeds.contains_key(&id) {
            self.open_subagent_connection(&id);
        }
        if let Some(old) = old_active {
            let drop_legacy = self.is_legacy_selected_feed(&old);
            if drop_legacy {
                if let Some(feed) = self.subagents.feeds.remove(&old) {
                    feed.handle.abort();
                }
            }
        }
    }

    fn reconcile_active_agent(&mut self) {
        if let Some(active) = self.subagents.active_agent_id.clone() {
            if !self.subagents.tracked.contains_key(&active) {
                self.select_agent(None);
            }
        }
    }

    /// Create the session for `id` if missing, recording retention order and
    /// evicting the oldest non-active session beyond the cap.
    pub(super) fn ensure_session(&mut self, id: &str) {
        if !self.subagents.sessions.contains_key(id) {
            self.subagents
                .sessions
                .insert(id.to_string(), SessionView::new(self.git_branch.clone()));
            Self::remember_session(&mut self.subagents.session_order, id);
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
        let subs = &mut self.subagents;
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

    fn upgrade_warm_feed_for_selection(&mut self, id: &str) {
        let warm_unsynced = self.subagents.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::interface::feed_state::FeedAuthority::WarmSync
                && !feed.supports_sync
        });
        if warm_unsynced {
            if let Some(feed) = self.subagents.feeds.remove(id) {
                feed.handle.abort();
            }
        }
    }

    fn refresh_synced_feed_for_focus(&mut self, id: &str) {
        let stale = self.subagents.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::interface::feed_state::FeedAuthority::SyncedAuthoritative
                && feed
                    .last_fresh_at
                    .is_none_or(|fresh| fresh.elapsed().as_secs() > 0)
        });
        if stale {
            if let Some(feed) = self.subagents.feeds.get_mut(id) {
                let _ = feed.cmd_tx.try_send(Command::Sync {
                    id: Some("subagent-sync".into()),
                    epoch: feed.epoch,
                    since_rev: feed.rev,
                });
            }
        }
    }

    pub(super) fn is_legacy_selected_feed(&self, id: &str) -> bool {
        self.subagents.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::interface::feed_state::FeedAuthority::LegacySelected
        })
    }

    pub(super) fn is_synced_authoritative_feed(&self, id: &str) -> bool {
        self.subagents.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::interface::feed_state::FeedAuthority::SyncedAuthoritative
        })
    }

    pub(super) fn enforce_warm_feed_cap(&mut self) {
        while self.subagents.feeds.len() > MAX_WARM_AGENT_FEEDS {
            let Some(victim) = self
                .subagents
                .session_order
                .iter()
                .filter(|id| Some(*id) != self.subagents.active_agent_id.as_ref())
                .find(|id| self.subagents.feeds.contains_key(id.as_str()))
                .cloned()
            else {
                break;
            };
            if let Some(feed) = self.subagents.feeds.remove(&victim) {
                feed.handle.abort();
            }
        }
    }

    // ── Panel keyboard navigation (#802 focus model) ──────────────────

    /// Move the panel highlight up (toward the master) WITHOUT switching the
    /// active session — commit happens only on Enter/Tab (#802).
    pub(super) fn panel_highlight_previous(&mut self) {
        let len = self.panel_rows().len();
        self.subagents.panel_nav.move_previous(len);
    }

    /// Move the panel highlight down WITHOUT switching the active session.
    pub(super) fn panel_highlight_next(&mut self) {
        let len = self.panel_rows().len();
        self.subagents.panel_nav.move_next(len);
    }

    /// Jump the panel highlight to a 1-based row number (digits 1–9). Row 1 is
    /// the master; row N+1 is the Nth listed sub-agent. No-op past the end.
    pub(super) fn panel_highlight_row(&mut self, one_based: usize) {
        let len = self.panel_rows().len();
        if one_based >= 1 && one_based <= len {
            self.subagents.panel_nav.set_selected(one_based - 1);
        }
    }

    /// Commit the highlighted panel row: switch the active session to that agent
    /// (the master when row 1 is highlighted) and open its connection (#802).
    pub(super) fn commit_panel_selection(&mut self) {
        let target = self
            .panel_rows()
            .get(self.subagents.panel_nav.selected())
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
        self.subagents.panel_nav.clamp(rows.len());
        self.sync_panel_selection_to_active_with(&rows);
    }

    pub(super) fn sync_panel_selection_to_active(&mut self) {
        let rows = self.panel_rows();
        self.sync_panel_selection_to_active_with(&rows);
    }

    fn sync_panel_selection_to_active_with(&mut self, rows: &[PanelRow]) {
        if let Some(idx) = rows
            .iter()
            .position(|r| r.id.as_deref() == self.subagents.active_agent_id.as_deref())
        {
            self.subagents.panel_nav.set_selected(idx);
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
        let master_wf = {
            let wf = &self.master_session.workflow_bar;
            (wf.total > 0).then_some((wf.done, wf.total))
        };
        let mut rows = vec![PanelRow {
            id: None,
            prefix: String::new(),
            label: "Master Agent".to_string(),
            status: self.master_status().to_string(),
            workflow: master_wf,
        }];
        for (id, prefix) in self.subagent_tree_order() {
            let info = self.subagents.tracked.get(&id).map(|t| &t.info);
            let workflow = info
                .and_then(|i| i.workflow.as_ref())
                .filter(|w| w.steps_total > 0)
                .map(|w| (w.steps_completed, w.steps_total));
            rows.push(PanelRow {
                label: id.clone(),
                status: info.map(|i| i.status.clone()).unwrap_or_default(),
                workflow,
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
        for (id, tracked) in &self.subagents.tracked {
            // Treat an unknown parent as a root so its subtree is not lost.
            let parent = tracked
                .info
                .parent_id
                .clone()
                .filter(|p| self.subagents.tracked.contains_key(p));
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

    pub(super) fn render_subagent_panel(
        &self,
        width: usize,
        height: usize,
        now: tokio::time::Instant,
    ) -> Vec<String> {
        let rows = self.panel_rows();
        let selected = self.subagents.panel_nav.selected();
        let active = self.subagents.active_agent_id.as_deref();
        let focused = matches!(self.subagents.focus, Focus::Panel);

        let blocks: Vec<Vec<String>> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let sel = i == selected;
                let is_active = row.id.as_deref() == active;
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
        use crate::interface::utils::{truncate_to_width, visible_width};
        // `show_bar` = selected AND panel-focused: the ▌ bar is the panel's
        // cursor, mirroring the editor's cursor which hides when focus is here.
        let selbar = if show_bar {
            theme::accent("▌")
        } else {
            " ".to_string()
        };
        let stalk_vis = visible_width(&row.prefix);
        let timer = self.panel_row_timer(row.id.as_deref(), now);
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
        let entry = self.subagents.tracked.get(id)?;
        if entry.info.read_only {
            Some(theme::OBSERVER_MARKER)
        } else {
            None
        }
    }

    fn panel_row_timer(&self, id: Option<&str>, now: tokio::time::Instant) -> String {
        match id {
            None => fmt_mss(now.saturating_duration_since(self.started_at).as_secs()),
            Some(id) => {
                let t = self.subagents.tracked.get(id);
                t.map(|t| fmt_mss(t.elapsed_secs(now))).unwrap_or_default()
            }
        }
    }

    /// The per-row elapsed label for the panel (#820): the Master row shows the
    /// session uptime; a sub-agent row shows its running/idle/frozen timer.
    pub(super) fn panel_row_elapsed(&self, id: Option<&str>, now: tokio::time::Instant) -> String {
        let Some(id) = id else {
            // Master row → session uptime.
            return fmt_mss(now.saturating_duration_since(self.started_at).as_secs());
        };
        let Some(t) = self.subagents.tracked.get(id) else {
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

    // ── Main-pane workflow indicator (selected agent) ──────────────────

    /// The main pane's top chrome for the selected agent (#820): a title line
    /// (`agent · status · elapsed · #issue workflow`) followed by the EXISTING
    /// yellow workflow bar rendered BOXED as a single content line
    /// (`progress-bar PHASE n/total`) — dropping the phase-pills and hints lines.
    /// Title always renders; the boxed bar is appended only when a workflow exists.
    pub(super) fn render_main_pane_workflow(
        &self,
        width: usize,
        box_width: usize,
        now: tokio::time::Instant,
    ) -> Vec<String> {
        if width < 4 {
            return Vec::new();
        }
        let state = self.active_workflow_bar();
        // Title ALWAYS renders; the boxed bar is conditional on a workflow (#820).
        let mut out = vec![pad_cell(&self.main_pane_title(state, now), width)];
        if let Some(content) = workflow_bar::render_compact_line(state) {
            let inner = box_width.saturating_sub(2);
            out.push(theme::dim(&"─".repeat(box_width)));
            out.push(crate::interface::utils::truncate_to_width(
                &format!(" {} ", boxed_inner(&content, inner)),
                box_width,
                None,
            ));
            out.push(theme::dim(&"─".repeat(box_width)));
        }
        out
    }

    /// Build the main-pane title line for the active agent (#820).
    fn main_pane_title(
        &self,
        state: &workflow_bar::WorkflowBarState,
        now: tokio::time::Instant,
    ) -> String {
        let (name, status) = match self.subagents.active_agent_id.as_deref() {
            None => ("Master".to_string(), self.master_status().to_string()),
            Some(id) => (
                id.to_string(),
                self.subagents
                    .tracked
                    .get(id)
                    .map(|t| t.info.status.clone())
                    .unwrap_or_default(),
            ),
        };
        let elapsed = self.panel_row_elapsed(self.subagents.active_agent_id.as_deref(), now);
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

/// Per-step workflow bar beneath an agent's name (`▰` done · `▱` pending), one
/// cell per step up to `MAX_CELLS`, else proportional. Column 0 is always blank
/// so the selection (`▌`) stays one line tall; the tree stalk continues down
/// through the bar via the agent's continuation prefix.
fn panel_bar_line(prefix: &str, done: u32, total: u32, width: usize) -> String {
    use crate::interface::utils::visible_width;
    const MAX_CELLS: usize = 20;
    let cont = bar_continuation(prefix);
    let cont_vis = visible_width(&cont);
    // Reserve column 0 (blank) + a 1-col right gutter, mirroring the name row (#875).
    let avail = width.saturating_sub(2 + cont_vis);
    let cells = (total as usize).min(MAX_CELLS).min(avail).max(1);
    let filled = ((done as usize) * cells / (total.max(1) as usize)).min(cells);
    let bar = format!(
        "{}{}",
        theme::accent(&"▰".repeat(filled)),
        theme::dim(&"▱".repeat(cells - filled)),
    );
    // pad_cell adds the trailing gutter and clamps any overshoot to exactly width.
    pad_cell(&format!(" {}{bar}", theme::dim(&cont)), width)
}

/// The tree prefix for an agent's bar line: its own connector becomes a vertical
/// (`├ `→`│ `) or blank (`└ `→`  `) so the stalk flows down past the bar to the
/// agent's following siblings/children.
fn bar_continuation(prefix: &str) -> String {
    if let Some(head) = prefix.strip_suffix("├ ") {
        format!("{head}│ ")
    } else if let Some(head) = prefix.strip_suffix("└ ") {
        format!("{head}  ")
    } else {
        prefix.to_string()
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
    /// `(steps_completed, steps_total)` when the agent has an active workflow —
    /// drives the per-step progress bar drawn beneath the name row.
    workflow: Option<(u32, u32)>,
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
    crate::interface::ansi::sanitize_control(s)
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
