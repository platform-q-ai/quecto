//! Sub-agent inspection panel wiring (#795).
//!
//! Glue between the `App` and the `subagent_inspector` component: opening/closing
//! via double-Up, key routing while open, building the master-detail data from
//! `subagent_local`, polling the selected agent's output tail by `agent_id`, and
//! rendering the full-screen frame.

use super::*;
use crate::interface::components::subagent_inspector::{
    AgentDetail, AgentRow, InspectorAction, SubagentInspector,
};

impl App {
    /// Whether the inspector panel is currently open.
    pub(super) fn inspector_open(&self) -> bool {
        self.subagent_inspector.is_some()
    }

    /// Decide whether a second Up press within the window should open the panel.
    /// Extracted for testability (no `App`/time coupling).
    pub(super) fn double_up_should_open(
        prev: Option<tokio::time::Instant>,
        now: tokio::time::Instant,
    ) -> bool {
        prev.is_some_and(|p| now.duration_since(p) <= DOUBLE_UP_WINDOW)
    }

    /// Handle an Up press at the editor fallthrough: detect double-Up to open the
    /// inspector. Returns `true` if the press was consumed (do not forward to the
    /// editor). Single Up — or Up while typing — falls through unchanged so
    /// history/cursor navigation is preserved (#795).
    pub(super) fn try_open_inspector_on_up(&mut self) -> bool {
        if self.subagent_local.is_empty() {
            self.last_up_press = None;
            return false;
        }
        let now = tokio::time::Instant::now();
        if Self::double_up_should_open(self.last_up_press, now) {
            self.last_up_press = None;
            self.open_subagent_inspector();
            return true;
        }
        // Arm the double-Up only from an empty editor so typing is unaffected.
        self.last_up_press = self.editor.text().is_empty().then_some(now);
        false
    }

    /// Open the inspector, seeded from the currently tracked sub-agents, and
    /// kick off the first output-tail fetch for the highlighted agent.
    pub(super) fn open_subagent_inspector(&mut self) {
        let rows = self.build_inspector_rows();
        if rows.is_empty() {
            return;
        }
        // Clear any history text the first arming Up may have recalled so the
        // editor is pristine when the panel closes.
        self.editor.set_text("");
        let inspector = SubagentInspector::new(rows);
        if let Some(agent_id) = inspector.selected_agent_id() {
            self.request_inspector_tail(&agent_id);
        }
        self.subagent_inspector = Some(inspector);
    }

    /// Close the inspector and return to the normal TUI view.
    pub(super) fn close_subagent_inspector(&mut self) {
        self.subagent_inspector = None;
        self.inspector_tail_inflight = None;
    }

    /// Route a key into the open inspector, acting on the resulting action.
    pub(super) fn handle_subagent_inspector_key(&mut self, key: &Key) {
        let Some(inspector) = &mut self.subagent_inspector else {
            return;
        };
        match inspector.handle_key(key) {
            InspectorAction::Close => self.close_subagent_inspector(),
            InspectorAction::SelectionChanged => {
                if let Some(agent_id) = self
                    .subagent_inspector
                    .as_ref()
                    .and_then(|i| i.selected_agent_id())
                {
                    let need_fetch = self
                        .subagent_inspector
                        .as_ref()
                        .is_some_and(|i| !i.has_tail(&agent_id));
                    if need_fetch {
                        self.request_inspector_tail(&agent_id);
                    }
                }
            }
            InspectorAction::Consumed => {}
        }
    }

    /// Build the left-list rows from the tracked sub-agents (stable id order).
    fn build_inspector_rows(&self) -> Vec<AgentRow> {
        self.subagent_local
            .iter()
            .map(|(id, tracked)| AgentRow {
                agent_id: id.clone(),
                label: inspector_row_label(id, &tracked.info),
            })
            .collect()
    }

    /// Build the live detail for one agent from `subagent_local`.
    fn build_inspector_detail(&self, agent_id: &str) -> Option<AgentDetail> {
        let tracked = self.subagent_local.get(agent_id)?;
        let now = tokio::time::Instant::now();
        Some(AgentDetail {
            agent_id: agent_id.to_string(),
            status: tracked.info.status.clone(),
            elapsed_secs: tracked.elapsed_secs(now),
            workflow: tracked
                .info
                .workflow
                .as_ref()
                .map(|w| (w.mode.clone(), w.steps_completed, w.steps_total)),
            output: Vec::new(),
        })
    }

    /// Request the selected sub-agent's recent output tail over the existing UDS
    /// connection (reuses `get_messages_tail` with an `agent_id`). The response
    /// id carries the agent id so the handler can route it back (#795).
    pub(super) fn request_inspector_tail(&mut self, agent_id: &str) {
        self.inspector_tail_inflight = Some(agent_id.to_string());
        self.send_command(Command::GetMessagesTail {
            id: Some(format!("inspector-tail:{agent_id}")),
            count: 20,
            agent_id: Some(agent_id.to_string()),
        });
    }

    /// Poll the highlighted agent's tail on the timer while the panel is open.
    /// Skips while a request is already outstanding so polls can't stack on the
    /// UDS faster than the kernel round-trip drains them (#795).
    pub(super) fn poll_inspector_tail(&mut self) {
        if self.inspector_tail_inflight.is_some() {
            return;
        }
        if let Some(agent_id) = self
            .subagent_inspector
            .as_ref()
            .and_then(|i| i.selected_agent_id())
        {
            self.request_inspector_tail(&agent_id);
        }
    }

    /// Route a `get_messages_tail` response whose id targets the inspector into
    /// the component's per-agent tail cache. Returns `true` if it was consumed.
    pub(super) fn handle_inspector_tail_response(
        &mut self,
        id: Option<&str>,
        data: Option<&serde_json::Value>,
    ) -> bool {
        let Some(agent_id) = id.and_then(|i| i.strip_prefix("inspector-tail:")) else {
            return false;
        };
        if self.inspector_tail_inflight.as_deref() == Some(agent_id) {
            self.inspector_tail_inflight = None;
        }
        let lines = data.map(messages_tail_to_lines).unwrap_or_default();
        if let Some(inspector) = &mut self.subagent_inspector {
            inspector.set_tail(agent_id, lines);
        }
        true
    }

    /// Compose the full-screen inspector frame (width-enforced).
    pub(super) fn compose_subagent_inspector_frame(&mut self) -> Vec<String> {
        let width = self.terminal.width;
        let height = self.terminal.height;
        // Refresh the left list from the live tracked agents before resolving the
        // detail, so the two panels share one lifetime and never desync (#795).
        let rows = self.build_inspector_rows();
        if let Some(inspector) = &mut self.subagent_inspector {
            inspector.sync_rows(rows);
        }
        let detail = self
            .subagent_inspector
            .as_ref()
            .and_then(|i| i.selected_agent_id())
            .and_then(|id| self.build_inspector_detail(&id));
        let Some(inspector) = &mut self.subagent_inspector else {
            return vec![String::new(); height];
        };
        inspector.render(detail.as_ref(), width, height)
    }
}

/// Build a left-list row label: status glyph + id + workflow progress.
fn inspector_row_label(
    id: &str,
    info: &crate::infrastructure::client::SubagentInfoEvent,
) -> String {
    let glyph = match info.status.as_str() {
        "running" | "starting" => "●",
        "exited" => "✓",
        "error" => "✗",
        _ => "○",
    };
    let wf = info
        .workflow
        .as_ref()
        .map(|w| format!("  wf {} {}/{}", w.mode, w.steps_completed, w.steps_total))
        .unwrap_or_default();
    format!("{glyph} {id}{wf}")
}

/// Flatten a `{"messages":[{role,content,...}]}` tail payload into display lines.
fn messages_tail_to_lines(data: &serde_json::Value) -> Vec<String> {
    let Some(messages) = data.get("messages").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for msg in messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        out.push(format!("[{role}]"));
        for text in content_to_text(msg.get("content")) {
            for line in text.lines() {
                out.push(line.to_string());
            }
        }
        out.push(String::new());
    }
    out
}

/// Flatten a message `content` field into display text. Content may be a plain
/// string or an array of typed blocks (`text`, `tool_use`, `tool_result`, …);
/// without this, structured/tool-call content silently renders blank (#795
/// review). Unknown blocks fall back to a compact `[type]` marker.
fn content_to_text(content: Option<&serde_json::Value>) -> Vec<String> {
    match content {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .map(|b| {
                // Most blocks carry a `text`; tool calls/results don't, so show
                // a typed marker (with the tool name when present) instead.
                if let Some(text) = b.get("text").and_then(|v| v.as_str()) {
                    text.to_string()
                } else {
                    let kind = b.get("type").and_then(|v| v.as_str()).unwrap_or("block");
                    match b.get("name").and_then(|v| v.as_str()) {
                        Some(name) => format!("[{kind}: {name}]"),
                        None => format!("[{kind}]"),
                    }
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "app_subagent_inspector_tests.rs"]
mod tests;
