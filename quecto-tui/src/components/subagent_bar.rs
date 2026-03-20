//! Subagent status bar widget (#525).
//!
//! Renders one line per spawned subagent, showing a progress-bar style
//! indicator with live status. Hidden when no subagents exist.
//!
//! Visual:
//! ```text
//!   reviewer  [████████░░░░░░] Running · bash
//!   formatter [██████████████] Idle
//! ```

use crate::component::Component;
use crate::theme;

/// Wire-format info for a single subagent (matches #524 protocol).
#[derive(Debug, Clone)]
pub struct SubagentInfoWire {
    pub agent_id: String,
    pub status: String,
    pub last_tool: Option<String>,
    pub last_error: Option<String>,
}

/// Widget that renders live subagent status bars.
#[derive(Debug)]
pub struct SubagentBar {
    agents: Vec<SubagentInfoWire>,
    cache: Option<Vec<String>>,
}

impl SubagentBar {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            cache: None,
        }
    }

    /// Update the agent list (full replacement).
    pub fn update(&mut self, agents: Vec<SubagentInfoWire>) {
        self.agents = agents;
        self.cache = None;
    }

    /// Whether there are any agents to display.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Format a single agent line for the given width.
    fn format_agent(info: &SubagentInfoWire, name_width: usize, bar_width: usize) -> String {
        let name = pad_right(&info.agent_id, name_width);
        let bar = render_bar(&info.status, bar_width);
        let status_label = status_styled(&info.status);
        let context = agent_context(info);
        if context.is_empty() {
            format!("  {} {} {}", name, bar, status_label)
        } else {
            format!("  {} {} {} · {}", name, bar, status_label, context)
        }
    }
}

impl Default for SubagentBar {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for SubagentBar {
    fn render(&mut self, width: usize) -> Vec<String> {
        if self.agents.is_empty() {
            return Vec::new();
        }
        if let Some(ref cache) = self.cache {
            return cache.clone();
        }
        let name_width = self
            .agents
            .iter()
            .map(|a| a.agent_id.len())
            .max()
            .unwrap_or(0)
            .max(4);
        // Bar width: 14 chars default, shrink if terminal is narrow
        let bar_width = if width > 60 { 14 } else { 8 };
        let lines: Vec<String> = self
            .agents
            .iter()
            .map(|a| SubagentBar::format_agent(a, name_width, bar_width))
            .collect();
        self.cache = Some(lines.clone());
        lines
    }

    fn invalidate(&mut self) {
        self.cache = None;
    }
}

/// Pad a string to the given width with spaces.
fn pad_right(s: &str, width: usize) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - s.len()))
    }
}

/// Render a progress bar for the given status.
fn render_bar(status: &str, width: usize) -> String {
    let (fill, empty) = match status {
        "running" => (width / 2, width - width / 2),
        "idle" => (width, 0),
        "error" => (width / 3, width - width / 3),
        "starting" => (1.min(width), width.saturating_sub(1)),
        "exited" => (0, width),
        _ => (0, width),
    };
    let fill_str = "█".repeat(fill);
    let empty_str = "░".repeat(empty);
    let bar_content = format!("{}{}", fill_str, empty_str);
    let colored = match status {
        "running" => theme::blue(&bar_content),
        "idle" => theme::green(&bar_content),
        "error" => theme::red(&bar_content),
        "starting" => theme::dim(&bar_content),
        "exited" => theme::dim(&bar_content),
        _ => theme::dim(&bar_content),
    };
    format!("[{}]", colored)
}

/// Style the status label text.
fn status_styled(status: &str) -> String {
    match status {
        "running" => theme::blue("Running"),
        "idle" => theme::green("Idle"),
        "error" => theme::red("Error"),
        "starting" => theme::dim("Starting"),
        "exited" => theme::dim("Exited"),
        _ => theme::dim(status),
    }
}

/// Build the context string (tool name or error) after the `·` separator.
fn agent_context(info: &SubagentInfoWire) -> String {
    if info.status == "error" {
        if let Some(ref err) = info.last_error {
            return theme::red(&truncate(err, 40));
        }
    }
    if info.status == "running" {
        if let Some(ref tool) = info.last_tool {
            return theme::dim(&truncate(tool, 30));
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
    ) -> SubagentInfoWire {
        SubagentInfoWire {
            agent_id: id.to_string(),
            status: status.to_string(),
            last_tool: tool.map(|s| s.to_string()),
            last_error: error.map(|s| s.to_string()),
        }
    }

    #[test]
    fn empty_bar_renders_nothing() {
        let mut bar = SubagentBar::new();
        assert!(bar.render(80).is_empty());
    }

    #[test]
    fn single_running_agent() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("reviewer", "running", Some("bash"), None)]);
        let lines = bar.render(80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("reviewer"));
        assert!(lines[0].contains("Running"));
        assert!(lines[0].contains("bash"));
    }

    #[test]
    fn idle_agent_no_context() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("fmt", "idle", None, None)]);
        let lines = bar.render(80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Idle"));
        assert!(!lines[0].contains("·"));
    }

    #[test]
    fn error_agent_shows_error() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info(
            "lint",
            "error",
            None,
            Some("tool 'bash' returned error"),
        )]);
        let lines = bar.render(80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Error"));
        assert!(lines[0].contains("tool 'bash' returned error"));
    }

    #[test]
    fn multiple_agents_render_multiple_lines() {
        let mut bar = SubagentBar::new();
        bar.update(vec![
            make_info("a", "running", Some("read"), None),
            make_info("b", "idle", None, None),
            make_info("c", "exited", None, None),
        ]);
        let lines = bar.render(80);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn update_replaces_agents() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("a", "running", None, None)]);
        assert_eq!(bar.render(80).len(), 1);
        bar.update(vec![]);
        assert!(bar.render(80).is_empty());
    }

    #[test]
    fn cache_is_invalidated_on_update() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("a", "running", None, None)]);
        let _ = bar.render(80);
        assert!(bar.cache.is_some());
        bar.update(vec![make_info("b", "idle", None, None)]);
        assert!(bar.cache.is_none());
    }

    #[test]
    fn invalidate_clears_cache() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("a", "idle", None, None)]);
        let _ = bar.render(80);
        bar.invalidate();
        assert!(bar.cache.is_none());
    }

    #[test]
    fn narrow_terminal_still_renders() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("x", "running", Some("bash"), None)]);
        let lines = bar.render(30);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Running"));
    }

    #[test]
    fn starting_status() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("init", "starting", None, None)]);
        let lines = bar.render(80);
        assert!(lines[0].contains("Starting"));
    }

    #[test]
    fn exited_status() {
        let mut bar = SubagentBar::new();
        bar.update(vec![make_info("done", "exited", None, None)]);
        let lines = bar.render(80);
        assert!(lines[0].contains("Exited"));
    }

    #[test]
    fn truncate_long_context() {
        let long = "a".repeat(100);
        let result = truncate(&long, 40);
        assert!(result.chars().count() <= 41); // 40 + "…"
    }

    #[test]
    fn pad_right_pads_short_strings() {
        assert_eq!(pad_right("ab", 5), "ab   ");
    }

    #[test]
    fn pad_right_no_pad_when_at_width() {
        assert_eq!(pad_right("abc", 3), "abc");
    }

    #[test]
    fn render_bar_running() {
        let bar = render_bar("running", 14);
        assert!(bar.starts_with('['));
        assert!(bar.contains("█"));
        assert!(bar.contains("░"));
    }

    #[test]
    fn render_bar_idle_full() {
        let bar = render_bar("idle", 10);
        assert!(bar.contains("██████████"));
    }
}
