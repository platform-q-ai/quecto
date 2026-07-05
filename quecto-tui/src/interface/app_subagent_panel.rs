use super::*;
use crate::interface::theme;

impl App {
    pub(super) fn subagent_panel_visible(&self) -> bool {
        self.agent_connected
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn active_agent_id(&self) -> Option<&str> {
        self.subagents.active_agent_id.as_deref()
    }

    #[cfg(test)]
    pub(super) fn retained_session_ids(&self) -> Vec<String> {
        self.subagents.sessions.keys().cloned().collect()
    }

    #[cfg(test)]
    pub(super) fn subagent_socket_path(&self, id: &str) -> Option<String> {
        self.subagents
            .local
            .get(id)
            .and_then(|t| t.info.socket_path.clone())
    }

    pub(super) fn active_session(&self) -> &SessionView {
        match self.subagents.active_agent_id.as_deref() {
            None => &self.master_session,
            Some(id) => self
                .subagents
                .sessions
                .get(id)
                .unwrap_or(&self.master_session),
        }
    }

    pub(super) fn active_session_mut(&mut self) -> &mut SessionView {
        let Some(id) = self.subagents.active_agent_id.clone() else {
            return &mut self.master_session;
        };
        if !self.subagents.sessions.contains_key(&id) {
            let git_branch = self.git_branch.clone();
            Self::remember_session(&mut self.subagents.session_order, &id);
            self.subagents
                .sessions
                .insert(id.clone(), SessionView::new(git_branch));
        }
        self.subagents.sessions.get_mut(&id).unwrap()
    }

    pub(super) fn active_chat_mut(&mut self) -> &mut Chat {
        &mut self.active_session_mut().chat
    }

    #[cfg(test)]
    pub(crate) fn session_chat_entry_count(&self, agent_id: &str) -> Option<usize> {
        self.subagents
            .sessions
            .get(agent_id)
            .map(|s| s.chat.entry_count())
    }

    pub(super) fn active_workflow_bar(&self) -> &workflow_bar::WorkflowBarState {
        &self.active_session().workflow_bar
    }

    pub(super) fn active_footer_render(&mut self, width: usize) -> Vec<String> {
        self.active_session_mut().footer.render(width)
    }

    pub(super) fn active_subagent_running(&self) -> bool {
        let session = self.active_session();
        if session.running {
            return true;
        }
        if session.observed_run_state {
            return false;
        }
        match &self.subagents.active_agent_id {
            Some(id) => self
                .subagents
                .local
                .get(id)
                .is_some_and(|t| subagent_status_is_active(&t.info.status)),
            None => false,
        }
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn focus_region(&self) -> Focus {
        self.subagents.focus
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn panel_highlight_index(&self) -> usize {
        self.subagents.panel_nav.selected()
    }

    pub(super) fn send_to_active_subagent(&mut self, cmd: Command) -> bool {
        let Some(id) = self.subagents.active_agent_id.clone() else {
            return false;
        };
        let Some((conn_id, tx)) = &self.subagents.active_cmd_tx else {
            return false;
        };
        if conn_id != &id {
            return false;
        }
        tx.try_send(cmd).is_ok()
    }

    pub(super) fn select_agent(&mut self, agent_id: Option<&str>) {
        let new_active = agent_id.map(str::to_string);
        if new_active == self.subagents.active_agent_id {
            return;
        }
        self.teardown_active_connection();
        self.subagents.active_agent_id = new_active.clone();
        self.sync_panel_selection_to_active();

        let Some(id) = new_active else {
            return;
        };
        self.ensure_session(&id);
        self.seed_session_bar_from_snapshot(&id); // main-pane bar from snapshot (#913)
        self.open_subagent_connection(&id);
    }

    fn reconcile_active_agent(&mut self) {
        if let Some(active) = self.subagents.active_agent_id.clone() {
            if !self.subagents.local.contains_key(&active) {
                self.select_agent(None);
            }
        }
    }

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

    fn evict_retained_sessions(&mut self) {
        while self.subagents.sessions.len() > MAX_RETAINED_SESSIONS {
            let Some(pos) = self
                .subagents
                .session_order
                .iter()
                .position(|id| Some(id) != self.subagents.active_agent_id.as_ref())
            else {
                break;
            };
            let victim = self.subagents.session_order.remove(pos);
            self.subagents.sessions.remove(&victim);
        }
    }

    pub(super) fn panel_highlight_previous(&mut self) {
        let len = self.panel_rows().len();
        self.subagents.panel_nav.move_previous(len);
    }

    pub(super) fn panel_highlight_next(&mut self) {
        let len = self.panel_rows().len();
        self.subagents.panel_nav.move_next(len);
    }

    pub(super) fn panel_highlight_row(&mut self, one_based: usize) {
        let len = self.panel_rows().len();
        if one_based >= 1 && one_based <= len {
            self.subagents.panel_nav.set_selected(one_based - 1);
        }
    }

    pub(super) fn commit_panel_selection(&mut self) {
        let target = self
            .panel_rows()
            .get(self.subagents.panel_nav.selected())
            .and_then(|r| r.id.clone());
        self.select_agent(target.as_deref());
    }

    pub(super) fn clamp_panel_selection(&mut self) {
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

    fn open_subagent_connection(&mut self, id: &str) {
        let Some(socket) = self
            .subagents
            .local
            .get(id)
            .and_then(|t| t.info.socket_path.clone())
        else {
            return;
        };
        let tx = self.subagents.event_tx.clone();
        let agent_id = id.to_string();
        let path = std::path::PathBuf::from(socket);
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);
        let handle = tokio::spawn(async move {
            let Ok(mut client) = Client::connect(&path).await else {
                return;
            };
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
                        Some(cmd) => {
                            let _ = client.send(&cmd).await;
                        }
                        None => break,
                    },
                }
            }
        });
        self.subagents.active_conn = Some((id.to_string(), handle));
        self.subagents.active_cmd_tx = Some((id.to_string(), cmd_tx));
    }

    fn teardown_active_connection(&mut self) {
        if let Some((_, handle)) = self.subagents.active_conn.take() {
            handle.abort();
        }
        self.subagents.active_cmd_tx = None;
    }

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
            let info = self.subagents.local.get(&id).map(|t| &t.info);
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

    fn subagent_tree_order(&self) -> Vec<(String, String)> {
        use std::collections::BTreeMap;
        let mut children: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
        for (id, tracked) in &self.subagents.local {
            let parent = tracked
                .info
                .parent_id
                .clone()
                .filter(|p| self.subagents.local.contains_key(p));
            children.entry(parent).or_default().push(id.clone());
        }
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

        let blocks: Vec<Vec<String>> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let sel = i == selected;
                let mut block =
                    vec![self.panel_name_line(row, sel, row.id.as_deref() == active, width, now)];
                if let Some((done, total)) = row.workflow {
                    block.push(panel_bar_line(&row.prefix, done, total, width));
                }
                block
            })
            .collect();

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

    fn panel_name_line(
        &self,
        row: &PanelRow,
        selected: bool,
        active: bool,
        width: usize,
        now: tokio::time::Instant,
    ) -> String {
        use crate::interface::utils::{truncate_to_width, visible_width};
        let selbar = if selected {
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
        let entry = self.subagents.local.get(id)?;
        if entry.info.read_only {
            Some(theme::OBSERVER_MARKER)
        } else {
            None
        }
    }

    fn panel_status(&self, id: &str) -> String {
        self.subagents
            .local
            .get(id)
            .map(|t| t.info.status.clone())
            .unwrap_or_default()
    }

    fn panel_row_timer(&self, id: Option<&str>, now: tokio::time::Instant) -> String {
        match id {
            None => fmt_mss(now.saturating_duration_since(self.started_at).as_secs()),
            Some(id) => self
                .subagents
                .local
                .get(id)
                .map(|t| fmt_mss(t.elapsed_secs(now)))
                .unwrap_or_default(),
        }
    }

    pub(super) fn panel_row_elapsed(&self, id: Option<&str>, now: tokio::time::Instant) -> String {
        let Some(id) = id else {
            return fmt_mss(now.saturating_duration_since(self.started_at).as_secs());
        };
        let Some(t) = self.subagents.local.get(id) else {
            return String::new();
        };
        let mss = fmt_mss(t.elapsed_secs(now));
        if t.info.status == "idle" {
            format!("idle (ran {mss})")
        } else {
            mss
        }
    }

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

    fn main_pane_title(
        &self,
        state: &workflow_bar::WorkflowBarState,
        now: tokio::time::Instant,
    ) -> String {
        let (name, status) = match self.subagents.active_agent_id.as_deref() {
            None => ("Master".to_string(), self.master_status().to_string()),
            Some(id) => (id.to_string(), self.panel_status(id)),
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

fn fmt_mss(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn panel_bar_line(prefix: &str, done: u32, total: u32, width: usize) -> String {
    use crate::interface::utils::visible_width;
    const MAX_CELLS: usize = 20;
    let cont = bar_continuation(prefix);
    let cont_vis = visible_width(&cont);
    let avail = width.saturating_sub(2 + cont_vis);
    let cells = (total as usize).min(MAX_CELLS).min(avail).max(1);
    let filled = ((done as usize) * cells / (total.max(1) as usize)).min(cells);
    let bar = format!(
        "{}{}",
        theme::accent(&"▰".repeat(filled)),
        theme::dim(&"▱".repeat(cells - filled)),
    );
    pad_cell(&format!(" {}{bar}", theme::dim(&cont)), width)
}

fn bar_continuation(prefix: &str) -> String {
    if let Some(head) = prefix.strip_suffix("├ ") {
        format!("{head}│ ")
    } else if let Some(head) = prefix.strip_suffix("└ ") {
        format!("{head}  ")
    } else {
        prefix.to_string()
    }
}

fn boxed_inner(content: &str, inner: usize) -> String {
    let visible = crate::interface::utils::visible_width(content);
    if visible >= inner {
        crate::interface::utils::truncate_to_width(content, inner, None)
    } else {
        format!("{}{}", content, " ".repeat(inner - visible))
    }
}

struct PanelRow {
    id: Option<String>,
    prefix: String,
    label: String,
    status: String,
    workflow: Option<(u32, u32)>,
}

fn status_colored_name(status: &str, name: &str) -> String {
    match status {
        "running" | "starting" => theme::green(name),
        "idle" => theme::yellow(name),
        "error" | "errored" => theme::red(name),
        "exited" => theme::dim(name),
        _ => name.to_string(),
    }
}

fn sanitize_panel_label(s: &str) -> String {
    crate::interface::components::sanitize::strip_terminal_control(s)
}

fn pad_cell(text: &str, width: usize) -> String {
    let visible = crate::interface::utils::visible_width(text);
    if visible > width {
        crate::interface::utils::truncate_to_width(text, width, None)
    } else {
        format!("{}{}", text, " ".repeat(width - visible))
    }
}
