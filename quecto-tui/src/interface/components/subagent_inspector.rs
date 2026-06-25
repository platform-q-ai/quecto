//! Sub-agent inspection panel (#795) — a full-screen master-detail
//! "dev team in a box" view.
//!
//! Reachable via double-Up while sub-agents are running. The left panel is a
//! navigable list of sub-agents (reusing [`SelectList`]/`ListNavigator`); the
//! right panel shows the highlighted agent's workflow/phase status header and
//! its recent (live-ish) output. Focus steps list → detail (Enter) and back /
//! closed (Esc). Rendering is pure and render-idempotent so the headless
//! harness can assert no-flash / stable layout.

use std::collections::BTreeMap;

use crate::interface::component::Component;
use crate::interface::components::select_list::{SelectItem, SelectList};
use crate::interface::components::workflow_bar;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width, wrap_text};

/// Width of the left agent-list column.
const LIST_WIDTH: usize = 34;
/// Footer hint shown at the bottom of the panel.
const FOOTER_HINT: &str = "↑↓ select · enter open · esc back";

/// Which panel currently owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorFocus {
    /// The left agent list — ↑/↓ move the highlight; Enter focuses the detail.
    List,
    /// The right detail panel — ↑/↓/PageUp/PageDown scroll the output.
    Detail,
}

/// Outcome of routing a key into the inspector, for the caller to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorAction {
    /// Key consumed; nothing further required.
    Consumed,
    /// The highlighted agent changed — the caller should (re)fetch its tail.
    SelectionChanged,
    /// The panel should close and the normal TUI view should be restored.
    Close,
}

/// One agent's live detail, supplied by the caller (`App`) at render time so the
/// component never depends on the app's private sub-agent types.
#[derive(Debug, Clone, Default)]
pub struct AgentDetail {
    pub agent_id: String,
    pub status: String,
    pub elapsed_secs: u64,
    /// `(mode, done, total)` workflow snapshot, when known.
    pub workflow: Option<(String, u32, u32)>,
    /// Recent output lines (most recent last).
    pub output: Vec<String>,
}

/// A row in the left agent list, supplied by the caller at open time.
#[derive(Debug, Clone)]
pub struct AgentRow {
    pub agent_id: String,
    pub label: String,
}

/// Full-screen master-detail inspector state.
pub struct SubagentInspector {
    list: SelectList,
    focus: InspectorFocus,
    /// Detail-panel vertical scroll offset (in lines).
    scroll: usize,
    /// Cached output tails per agent id so switching agents is instant.
    tails: BTreeMap<String, Vec<String>>,
}

impl SubagentInspector {
    /// Build the inspector from the current sub-agent rows. Selection starts on
    /// the first agent and focus on the list.
    pub fn new(rows: Vec<AgentRow>) -> Self {
        let items = rows
            .into_iter()
            .map(|r| SelectItem {
                value: r.agent_id,
                label: r.label,
                description: None,
            })
            .collect();
        Self {
            list: SelectList::new(items, 24),
            focus: InspectorFocus::List,
            scroll: 0,
            tails: BTreeMap::new(),
        }
    }

    pub fn focus(&self) -> InspectorFocus {
        self.focus
    }

    /// The currently highlighted agent id, if any.
    pub fn selected_agent_id(&self) -> Option<String> {
        self.list.selected_item().map(|i| i.value.clone())
    }

    /// Store a freshly fetched output tail for an agent.
    pub fn set_tail(&mut self, agent_id: &str, lines: Vec<String>) {
        self.tails.insert(agent_id.to_string(), lines);
    }

    /// Whether an output tail has already been fetched for an agent.
    pub fn has_tail(&self, agent_id: &str) -> bool {
        self.tails.contains_key(agent_id)
    }

    fn cached_tail(&self, agent_id: &str) -> Vec<String> {
        self.tails.get(agent_id).cloned().unwrap_or_default()
    }

    /// Route a key. The focus state machine:
    /// - List: ↑/↓ navigate (→ `SelectionChanged`), Enter → Detail focus,
    ///   Esc → `Close`.
    /// - Detail: ↑/↓/PageUp/PageDown scroll, Esc → back to List focus.
    pub fn handle_key(&mut self, key: &Key) -> InspectorAction {
        match self.focus {
            InspectorFocus::List => match key {
                Key::Up | Key::Down => {
                    self.list.handle_input(key);
                    // Discard the SelectList's internal result so a later Enter
                    // does not see a stale selection.
                    let _ = self.list.take_result();
                    self.scroll = 0;
                    InspectorAction::SelectionChanged
                }
                Key::Enter => {
                    self.focus = InspectorFocus::Detail;
                    InspectorAction::Consumed
                }
                Key::Escape => InspectorAction::Close,
                _ => InspectorAction::Consumed,
            },
            InspectorFocus::Detail => match key {
                Key::Up => {
                    self.scroll = self.scroll.saturating_sub(1);
                    InspectorAction::Consumed
                }
                Key::Down => {
                    self.scroll = self.scroll.saturating_add(1);
                    InspectorAction::Consumed
                }
                Key::PageUp => {
                    self.scroll = self.scroll.saturating_sub(10);
                    InspectorAction::Consumed
                }
                Key::PageDown => {
                    self.scroll = self.scroll.saturating_add(10);
                    InspectorAction::Consumed
                }
                Key::Escape => {
                    self.focus = InspectorFocus::List;
                    InspectorAction::Consumed
                }
                _ => InspectorAction::Consumed,
            },
        }
    }

    /// Render the full-screen panel. `detail` is the live data for the currently
    /// highlighted agent (the caller resolves it from `selected_agent_id`).
    /// Pure / render-idempotent: no state mutation, identical output for
    /// identical inputs.
    pub fn render(
        &mut self,
        detail: Option<&AgentDetail>,
        width: usize,
        height: usize,
    ) -> Vec<String> {
        let mut lines = Vec::with_capacity(height);

        // Title.
        let count = self.list.item_count();
        lines.push(theme::bold(&format!(
            "  Sub-agents — dev team in a box   ({count} agent{})",
            if count == 1 { "" } else { "s" }
        )));
        lines.push(String::new());

        let list_width = LIST_WIDTH.min(width.saturating_sub(20)).max(12);
        let sep = " │ ";
        let right_width = width.saturating_sub(list_width + sep.len()).max(8);

        // Body height between title (2) and footer (2).
        let body_height = height.saturating_sub(4);

        let left = self.list.render(list_width);
        let right = render_detail(
            detail,
            &self.cached_tail_for(detail),
            right_width,
            self.scroll,
        );

        for row in 0..body_height {
            let l = left.get(row).cloned().unwrap_or_default();
            let r = right.get(row).cloned().unwrap_or_default();
            let l_pad = pad_to(&l, list_width);
            let focus_sep = if self.focus == InspectorFocus::Detail {
                theme::accent(sep)
            } else {
                theme::dim(sep)
            };
            lines.push(format!("{l_pad}{focus_sep}{r}"));
        }

        // Footer.
        lines.push(String::new());
        lines.push(theme::dim(&format!("  {FOOTER_HINT}")));

        // Exactly `height` lines.
        lines.truncate(height);
        while lines.len() < height {
            lines.push(String::new());
        }
        for line in &mut lines {
            if visible_width(line) > width {
                *line = truncate_to_width(line, width, None);
            }
        }
        lines
    }

    fn cached_tail_for(&self, detail: Option<&AgentDetail>) -> Vec<String> {
        match detail {
            Some(d) if !d.output.is_empty() => d.output.clone(),
            Some(d) => self.cached_tail(&d.agent_id),
            None => Vec::new(),
        }
    }
}

/// Pad a (possibly ANSI-coloured) string to a visible width with spaces.
fn pad_to(s: &str, width: usize) -> String {
    let vis = visible_width(s);
    if vis >= width {
        truncate_to_width(s, width, None)
    } else {
        format!("{s}{}", " ".repeat(width - vis))
    }
}

/// Render the right detail panel: a workflow/phase status header followed by the
/// agent's recent output, scrolled by `scroll`.
fn render_detail(
    detail: Option<&AgentDetail>,
    output: &[String],
    width: usize,
    scroll: usize,
) -> Vec<String> {
    let Some(detail) = detail else {
        return vec![theme::dim("No sub-agent selected")];
    };

    let mut out = Vec::new();
    // Header line: id · status · elapsed.
    out.push(format!(
        "{}  {}  {}",
        theme::accent(&theme::bold(&detail.agent_id)),
        theme::muted(&detail.status),
        theme::dim(&format!("{}s", detail.elapsed_secs)),
    ));

    // Workflow/phase status header, reusing the workflow-bar phase rendering.
    if let Some((mode, done, total)) = &detail.workflow {
        let state = workflow_bar::WorkflowBarState {
            done: *done,
            total: *total,
            mode: Some(mode.clone()),
            ..Default::default()
        };
        out.extend(workflow_status_header(&state, width));
    }
    out.push(theme::dim(&"─".repeat(width.min(60))));

    // Output body (word-wrapped), scrolled.
    let mut body: Vec<String> = Vec::new();
    for line in output {
        if line.is_empty() {
            body.push(String::new());
        } else {
            body.extend(wrap_text(line, width));
        }
    }
    if body.is_empty() {
        body.push(theme::dim("(waiting for output…)"));
    }
    let start = scroll.min(body.len().saturating_sub(1));
    out.extend(body[start..].iter().cloned());
    out
}

/// A one-line workflow/phase header derived from a [`workflow_bar::WorkflowBarState`].
///
/// Reuses the workflow-bar progress-bar style for visual consistency with the
/// main TUI bar without requiring per-step data (sub-agent snapshots carry only
/// mode + done/total).
fn workflow_status_header(state: &workflow_bar::WorkflowBarState, width: usize) -> Vec<String> {
    let done = state.done;
    let total = state.total.max(1);
    let pct = ((done as f32 / total as f32) * 100.0).round() as u32;
    let filled = ((done as usize) * 12) / (total as usize);
    let bar = format!(
        "{}{}",
        theme::success(&"█".repeat(filled)),
        theme::dim(&"░".repeat(12usize.saturating_sub(filled)))
    );
    let mode = state.mode.as_deref().unwrap_or("workflow");
    let line = format!(
        "  {} {} {}",
        theme::muted(mode),
        bar,
        theme::muted(&format!("{done}/{total} ({pct}%)"))
    );
    vec![truncate_to_width(&line, width, Some("…"))]
}

#[cfg(test)]
#[path = "subagent_inspector_tests.rs"]
mod tests;
