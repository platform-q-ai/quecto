//! Subagent status bar widget (#525).
//!
//! Renders a `▸ Subagents` panel header followed by one indented row per spawned
//! subagent: an animated spinner (or terminal-state glyph), the agent id, its
//! status, how long it has been alive, and a short context (current tool, or
//! error message). The header/indent layout and colour semantics
//! (green=done, cyan=active, red=error, dim=pending) mirror the workflow widget
//! so the two read as sibling panels. Hidden when no subagents exist.

use crate::infrastructure::client::SubagentInfoEvent;
use crate::interface::component::Component;
use crate::interface::components::sanitize::strip_terminal_control;
use crate::interface::theme;

/// Braille spinner frames — matches the main spinner (`components/spinner.rs`)
/// so a running subagent animates identically to the agent spinner.
use crate::interface::theme::SPINNER_FRAMES;

/// A single subagent's display row: wire info plus client-computed liveness.
#[derive(Debug, Clone)]
pub struct SubagentRow {
    pub info: SubagentInfoEvent,
    /// Seconds the agent has been alive (client-side clock; frozen on exit).
    pub elapsed_secs: u64,
}

impl SubagentRow {
    /// Convenience constructor used by callers and tests.
    pub fn new(info: SubagentInfoEvent, elapsed_secs: u64) -> Self {
        Self { info, elapsed_secs }
    }
}

/// Widget that renders live subagent status bars.
#[derive(Debug)]
pub struct SubagentBar {
    rows: Vec<SubagentRow>,
    /// Animation frame for running-agent spinners.
    frame: usize,
    /// Sub-agent currently being awaited by the parent, shown with a per-row
    /// "awaiting" indicator.
    awaited: Option<String>,
    cache: Option<Vec<String>>,
    cached_width: usize,
}

impl SubagentBar {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            frame: 0,
            awaited: None,
            cache: None,
            cached_width: 0,
        }
    }

    /// Update the agent rows and animation frame (full replacement).
    pub fn update(&mut self, rows: Vec<SubagentRow>, frame: usize) {
        self.rows = rows;
        self.frame = frame;
        self.cache = None;
    }

    /// Set which sub-agent (if any) the parent is awaiting.
    pub fn set_awaited(&mut self, awaited: Option<String>) {
        self.awaited = awaited;
        self.cache = None;
    }

    /// Whether there are any agents to display.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl Default for SubagentBar {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for SubagentBar {
    fn render(&mut self, width: usize) -> Vec<String> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        if width != self.cached_width {
            self.cache = None;
        }
        if let Some(ref cache) = self.cache {
            return cache.clone();
        }
        let name_width = self
            .rows
            .iter()
            .map(|r| r.info.agent_id.chars().count())
            .max()
            .unwrap_or(0)
            .clamp(4, 20);

        let mut lines: Vec<String> = Vec::with_capacity(self.rows.len() + 1);
        // Panel header (always present) gives the block an identity and mirrors
        // the workflow widget's `▸ Workflow` header for visual alignment.
        lines.push(header_line(&self.rows));
        lines.extend(self.rows.iter().map(|r| {
            let awaited = self.awaited.as_deref() == Some(r.info.agent_id.as_str());
            format_agent(r, name_width, self.frame, awaited)
        }));

        self.cache = Some(lines.clone());
        self.cached_width = width;
        lines
    }

    fn invalidate(&mut self) {
        self.cache = None;
    }
}

/// Strip terminal control sequences to prevent terminal escape injection.
fn sanitize(s: &str) -> String {
    strip_terminal_control(s)
}

/// Panel header: `▸ Subagents  X running · Y error · Z done`.
fn header_line(rows: &[SubagentRow]) -> String {
    let mut running = 0usize;
    let mut error = 0usize;
    let mut done = 0usize;
    for r in rows {
        match r.info.status.as_str() {
            "running" | "starting" => running += 1,
            "error" => error += 1,
            "idle" | "exited" => done += 1,
            _ => {}
        }
    }
    let mut counts = Vec::new();
    if running > 0 {
        counts.push(theme::accent(&format!("{running} running")));
    }
    if error > 0 {
        counts.push(theme::red(&format!("{error} error")));
    }
    if done > 0 {
        counts.push(theme::dim(&format!("{done} done")));
    }
    let title = theme::accent(&theme::bold("Subagents"));
    if counts.is_empty() {
        format!("  {} {}", theme::dim("▸"), title)
    } else {
        format!(
            "  {} {}  {}",
            theme::dim("▸"),
            title,
            counts.join(theme::dim(" · ").as_str())
        )
    }
}

/// Format a single agent row, indented to nest under the panel header.
fn format_agent(row: &SubagentRow, name_width: usize, frame: usize, awaited: bool) -> String {
    let info = &row.info;
    let glyph = leading_glyph(&info.status, frame);
    let name = pad_right(&sanitize(&info.agent_id), name_width);
    let status_label = status_styled(&info.status);
    let elapsed = theme::dim(&fmt_elapsed(row.elapsed_secs));

    // Trailing detail segments (joined with " · ") — current tool/error, then
    // the agent's own workflow progress (PRD Stage B snapshot).
    let mut segments: Vec<String> = Vec::new();
    let context = agent_context(info);
    if !context.is_empty() {
        segments.push(context);
    }
    if let Some(wf) = &info.workflow {
        segments.push(workflow_segment(wf));
    }

    let mut line = if segments.is_empty() {
        format!("    {} {} {} {}", glyph, name, status_label, elapsed)
    } else {
        format!(
            "    {} {} {} {} · {}",
            glyph,
            name,
            status_label,
            elapsed,
            segments.join(&theme::dim(" · "))
        )
    };
    // Per-row "awaiting" indicator for the agent the parent is blocked on.
    if awaited {
        let spin = theme::spinner(SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]);
        line.push_str(&format!("  {} {}", spin, theme::accent("awaiting")));
    }
    line
}

/// Compact one-line workflow progress for a sub-agent row, e.g. `wf active 3/5`.
fn workflow_segment(wf: &crate::infrastructure::client::SubagentWorkflow) -> String {
    theme::dim(&format!(
        "wf {} {}/{}",
        sanitize(&wf.mode),
        wf.steps_completed,
        wf.steps_total
    ))
}

/// Leading glyph: animated spinner while active, terminal glyph otherwise.
fn leading_glyph(status: &str, frame: usize) -> String {
    match status {
        "running" | "starting" => theme::spinner(SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]),
        "idle" => theme::green("✓"),
        "error" => theme::red("✗"),
        "exited" => theme::dim("•"),
        _ => theme::dim("·"),
    }
}

/// Pad a string to the given width with spaces.
fn pad_right(s: &str, width: usize) -> String {
    let vis_len = s.chars().count();
    if vis_len >= width {
        s.chars().take(width).collect()
    } else {
        format!("{}{}", s, " ".repeat(width - vis_len))
    }
}

/// Format an elapsed duration compactly: `12s`, `1m12s`, `2h05m`.
fn fmt_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Style the status label text.
fn status_styled(status: &str) -> String {
    match status {
        "running" => theme::accent("Running"),
        "idle" => theme::green("Idle"),
        "error" => theme::red("Error"),
        "starting" => theme::dim("Starting"),
        "exited" => theme::dim("Exited"),
        _ => theme::dim("Unknown"),
    }
}

/// Build the context string (tool name or error) after the `·` separator.
fn agent_context(info: &SubagentInfoEvent) -> String {
    if info.status == "error" {
        if let Some(ref err) = info.last_error {
            return theme::red(&sanitize(&truncate(err, 40)));
        }
    }
    if info.status == "running" {
        if let Some(ref tool) = info.last_tool {
            return theme::dim(&sanitize(&truncate(tool, 30)));
        }
    }
    String::new()
}

/// Truncate a string to max chars, appending "…" if truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map_or(s.len(), |(i, _)| i);
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_info(
        id: &str,
        status: &str,
        tool: Option<&str>,
        error: Option<&str>,
    ) -> SubagentInfoEvent {
        SubagentInfoEvent {
            agent_id: id.to_string(),
            status: status.to_string(),
            last_tool: tool.map(|s| s.to_string()),
            last_error: error.map(|s| s.to_string()),
            pid: 0,
            parent_id: None,
            workflow: None,
        }
    }

    fn row(id: &str, status: &str, tool: Option<&str>, error: Option<&str>) -> SubagentRow {
        SubagentRow::new(make_info(id, status, tool, error), 0)
    }

    fn update(bar: &mut SubagentBar, rows: Vec<SubagentRow>) {
        bar.update(rows, 0);
    }

    /// True if any rendered line contains the needle.
    fn any_contains(lines: &[String], needle: &str) -> bool {
        lines.iter().any(|l| l.contains(needle))
    }

    #[test]
    fn empty_bar_renders_nothing() {
        let mut bar = SubagentBar::new();
        assert!(bar.render(80).is_empty());
    }

    #[test]
    fn row_shows_workflow_snapshot() {
        let mut info = make_info("w", "running", None, None);
        info.workflow = Some(crate::infrastructure::client::SubagentWorkflow {
            mode: "active".into(),
            steps_completed: 3,
            steps_total: 5,
        });
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![SubagentRow::new(info, 0)]);
        assert!(any_contains(&bar.render(80), "wf active 3/5"));
    }

    #[test]
    fn awaited_agent_row_shows_awaiting_indicator() {
        let mut bar = SubagentBar::new();
        update(
            &mut bar,
            vec![
                row("busy", "running", None, None),
                row("other", "running", None, None),
            ],
        );
        bar.set_awaited(Some("busy".into()));
        let lines = bar.render(80);
        let busy = lines.iter().find(|l| l.contains("busy")).unwrap();
        assert!(
            busy.contains("awaiting"),
            "awaited row should show indicator"
        );
        let other = lines.iter().find(|l| l.contains("other")).unwrap();
        assert!(
            !other.contains("awaiting"),
            "non-awaited row must not show indicator"
        );
    }

    #[test]
    fn no_awaiting_indicator_without_await() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("w", "running", None, None)]);
        assert!(!any_contains(&bar.render(80), "awaiting"));
    }

    #[test]
    fn single_running_agent() {
        let mut bar = SubagentBar::new();
        update(
            &mut bar,
            vec![row("reviewer", "running", Some("bash"), None)],
        );
        let lines = bar.render(80);
        // panel header + 1 agent row
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("Subagents"));
        assert!(lines[1].contains("reviewer"));
        assert!(lines[1].contains("Running"));
        assert!(lines[1].contains("bash"));
    }

    #[test]
    fn idle_agent_no_context() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("fmt", "idle", None, None)]);
        let lines = bar.render(80);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Idle"));
        // no `·` context separator on the agent row
        assert!(!lines[1].contains("·"));
    }

    #[test]
    fn error_agent_shows_error() {
        let mut bar = SubagentBar::new();
        update(
            &mut bar,
            vec![row(
                "lint",
                "error",
                None,
                Some("tool 'bash' returned error"),
            )],
        );
        let lines = bar.render(80);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Error"));
        assert!(lines[1].contains("tool 'bash' returned error"));
    }

    #[test]
    fn multiple_agents_render_header_plus_lines() {
        let mut bar = SubagentBar::new();
        update(
            &mut bar,
            vec![
                row("a", "running", Some("read"), None),
                row("b", "idle", None, None),
                row("c", "exited", None, None),
            ],
        );
        let lines = bar.render(80);
        // header + 3 agents
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("Subagents"));
        assert!(lines[0].contains("1 running"));
        assert!(lines[0].contains("2 done"));
    }

    #[test]
    fn elapsed_is_rendered() {
        let mut bar = SubagentBar::new();
        bar.update(
            vec![SubagentRow::new(make_info("a", "running", None, None), 72)],
            0,
        );
        assert!(any_contains(&bar.render(80), "1m12s"));
    }

    #[test]
    fn running_agent_shows_spinner_frame() {
        let mut bar = SubagentBar::new();
        bar.update(
            vec![SubagentRow::new(make_info("a", "running", None, None), 0)],
            2,
        );
        assert!(any_contains(&bar.render(80), SPINNER_FRAMES[2]));
    }

    #[test]
    fn update_replaces_agents() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("a", "running", None, None)]);
        assert!(!bar.render(80).is_empty());
        update(&mut bar, vec![]);
        assert!(bar.render(80).is_empty());
    }

    #[test]
    fn cache_is_invalidated_on_update() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("a", "running", None, None)]);
        let _ = bar.render(80);
        assert!(bar.cache.is_some());
        update(&mut bar, vec![row("b", "idle", None, None)]);
        assert!(bar.cache.is_none());
    }

    #[test]
    fn invalidate_clears_cache() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("a", "idle", None, None)]);
        let _ = bar.render(80);
        bar.invalidate();
        assert!(bar.cache.is_none());
    }

    #[test]
    fn narrow_terminal_still_renders() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("x", "running", Some("bash"), None)]);
        let lines = bar.render(30);
        assert_eq!(lines.len(), 2);
        assert!(any_contains(&lines, "Running"));
    }

    #[test]
    fn starting_status() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("init", "starting", None, None)]);
        assert!(any_contains(&bar.render(80), "Starting"));
    }

    #[test]
    fn exited_status() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("done", "exited", None, None)]);
        assert!(any_contains(&bar.render(80), "Exited"));
    }

    #[test]
    fn fmt_elapsed_formats() {
        assert_eq!(fmt_elapsed(5), "5s");
        assert_eq!(fmt_elapsed(72), "1m12s");
        assert_eq!(fmt_elapsed(3 * 3600 + 5 * 60), "3h05m");
    }

    #[test]
    fn truncate_long_context() {
        let long = "a".repeat(100);
        let result = truncate(&long, 40);
        assert!(result.chars().count() <= 41);
    }

    #[test]
    fn pad_right_pads_short_strings() {
        assert_eq!(pad_right("ab", 5), "ab   ");
    }

    #[test]
    fn pad_right_truncates_long() {
        assert_eq!(pad_right("abcdef", 3), "abc");
    }

    #[test]
    fn sanitize_strips_control_chars() {
        assert_eq!(sanitize("ab\x1b[31mcd"), "abcd");
    }

    #[test]
    fn cache_invalidated_on_width_change() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("a", "idle", None, None)]);
        let _ = bar.render(80);
        assert!(bar.cache.is_some());
        let _ = bar.render(40); // different width
        assert_eq!(bar.cached_width, 40);
    }

    #[test]
    fn unknown_status_shows_unknown() {
        let mut bar = SubagentBar::new();
        update(&mut bar, vec![row("x", "weird", None, None)]);
        assert!(any_contains(&bar.render(80), "Unknown"));
    }
}
