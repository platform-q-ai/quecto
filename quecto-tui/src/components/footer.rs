//! Footer component — status bar showing model, context, git branch.

use crate::component::Component;
use crate::theme;
use crate::utils::{truncate_to_width, visible_width};

/// Footer status bar.
pub struct Footer {
    model: String,
    git_branch: Option<String>,
    session_name: Option<String>,
    context_percent: Option<f64>,
    context_window: usize,
    is_streaming: bool,
    /// Cached working directory (read once at construction).
    pwd: String,
}

impl Footer {
    pub fn new() -> Self {
        let mut pwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".to_string());
        if let Ok(home) = std::env::var("HOME") {
            if pwd.starts_with(&home) {
                pwd = format!("~{}", &pwd[home.len()..]);
            }
        }
        Self {
            model: "unknown".to_string(),
            git_branch: None,
            session_name: None,
            context_percent: None,
            context_window: 0,
            is_streaming: false,
            pwd,
        }
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }

    pub fn set_git_branch(&mut self, branch: Option<String>) {
        self.git_branch = branch;
    }

    pub fn set_session_name(&mut self, name: Option<String>) {
        self.session_name = name;
    }

    pub fn set_context(&mut self, percent: Option<f64>, window: usize) {
        self.context_percent = percent;
        self.context_window = window;
    }

    pub fn set_streaming(&mut self, streaming: bool) {
        self.is_streaming = streaming;
    }
}

impl Component for Footer {
    fn render(&mut self, width: usize) -> Vec<String> {
        // Line 1: pwd + git branch + session name
        let mut pwd = self.pwd.clone();

        if let Some(branch) = &self.git_branch {
            pwd = format!("{} ({})", pwd, branch);
        }
        if let Some(name) = &self.session_name {
            pwd = format!("{} • {}", pwd, name);
        }

        let pwd_line = truncate_to_width(&theme::dim(&pwd), width, Some("..."));

        // Line 2: context usage (left) + model name (right)
        let context_str = match self.context_percent {
            Some(pct) => {
                let window = format_tokens(self.context_window);
                let display = format!("{:.1}%/{}", pct, window);
                if pct > 90.0 {
                    theme::error(&display)
                } else if pct > 70.0 {
                    theme::warning(&display)
                } else {
                    display
                }
            }
            None => format!("?/{}", format_tokens(self.context_window)),
        };

        let left = context_str;
        let right = &self.model;
        let left_width = visible_width(&left);
        let right_width = visible_width(right);
        let min_padding = 2;

        let stats_line = if left_width + min_padding + right_width <= width {
            let padding = " ".repeat(width - left_width - right_width);
            format!("{}{}{}", left, padding, right)
        } else {
            // Truncate right side
            let avail = width.saturating_sub(left_width + min_padding);
            if avail > 0 {
                let truncated = truncate_to_width(right, avail, Some(""));
                let trunc_width = visible_width(&truncated);
                let padding = " ".repeat(width.saturating_sub(left_width + trunc_width));
                format!("{}{}{}", left, padding, truncated)
            } else {
                truncate_to_width(&left, width, None)
            }
        };

        let stats_styled = theme::dim(&stats_line);

        vec![pwd_line, stats_styled]
    }

    fn invalidate(&mut self) {}
}

fn format_tokens(count: usize) -> String {
    if count == 0 {
        return "0".to_string();
    }
    if count < 1000 {
        return count.to_string();
    }
    if count < 10_000 {
        return format!("{:.1}k", count as f64 / 1000.0);
    }
    if count < 1_000_000 {
        return format!("{}k", count / 1000);
    }
    format!("{:.1}M", count as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_shows_model() {
        let mut f = Footer::new();
        f.set_model("claude-sonnet-4-20250514");
        let lines = f.render(80);
        let joined = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(
            plain.contains("claude-sonnet-4-20250514"),
            "should contain model: {}",
            plain
        );
    }

    #[test]
    fn footer_shows_git_branch() {
        let mut f = Footer::new();
        f.set_git_branch(Some("main".to_string()));
        let lines = f.render(80);
        let joined = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(plain.contains("main"), "should contain branch: {}", plain);
    }

    #[test]
    fn footer_respects_width() {
        let mut f = Footer::new();
        f.set_model("very-long-model-name-that-might-overflow");
        let lines = f.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "footer line exceeds width: {} (width={})",
                line,
                visible_width(line)
            );
        }
    }

    #[test]
    fn format_tokens_various() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1500), "1.5k");
        assert_eq!(format_tokens(15000), "15k");
        assert_eq!(format_tokens(1500000), "1.5M");
    }

    #[test]
    fn format_tokens_edge_cases() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1.0k");
        assert_eq!(format_tokens(9999), "10.0k");
        assert_eq!(format_tokens(10000), "10k");
        assert_eq!(format_tokens(999_999), "999k");
        assert_eq!(format_tokens(1_000_000), "1.0M");
    }

    #[test]
    fn footer_shows_session_name() {
        let mut f = Footer::new();
        f.set_session_name(Some("my-session".to_string()));
        let lines = f.render(80);
        let joined = lines.join("\n");
        assert!(joined.contains("my-session"));
    }

    #[test]
    fn footer_shows_context_percent() {
        let mut f = Footer::new();
        f.set_context(Some(42.5), 200_000);
        let lines = f.render(80);
        let joined = lines.join("\n");
        assert!(joined.contains("42.5%"));
    }

    #[test]
    fn footer_context_high_usage_warning() {
        let mut f = Footer::new();
        f.set_context(Some(75.0), 200_000);
        let lines = f.render(80);
        let joined = lines.join("");
        // Should contain warning color (yellow)
        assert!(joined.contains("75.0%"));
    }

    #[test]
    fn footer_context_critical_usage_error() {
        let mut f = Footer::new();
        f.set_context(Some(95.0), 200_000);
        let lines = f.render(80);
        let joined = lines.join("");
        assert!(joined.contains("95.0%"));
    }

    #[test]
    fn footer_no_context() {
        let mut f = Footer::new();
        f.set_context(None, 200_000);
        let lines = f.render(80);
        let joined = lines.join("");
        assert!(joined.contains("?/200k"));
    }

    #[test]
    fn footer_narrow_width_truncates() {
        let mut f = Footer::new();
        f.set_model("very-long-model-name-that-will-be-truncated");
        f.set_context(Some(50.0), 200_000);
        let lines = f.render(20);
        for line in &lines {
            assert!(visible_width(line) <= 20);
        }
    }

    #[test]
    fn footer_streaming_flag() {
        let mut f = Footer::new();
        f.set_streaming(true);
        assert!(f.is_streaming);
        f.set_streaming(false);
        assert!(!f.is_streaming);
    }

    #[test]
    fn footer_invalidate_no_panic() {
        let mut f = Footer::new();
        f.invalidate(); // should not panic
        assert_eq!(f.render(80).len(), 2);
    }

    #[test]
    fn footer_all_fields_set() {
        let mut f = Footer::new();
        f.set_model("gpt-4o");
        f.set_git_branch(Some("feature".to_string()));
        f.set_session_name(Some("sess-1".to_string()));
        f.set_context(Some(30.0), 128_000);
        f.set_streaming(true);
        let lines = f.render(100);
        assert_eq!(lines.len(), 2);
        let joined = lines.join("\n");
        assert!(joined.contains("feature"));
        assert!(joined.contains("sess-1"));
    }

    #[test]
    fn footer_extremely_narrow() {
        let mut f = Footer::new();
        f.set_model("m");
        f.set_context(Some(50.0), 100);
        let lines = f.render(5);
        for line in &lines {
            assert!(visible_width(line) <= 5);
        }
    }
}
