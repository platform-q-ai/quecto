//! Environment grouping model and main-pane chrome for the sub-agent panel
//! (#1369 slice 4), plus the selected-agent title chrome moved from
//! `controller_subagent_panel.rs` (750-line cap).
//!
//! Hybrid rendering rule: one agent in an environment renders as a flat agent
//! row with a dim `CN` badge; two or more agents render one selectable
//! environment row with the members nested below (and suppressed from the
//! root list). Selecting an environment row renders its details in the main
//! pane's top chrome.

use super::*;
use crate::components::theme;
use crate::protocol::client::SubagentEnvironmentInfo;
use std::collections::BTreeMap;

use super::app_subagent_panel::controller_subagent_panel_helpers::{
    pad_cell, sanitize_panel_label, status_colored_name,
};

impl App {
    // ── Environment grouping model ─────────────────────────────────────

    /// Environments shared by two or more tracked agents: `group key →
    /// member ids` (sorted, from the sorted tracked map). Keyed on
    /// [`SubagentEnvironmentInfo::group_key`] — the globally-unique uuid when
    /// reported — NOT the session-scoped `CN` ref, which restarts at `C1` per
    /// session and would merge unrelated forwarded-descendant environments
    /// (review #1392). Solo environments are not grouped — their agent renders
    /// flat with a badge.
    pub(super) fn environment_groups(&self) -> BTreeMap<String, Vec<String>> {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, tracked) in &self.subagents.tracked {
            if let Some(env) = &tracked.info.environment {
                if !env.group_key().is_empty() {
                    groups
                        .entry(env.group_key().to_string())
                        .or_default()
                        .push(id.clone());
                }
            }
        }
        groups.retain(|_, members| members.len() >= 2);
        groups
    }

    /// The dim badge for `id`'s row: its environment ref when it runs alone in
    /// that environment (grouped members carry no badge — the environment row
    /// names the ref once).
    pub(super) fn solo_environment_badge(
        &self,
        id: &str,
        groups: &BTreeMap<String, Vec<String>>,
    ) -> Option<String> {
        let env = self.subagents.tracked.get(id)?.info.environment.as_ref()?;
        if env.environment_ref.is_empty() || groups.contains_key(env.group_key()) {
            return None;
        }
        // Sanitize here: the ref is wire-controlled and this badge is the one
        // label rendered outside `sanitize_panel_label` (review #1392 —
        // terminal escape injection otherwise).
        Some(sanitize_panel_label(&env.environment_ref))
    }

    /// Any tracked member's environment metadata for the group `key` (members
    /// share the registry-owned fields; sticky merge keeps them through sparse
    /// refreshes). Per-member fields (`socket_mode`) must NOT be read from this
    /// arbitrary member — use [`Self::environment_socket_mode`].
    pub(super) fn environment_info(&self, key: &str) -> Option<&SubagentEnvironmentInfo> {
        self.subagents
            .tracked
            .values()
            .filter_map(|t| t.info.environment.as_ref())
            .find(|e| e.group_key() == key)
    }

    /// Aggregate status across every member of the environment `key`: the
    /// most-degraded value wins, so a stale forwarded member's copy cannot
    /// show `running` after the environment began dying (review #1392).
    /// Unknown labels rank just above `running`, so a new wire value beats a
    /// healthy member but is still masked by any known terminal state.
    pub(super) fn environment_status(&self, key: &str) -> String {
        fn rank(status: &str) -> u8 {
            match status {
                "cleanup-failed" => 5,
                "killing" => 4,
                "stopped" => 3,
                "empty" => 2,
                "running" => 0,
                _ => 1,
            }
        }
        self.subagents
            .tracked
            .values()
            .filter_map(|t| t.info.environment.as_ref())
            .filter(|e| e.group_key() == key)
            .map(|e| e.status.as_str())
            .filter(|s| !s.is_empty())
            .max_by_key(|s| (rank(s), s.to_string()))
            .unwrap_or_default()
            .to_string()
    }

    /// Aggregate socket mode across every member of the environment `key`:
    /// the shared value when all members agree, `mixed` when they differ
    /// (socket mode is per-member, review #1392), `-` when unreported.
    pub(super) fn environment_socket_mode(&self, key: &str) -> String {
        let mut modes: Vec<&str> = self
            .subagents
            .tracked
            .values()
            .filter_map(|t| t.info.environment.as_ref())
            .filter(|e| e.group_key() == key)
            .map(|e| e.socket_mode.as_str())
            .filter(|m| !m.is_empty())
            .collect();
        modes.sort_unstable();
        modes.dedup();
        match modes.as_slice() {
            [] => "-".to_string(),
            [one] => (*one).to_string(),
            _ => "mixed".to_string(),
        }
    }

    // ── Environment main-pane chrome ───────────────────────────────────

    /// The selected environment's detail chrome for the main pane, or `None`
    /// when no environment is selected (or its metadata is gone).
    pub(super) fn render_environment_chrome(&self, width: usize) -> Option<Vec<String>> {
        let env_key = self.subagents.selected_environment.as_deref()?;
        let env = self.environment_info(env_key)?;
        let dot = theme::dim("·");
        let name = env
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| format!("{} ", sanitize_panel_label(n)))
            .unwrap_or_default();
        // Status is aggregated worst-wins across members — a stale forwarded
        // copy must not report `running` for a dying environment.
        let status = self.environment_status(env_key);
        let title = format!(
            "{} {name}{dot} status: {}",
            theme::bold(&sanitize_panel_label(&env.environment_ref)),
            status_colored_name(&status, &sanitize_panel_label(&status)),
        );
        let repo = format!(
            "repo: {} {dot} branch: {}",
            sanitize_panel_label(&env.repository),
            sanitize_panel_label(env.branch.as_deref().unwrap_or("-")),
        );
        let runtime = format!(
            "runtime: {} {dot} workspace: {} {dot} socket: {}",
            sanitize_panel_label(&env.runtime_id),
            sanitize_panel_label(&env.workspace),
            // Aggregated across members — socket mode is per-member and one
            // environment can mix direct and proxy endpoints (review #1392).
            sanitize_panel_label(&self.environment_socket_mode(env_key)),
        );
        Some(
            [title, repo, runtime]
                .into_iter()
                .map(|line| {
                    let line = crate::components::utils::truncate_to_width(&line, width, Some("…"));
                    pad_cell(&line, width)
                })
                .collect(),
        )
    }

    // ── Main-pane chrome for the selected agent (#820/#1288/#1309) ─────

    /// The main pane's top chrome (#820 / #1288 / #1309): a title line
    /// (`agent · status · elapsed · #issue workflow`) plus, when a workflow is
    /// active, one compact progress line framed by separator rules above and
    /// below (`────` / content / `────`). Title always renders; the compact
    /// indicator is appended only when `render_compact_line` has content.
    /// Phase pills and shortcut hints from the old multi-line status box
    /// (#1246) must not return — only the top/bottom rule separators around
    /// the single compact line. When an environment row is selected (#1369
    /// slice 4), its detail chrome replaces the agent title.
    pub(super) fn render_main_pane_workflow(
        &self,
        width: usize,
        box_width: usize,
        now: tokio::time::Instant,
    ) -> Vec<String> {
        if width < 4 {
            return Vec::new();
        }
        if let Some(lines) = self.render_environment_chrome(width) {
            return lines;
        }
        let state = self.active_workflow_bar();
        let mut out = vec![pad_cell(&self.main_pane_title(state, now), width)];
        if let Some(content) = workflow_bar::render_compact_line(state) {
            let rule_width = box_width.max(1);
            let inner = rule_width.saturating_sub(2);
            let rule = theme::dim(&"─".repeat(rule_width));
            // Truncate the label with ellipsis (#1288), then pad the framed row
            // to the same width as the separator rules so framing stays flush.
            let framed = format!(
                " {} ",
                crate::components::utils::truncate_to_width(&content, inner, Some("…")),
            );
            out.push(rule.clone());
            out.push(pad_cell(&framed, rule_width));
            out.push(rule);
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
            Some(id) => {
                // Selection is UUID-keyed; paint the human display label (#1378).
                let tracked = self.subagents.tracked.get(id);
                let label = tracked
                    .map(|t| {
                        t.info
                            .display_name
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(t.info.agent_id.as_str())
                            .to_string()
                    })
                    .unwrap_or_else(|| id.to_string());
                let status = tracked.map(|t| t.info.status.clone()).unwrap_or_default();
                (label, status)
            }
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
            let auto = if state.workflow_auto_continue {
                "auto:on"
            } else {
                "auto:off"
            };
            title.push_str(&format!(
                " {} {} {} {}",
                theme::dim("·"),
                theme::accent(&theme::bold(&format!("#{n}"))),
                theme::dim("workflow"),
                theme::dim(auto),
            ));
        }
        title
    }
}
