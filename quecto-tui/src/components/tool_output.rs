//! Tool execution component — collapsible tool output with diff support.
//!
//! Shows tool name + args as a header, with expandable result body.
//! Edit tool results show colored diff (+/- lines).

use crate::component::Component;
use crate::theme;
use crate::utils::truncate_to_width;
#[cfg(test)]
use crate::utils::visible_width;

/// A rendered tool execution block.
pub struct ToolOutput {
    tool_name: String,
    args_summary: String,
    result: Option<ToolResult>,
    expanded: bool,
    is_running: bool,
    cached_width: Option<usize>,
    cached_lines: Option<Vec<String>>,
}

/// Result of a tool execution.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    pub duration_ms: Option<u64>,
}

impl ToolOutput {
    pub fn new(tool_name: &str, args: &str) -> Self {
        let args_summary = summarize_args(args);
        Self {
            tool_name: tool_name.to_string(),
            args_summary,
            result: None,
            expanded: false,
            is_running: true,
            cached_width: None,
            cached_lines: None,
        }
    }

    pub fn set_result(&mut self, result: ToolResult) {
        self.result = Some(result);
        self.is_running = false;
        self.invalidate();
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        if self.expanded != expanded {
            self.expanded = expanded;
            self.invalidate();
        }
    }

    pub fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
        self.invalidate();
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }
}

impl Component for ToolOutput {
    fn render(&mut self, width: usize) -> Vec<String> {
        if let (Some(cw), Some(cl)) = (self.cached_width, &self.cached_lines) {
            if cw == width {
                return cl.clone();
            }
        }

        let mut lines = Vec::new();

        // Header: icon + tool name + args
        let (icon, icon_style): (&str, fn(&str) -> String) = if self.is_running {
            ("⠋", theme::spinner)
        } else if self.result.as_ref().is_some_and(|r| r.is_error) {
            ("✗", theme::error)
        } else {
            ("✓", theme::success)
        };

        let dur_str = self
            .result
            .as_ref()
            .and_then(|r| r.duration_ms.map(|ms| format!("  {}ms", ms)))
            .unwrap_or_default();

        let expand_indicator = if self.result.is_some() && !self.is_running {
            if self.expanded { "▼ " } else { "▶ " }
        } else {
            "  "
        };

        let header = format!(
            "  {}{} {} {}{}",
            theme::dim(expand_indicator),
            icon_style(icon),
            theme::tool_name(&self.tool_name),
            theme::dim(&self.args_summary),
            theme::dim(&dur_str),
        );
        lines.push(truncate_to_width(&header, width, None));

        // Result body (if expanded or running).
        if let Some(result) = &self.result {
            if self.expanded {
                let content = &result.content;
                let is_diff = is_diff_content(content);

                for line in content.lines() {
                    let styled = if is_diff {
                        style_diff_line(line)
                    } else if result.is_error {
                        theme::error(line)
                    } else {
                        theme::dim(line)
                    };
                    lines.push(truncate_to_width(&format!("    {}", styled), width, None));
                }
            } else if !result.content.is_empty() {
                // Collapsed: show first line preview.
                let first_line = result.content.lines().next().unwrap_or("");
                let total_lines = result.content.lines().count();
                let preview = if total_lines > 1 {
                    format!("{} (+{} lines)", first_line, total_lines - 1)
                } else {
                    first_line.to_string()
                };
                let color: fn(&str) -> String = if result.is_error {
                    theme::error
                } else {
                    theme::dim
                };
                lines.push(truncate_to_width(
                    &format!("    {}", color(&preview)),
                    width,
                    None,
                ));
            }
        }

        self.cached_width = Some(width);
        self.cached_lines = Some(lines.clone());
        lines
    }

    fn invalidate(&mut self) {
        self.cached_width = None;
        self.cached_lines = None;
    }
}

/// Summarize tool arguments for display (truncated, single-line).
fn summarize_args(args: &str) -> String {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Try to extract the most useful field from JSON args.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        // Common patterns: {"command": "..."}, {"path": "..."}, {"query": "..."}
        for key in &["command", "path", "query", "url", "content"] {
            if let Some(val) = v.get(key).and_then(|v| v.as_str()) {
                let s: String = val.chars().take(80).collect();
                if val.chars().count() > 80 {
                    return format!("{}...", s);
                }
                return s;
            }
        }
    }

    // Fallback: truncate raw args.
    let s: String = trimmed.chars().take(80).collect();
    if trimmed.chars().count() > 80 {
        format!("{}...", s)
    } else {
        s
    }
}

/// Check if content looks like a diff (has +/- prefixed lines).
fn is_diff_content(content: &str) -> bool {
    let mut has_add = false;
    let mut has_remove = false;
    for line in content.lines().take(20) {
        if line.starts_with('+') && !line.starts_with("+++") {
            has_add = true;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            has_remove = true;
        }
        if has_add && has_remove {
            return true;
        }
    }
    false
}

/// Style a single diff line with color.
fn style_diff_line(line: &str) -> String {
    if line.starts_with('+') {
        theme::green(line)
    } else if line.starts_with('-') {
        theme::red(line)
    } else if line.starts_with("@@") {
        theme::cyan(line)
    } else {
        theme::dim(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        let mut result = String::new();
        let mut in_escape = false;
        for ch in s.chars() {
            if in_escape {
                if ch.is_ascii_alphabetic() || ch == '~' {
                    in_escape = false;
                }
            } else if ch == '\x1b' {
                in_escape = true;
            } else {
                result.push(ch);
            }
        }
        result
    }

    #[test]
    fn running_tool_shows_spinner() {
        let mut t = ToolOutput::new("bash", r#"{"command": "ls -la"}"#);
        let lines = t.render(80);
        let joined: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("bash"), "should show tool name: {}", joined);
        assert!(joined.contains("ls -la"), "should show args: {}", joined);
    }

    #[test]
    fn completed_tool_shows_result_collapsed() {
        let mut t = ToolOutput::new("bash", r#"{"command": "ls"}"#);
        t.set_result(ToolResult {
            content: "file1.txt\nfile2.txt\nfile3.txt".to_string(),
            is_error: false,
            duration_ms: Some(42),
        });
        let lines = t.render(80);
        let joined: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("bash"), "should show tool name");
        assert!(joined.contains("42ms"), "should show duration");
        assert!(
            joined.contains("file1.txt"),
            "should show first line preview"
        );
        assert!(
            joined.contains("+2 lines"),
            "should show line count: {}",
            joined
        );
    }

    #[test]
    fn expanded_tool_shows_all_lines() {
        let mut t = ToolOutput::new("bash", r#"{"command": "ls"}"#);
        t.set_result(ToolResult {
            content: "file1.txt\nfile2.txt\nfile3.txt".to_string(),
            is_error: false,
            duration_ms: None,
        });
        t.set_expanded(true);
        let lines = t.render(80);
        let joined: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("file1.txt"));
        assert!(joined.contains("file2.txt"));
        assert!(joined.contains("file3.txt"));
    }

    #[test]
    fn error_tool_styled() {
        let mut t = ToolOutput::new("bash", "{}");
        t.set_result(ToolResult {
            content: "command not found".to_string(),
            is_error: true,
            duration_ms: None,
        });
        t.set_expanded(true);
        let lines = t.render(80);
        let joined = lines.join("");
        // Error styling uses \x1b[31m (red).
        assert!(
            joined.contains("\x1b[31m"),
            "should contain red styling: {}",
            joined
        );
    }

    #[test]
    fn diff_content_detected() {
        assert!(is_diff_content("+added\n-removed\n context"));
        assert!(!is_diff_content("just normal text\nno diff here"));
    }

    #[test]
    fn diff_lines_colored() {
        let mut t = ToolOutput::new("edit", "{}");
        t.set_result(ToolResult {
            content: "+added line\n-removed line\n context".to_string(),
            is_error: false,
            duration_ms: None,
        });
        t.set_expanded(true);
        let lines = t.render(80);
        let joined = lines.join("");
        // Green for added: \x1b[32m, Red for removed: \x1b[31m
        assert!(joined.contains("\x1b[32m"), "added should be green");
        assert!(joined.contains("\x1b[31m"), "removed should be red");
    }

    #[test]
    fn toggle_expanded() {
        let mut t = ToolOutput::new("bash", "{}");
        assert!(!t.is_expanded());
        t.toggle_expanded();
        assert!(t.is_expanded());
        t.toggle_expanded();
        assert!(!t.is_expanded());
    }

    #[test]
    fn summarize_args_extracts_command() {
        let s = summarize_args(r#"{"command": "ls -la /tmp"}"#);
        assert_eq!(s, "ls -la /tmp");
    }

    #[test]
    fn summarize_args_extracts_path() {
        let s = summarize_args(r#"{"path": "/home/user/file.rs"}"#);
        assert_eq!(s, "/home/user/file.rs");
    }

    #[test]
    fn summarize_args_truncates_long() {
        let long = "a".repeat(100);
        let args = format!(r#"{{"command": "{}"}}"#, long);
        let s = summarize_args(&args);
        assert!(s.len() <= 84); // 80 + "..."
        assert!(s.ends_with("..."));
    }

    #[test]
    fn respects_width() {
        let mut t = ToolOutput::new("bash", r#"{"command": "very long command here"}"#);
        t.set_result(ToolResult {
            content: "some result".to_string(),
            is_error: false,
            duration_ms: Some(100),
        });
        t.set_expanded(true);
        let lines = t.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "line exceeds width: {} (width={})",
                line,
                visible_width(line)
            );
        }
    }
}
