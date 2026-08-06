use super::*;
use crate::components::theme;

#[path = "controller_subagent_panel_helpers.rs"]
pub(crate) mod controller_subagent_panel_helpers;
use controller_subagent_panel_helpers::{
    fmt_mss, pad_cell, panel_bar_line, sanitize_panel_label, status_colored_name,
};

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

    /// The socket path the child-feed connection would dial for `id`, as
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
            let git_branch = self.workspace.git_branch.clone();
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
        // No stream state observed yet: the child feed may have joined MID-TURN
        // and missed `agent_start`, so `session.running` reads a false
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
        // Focusing a concrete agent always dismisses environment-detail chrome
        // (#1369 slice 4); selecting the master keeps it (environment rows
        // render their details over the master body).
        if agent_id.is_some() {
            self.subagents.selected_environment = None;
        }
        let new_active = agent_id.map(str::to_string);
        if new_active == self.subagents.active_agent_id {
            return;
        }
        self.subagents.active_agent_id = new_active.clone();
        self.sync_panel_selection_to_active();
        let Some(id) = new_active else {
            // Restore model/effort markers from master footer (#1085).
            self.inference.current_model =
                self.master_session.footer.known_model().map(str::to_string);
            self.inference.current_effort = self.master_session.footer.effort().map(str::to_string);
            self.inference.effort_levels.clear();
            self.send_state_resync();
            return;
        };
        self.ensure_session(&id);
        let f = self.subagents.sessions.get(&id).map(|s| &s.footer);
        self.inference.current_model = f.and_then(|f| f.known_model()).map(str::to_string);
        self.inference.current_effort = f.and_then(|f| f.effort()).map(str::to_string);
        self.inference.effort_levels.clear();
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
            .subagents
            .feeds
            .get(id)
            .filter(|f| f.authority == crate::agents::feed::FeedAuthority::SyncedAuthoritative)
            .map(|f| f.transcript.entries())
        else {
            return;
        };
        let Some(session) = self.subagents.sessions.get_mut(id) else {
            return;
        };
        // Focus always attaches the retained buffer; never clear it here.
        session.project_ledger_with_live(entries, true, false);
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
            self.subagents.sessions.insert(
                id.to_string(),
                SessionView::new(self.workspace.git_branch.clone()),
            );
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

    fn refresh_synced_feed_for_focus(&mut self, id: &str) {
        let stale = self.subagents.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::agents::feed::FeedAuthority::SyncedAuthoritative
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

    pub(super) fn is_synced_authoritative_feed(&self, id: &str) -> bool {
        self.subagents.feeds.get(id).is_some_and(|feed| {
            feed.authority == crate::agents::feed::FeedAuthority::SyncedAuthoritative
        })
    }

    /// Whether this feed should retain a mid-turn live buffer (#1259).
    /// Includes warm-sync feeds before the first authoritative sync response so
    /// connect races do not drop the in-flight prefix.
    pub(super) fn retains_live_inflight_feed(&self, id: &str) -> bool {
        self.subagents.feeds.get(id).is_some_and(|feed| {
            feed.supports_sync
                || matches!(
                    feed.authority,
                    crate::agents::feed::FeedAuthority::WarmSync
                        | crate::agents::feed::FeedAuthority::SyncedAuthoritative
                )
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
        let rows = self.panel_rows();
        let Some(row) = rows.get(self.subagents.panel_nav.selected()) else {
            return;
        };
        if row.is_environment() {
            // Selecting an environment row shows its details in the main-pane
            // chrome over the master body (#1369 slice 4).
            self.subagents.selected_environment = row.env_key.clone();
            self.select_agent(None);
            return;
        }
        self.subagents.selected_environment = None;
        let target = row.id.clone();
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
        // A committed environment selection owns the cursor: with an
        // environment selected `active_agent_id` is `None`, which would
        // otherwise match the Master row and silently snap the cursor away
        // from the environment whose chrome is showing (review #1392).
        if let Some(env_key) = self.subagents.selected_environment.as_deref() {
            if let Some(idx) = rows
                .iter()
                .position(|r| r.env_key.as_deref() == Some(env_key))
            {
                self.subagents.panel_nav.set_selected(idx);
                return;
            }
            // The group dissolved (member exited/refreshed away): drop the
            // stale selection so the chrome and cursor fall back together.
            self.subagents.selected_environment = None;
        }
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
    pub(super) fn master_status(&self) -> &'static str {
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
            env_key: None,
            prefix: String::new(),
            badge: None,
            label: "Master Agent".to_string(),
            status: self.master_status().to_string(),
            workflow: master_wf,
        }];
        let groups = self.environment_groups();
        for (node, prefix) in self.subagent_tree_order(&groups) {
            match node {
                PanelNode::Environment(env_key) => {
                    // One selectable row naming the shared environment; its
                    // members nest below via the tree walk (#1369 slice 4).
                    // The node key is the grouping identity (uuid when
                    // reported, review #1392); the painted label is the ref.
                    let env = self.environment_info(&env_key);
                    let env_ref = env
                        .map(|e| e.environment_ref.clone())
                        .unwrap_or_else(|| env_key.clone());
                    let name = env
                        .and_then(|e| e.name.as_deref())
                        .filter(|n| !n.is_empty());
                    let label = match name {
                        Some(name) => format!("{env_ref} {name}"),
                        None => env_ref,
                    };
                    rows.push(PanelRow {
                        label,
                        status: env.map(|e| e.status.clone()).unwrap_or_default(),
                        workflow: None,
                        id: None,
                        env_key: Some(env_key),
                        badge: None,
                        prefix,
                    });
                }
                PanelNode::Agent(id) => {
                    let info = self.subagents.tracked.get(&id).map(|t| &t.info);
                    let workflow = info
                        .and_then(|i| i.workflow.as_ref())
                        .filter(|w| w.steps_total > 0)
                        .map(|w| (w.steps_completed, w.steps_total));
                    // Store/select by durable UUID identity; paint the human display
                    // label (display_name / compatibility agentId) in the panel (#1378).
                    let label = info
                        .map(|i| {
                            i.display_name
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(i.agent_id.as_str())
                                .to_string()
                        })
                        .unwrap_or_else(|| id.clone());
                    rows.push(PanelRow {
                        label,
                        status: info.map(|i| i.status.clone()).unwrap_or_default(),
                        workflow,
                        badge: self.solo_environment_badge(&id, &groups),
                        id: Some(id),
                        env_key: None,
                        prefix,
                    });
                }
            }
        }
        rows
    }

    /// Depth-first `(node, tree_prefix)` listing of the sub-agent tree. Root
    /// sub-agents (no in-map parent) sit under the master; `tree_prefix` is the
    /// connector stalk (`├ `/`└ ` with `│ `/`  ` ancestor continuation) so the
    /// panel draws tree lines back up to each parent. Order follows sorted ids.
    ///
    /// Environments shared by two or more agents (#1369 slice 4) contribute
    /// one environment node after the agent roots, with the member agents as
    /// its children — suppressed from the root list so no member is duplicated.
    fn subagent_tree_order(
        &self,
        groups: &std::collections::BTreeMap<String, Vec<String>>,
    ) -> Vec<(PanelNode, String)> {
        use std::collections::{BTreeMap, BTreeSet};
        let grouped: BTreeSet<&str> = groups.values().flatten().map(String::as_str).collect();
        // Parent key: `None` = under the master; `Some(key)` = under the node
        // with that key (an agent id, or an environment node key for grouped
        // members). Environment node keys can never collide with sanitized
        // agent ids because of the `\0` byte.
        let mut children: BTreeMap<Option<String>, Vec<PanelNode>> = BTreeMap::new();
        for (id, tracked) in &self.subagents.tracked {
            let parent = if grouped.contains(id.as_str()) {
                // Grouped members always nest under their environment row.
                tracked
                    .info
                    .environment
                    .as_ref()
                    .map(|e| PanelNode::env_key(e.group_key()))
            } else {
                // Treat an unknown parent as a root so its subtree is not lost.
                tracked
                    .info
                    .parent_id
                    .clone()
                    .filter(|p| self.subagents.tracked.contains_key(p))
            };
            children
                .entry(parent)
                .or_default()
                .push(PanelNode::Agent(id.clone()));
        }
        // Push siblings reversed (with their connector) so popping preserves order.
        // Stack item: (node, own_prefix, descendant_continuation_prefix).
        let push_children =
            |stack: &mut Vec<(PanelNode, String, String)>, kids: &[PanelNode], cont: &str| {
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
        let mut roots: Vec<PanelNode> = children.remove(&None).unwrap_or_default();
        roots.extend(groups.keys().map(|r| PanelNode::Environment(r.clone())));
        let mut out = Vec::new();
        let mut stack: Vec<(PanelNode, String, String)> = Vec::new();
        push_children(&mut stack, &roots, "");
        while let Some((node, own_prefix, cont)) = stack.pop() {
            if let Some(kids) = children.get(&Some(node.child_key())) {
                push_children(&mut stack, kids, &cont);
            }
            out.push((node, own_prefix));
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
        // Dim solo-environment badge between the tree stalk and the name
        // (#1369 slice 4); rendered as its own dim span plus one space.
        let badge = row.badge.as_deref().unwrap_or("");
        let badge_vis = if badge.is_empty() {
            0
        } else {
            visible_width(badge) + 1
        };
        let badge_span = if badge.is_empty() {
            String::new()
        } else {
            format!("{} ", theme::dim(badge))
        };
        let usable = width.saturating_sub(1);
        let name_avail =
            usable.saturating_sub(1 + stalk_vis + badge_vis + 1 + observer_vis + timer.len());
        let name = truncate_to_width(&sanitize_panel_label(&row.label), name_avail, Some("…"));
        let name_vis = visible_width(&name);
        let mut name = status_colored_name(&row.status, &name);
        if active {
            name = theme::bold(&name);
        }
        let pad = usable
            .saturating_sub(1 + stalk_vis + badge_vis + name_vis + observer_vis + timer.len());
        let line = format!(
            "{selbar}{}{badge_span}{name}{observer}{}{} ",
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
}

struct PanelRow {
    id: Option<String>,
    /// `Some(group key)` when this is a selectable environment row for a
    /// shared environment (#1369 slice 4); `id` is `None` for such rows. The
    /// key is the grouping identity (environment uuid when reported, else the
    /// session-scoped ref — review #1392), not necessarily the painted ref.
    env_key: Option<String>,
    /// Tree connector stalk drawn before the name (`├ `/`└ ` + ancestor `│ `).
    prefix: String,
    /// Dim environment ref drawn between the stalk and the name for an agent
    /// running alone in its environment (#1369 slice 4).
    badge: Option<String>,
    label: String,
    status: String,
    /// `(steps_completed, steps_total)` when the agent has an active workflow —
    /// drives the per-step progress bar drawn beneath the name row.
    workflow: Option<(u32, u32)>,
}

impl PanelRow {
    /// Whether this is a selectable environment row (not master, not an agent).
    fn is_environment(&self) -> bool {
        self.id.is_none() && self.env_key.is_some()
    }
}

/// One node in the panel tree walk: a tracked agent, or a shared-environment
/// grouping row (#1369 slice 4).
#[derive(Clone, Debug, PartialEq, Eq)]
enum PanelNode {
    Agent(String),
    Environment(String),
}

impl PanelNode {
    /// Parent-map key for members nesting under an environment node. The `\0`
    /// byte cannot appear in sanitized agent ids, so environment keys can
    /// never collide with them.
    fn env_key(env_ref: &str) -> String {
        format!("\0env:{env_ref}")
    }

    /// The key this node's children are registered under in the parent map.
    fn child_key(&self) -> String {
        match self {
            Self::Agent(id) => id.clone(),
            Self::Environment(env_ref) => Self::env_key(env_ref),
        }
    }
}
