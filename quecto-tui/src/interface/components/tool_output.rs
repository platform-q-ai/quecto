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

/// Strip carriage return characters from tool output content (#529).
///
/// Git and other tools write progress output using bare `\r` (carriage return)
/// to overwrite the same line in a real terminal. When captured, these `\r`
/// characters cause the TUI renderer to produce black line artefacts.
///
/// This function:
/// 1. Removes trailing `\r` before `\n` (i.e. normalizes `\r\n` → `\n`)
/// 2. For bare `\r` within a line (progress overwrites), keeps only the
///    content after the last `\r` on each logical line
fn strip_carriage_returns(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for line in s.split('\n') {
        if !result.is_empty() {
            result.push('\n');
        }
        // Strip trailing \r (Windows-style line endings: \r\n → \n)
        let trimmed = line.strip_suffix('\r').unwrap_or(line);
        // Handle bare \r within the remaining line (progress overwrites):
        // keep only the content after the last \r
        if let Some(last_cr) = trimmed.rfind('\r') {
            result.push_str(&trimmed[last_cr + 1..]);
        } else {
            result.push_str(trimmed);
        }
    }
    result
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
        // Strip carriage returns from content to prevent rendering artefacts (#529).
        let cleaned = ToolResult {
            content: strip_carriage_returns(&result.content),
            ..result
        };
        self.result = Some(cleaned);
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
    #[allow(clippy::cognitive_complexity)]
    fn render(&mut self, width: usize) -> Vec<String> {
        if let (Some(cw), Some(cl)) = (self.cached_width, &self.cached_lines) {
            if cw == width {
                return cl.clone();
            }
        }

        let mut lines = Vec::new();

        // Box indent and top border.
        let box_indent = "  ";
        let border_width = width.saturating_sub(2);
        let top_border = format!("{}{}", box_indent, theme::dim(&"─".repeat(border_width)));
        lines.push(truncate_to_width(&top_border, width, None));

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
                let max_display_lines = 200;
                let total_lines = content.lines().count();

                for line in content.lines().take(max_display_lines) {
                    let styled = if is_diff {
                        style_diff_line(line)
                    } else if result.is_error {
                        theme::error(line)
                    } else {
                        theme::dim(line)
                    };
                    lines.push(truncate_to_width(&format!("    {}", styled), width, None));
                }
                if total_lines > max_display_lines {
                    lines.push(truncate_to_width(
                        &format!(
                            "    {}",
                            theme::dim(&format!(
                                "... ({} more lines)",
                                total_lines - max_display_lines
                            ))
                        ),
                        width,
                        None,
                    ));
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

        // Bottom border (only when tool is completed).
        if !self.is_running {
            let bottom_border = format!("{}{}", box_indent, theme::dim(&"─".repeat(border_width)));
            lines.push(truncate_to_width(&bottom_border, width, None));
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

/// Summarize tool arguments for display (truncated, single-line, sanitized).
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
                return sanitize_and_truncate(val, 80);
            }
        }
    }

    // Fallback: truncate raw args.
    sanitize_and_truncate(trimmed, 80)
}

/// Sanitize text for safe terminal display and truncate.
///
/// Strips ANSI escape sequences and control characters to prevent
/// terminal injection via LLM-controlled tool arguments.
fn sanitize_and_truncate(s: &str, max_chars: usize) -> String {
    let clean: String = s
        .chars()
        .filter(|&c| c >= '\u{0020}' && c != '\u{007F}')
        .collect();
    let truncated: String = clean.chars().take(max_chars).collect();
    if clean.chars().count() > max_chars {
        format!("{}...", truncated)
    } else {
        truncated
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

    // ── Carriage return stripping tests (issue #529) ──────────────────

    #[test]
    fn strip_cr_from_crlf() {
        assert_eq!(
            strip_carriage_returns("line1\r\nline2\r\n"),
            "line1\nline2\n"
        );
    }

    #[test]
    fn strip_bare_cr_keeps_last_segment() {
        // Git progress: "Counting: 5\rCounting: 10\rDone"
        assert_eq!(
            strip_carriage_returns("Counting: 5\rCounting: 10\rDone"),
            "Done"
        );
    }

    #[test]
    fn strip_cr_multiline_mixed() {
        let input = "origin\thttps://example.com (fetch)\r\norigin\thttps://example.com (push)\r\n";
        let output = strip_carriage_returns(input);
        assert_eq!(
            output,
            "origin\thttps://example.com (fetch)\norigin\thttps://example.com (push)\n"
        );
    }

    #[test]
    fn strip_cr_no_cr_unchanged() {
        let input = "file1.txt\nfile2.txt\nfile3.txt";
        assert_eq!(strip_carriage_returns(input), input);
    }

    #[test]
    fn strip_cr_empty_string() {
        assert_eq!(strip_carriage_returns(""), "");
    }

    #[test]
    fn strip_cr_only_cr() {
        assert_eq!(strip_carriage_returns("\r"), "");
    }

    #[test]
    fn strip_cr_mixed_progress_and_normal() {
        let input = "Working...\rDone!\nResults: 5 files\n";
        let output = strip_carriage_returns(input);
        assert_eq!(output, "Done!\nResults: 5 files\n");
    }

    #[test]
    fn tool_output_strips_cr_on_set_result() {
        let mut t = ToolOutput::new("bash", r#"{"command": "git remote -v"}"#);
        t.set_result(ToolResult {
            content: "origin\thttps://ex.com (fetch)\r\norigin\thttps://ex.com (push)\r\n"
                .to_string(),
            is_error: false,
            duration_ms: Some(10),
        });
        t.set_expanded(true);
        let lines = t.render(80);
        for line in &lines {
            assert!(
                !line.contains('\r'),
                "rendered line should not contain CR: {:?}",
                line
            );
        }
    }

    // ── Box rendering tests (issue #473) ──────────────────────────────

    #[test]
    fn running_tool_has_top_border() {
        let mut t = ToolOutput::new("bash", r#"{"command": "ls"}"#);
        let lines = t.render(80);
        let joined: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains('─'),
            "should contain box border: {}",
            joined
        );
    }

    #[test]
    fn completed_tool_has_bottom_border() {
        let mut t = ToolOutput::new("bash", r#"{"command": "ls"}"#);
        t.set_result(ToolResult {
            content: "file.txt".to_string(),
            is_error: false,
            duration_ms: Some(10),
        });
        let lines = t.render(80);
        let last = lines
            .iter()
            .rev()
            .find(|l| !strip_ansi(l).trim().is_empty())
            .map(|l| strip_ansi(l))
            .unwrap_or_default();
        assert!(last.contains('─'), "last line should be border: {}", last);
    }

    #[test]
    fn box_respects_width() {
        let mut t = ToolOutput::new("bash", r#"{"command": "very long command"}"#);
        t.set_result(ToolResult {
            content: "long result text here".to_string(),
            is_error: false,
            duration_ms: Some(999),
        });
        t.set_expanded(true);
        let lines = t.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "line exceeds width: '{}' (width={})",
                strip_ansi(line),
                visible_width(line)
            );
        }
    }
}
