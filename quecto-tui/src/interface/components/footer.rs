//! Footer component — status bar showing model, context, git branch.

use std::borrow::Cow;

use crate::interface::component::Component;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

/// Footer status bar.
pub struct Footer {
    model: String,
    git_branch: Option<String>,
    context_percent: Option<f64>,
    context_window: usize,
    /// Tokens currently occupying the context window (last turn's input count).
    /// `None` until the first usage report arrives.
    context_used: Option<u64>,
    /// Cumulative session cost in USD. `None` until the first stats report.
    session_cost: Option<f64>,
    is_streaming: bool,
    /// Cached working directory (read once at construction).
    pwd: String,
}

impl Default for Footer {
    fn default() -> Self {
        Self::new()
    }
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
            context_percent: None,
            context_window: 0,
            context_used: None,
            session_cost: None,
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

    pub fn set_context(&mut self, percent: Option<f64>, window: usize) {
        self.context_percent = percent;
        self.context_window = window;
        // Callers using set_context don't supply a token count (None resets the
        // display to the percent-only / unknown form).
        self.context_used = None;
    }

    pub fn set_context_window(&mut self, window: usize) {
        self.context_window = window;
    }

    pub fn update_context_usage(&mut self, input_tokens: u64, window: usize) {
        let pct = if window > 0 {
            Some((input_tokens as f64 / window as f64) * 100.0)
        } else {
            None
        };
        self.context_percent = pct;
        self.context_window = window;
        self.context_used = Some(input_tokens);
    }

    pub fn set_streaming(&mut self, streaming: bool) {
        self.is_streaming = streaming;
    }

    /// Record cumulative session cost (USD). `None` hides the indicator.
    pub fn set_cost(&mut self, cost: Option<f64>) {
        self.session_cost = cost;
    }

    /// Apply a `get_state` payload's model + context-window to this footer.
    /// Returns the sanitized model id when present so callers can track it.
    /// Single source of truth for the get_state→footer mapping shared by the
    /// master footer path and per-session sub-agent footers (#805).
    pub fn apply_get_state(&mut self, data: &serde_json::Value) -> Option<String> {
        let model = data.get("model").and_then(|m| m.as_str()).map(|m| {
            let sanitized = crate::interface::ansi::sanitize_control(m);
            self.set_model(&sanitized);
            sanitized
        });
        if let Some(max_ctx) = data.get("maxContextTokens").and_then(|v| v.as_u64()) {
            self.set_context_window(max_ctx as usize);
        }
        model
    }

    /// Apply a parsed session-stats snapshot to the context + cost gauges.
    /// Single source of truth for the stats→footer mapping shared by the master
    /// footer path and per-session sub-agent footers (#805) — keeps the cost
    /// gate (`cost > 0`) from drifting between the two.
    pub fn apply_session_stats(
        &mut self,
        stats: &crate::application::session_payloads::SessionStats,
    ) {
        if let Some((used, window)) = stats.context_usage {
            self.update_context_usage(used, window);
        }
        self.set_cost((stats.cost > 0.0).then_some(stats.cost));
    }
}

impl Component for Footer {
    fn render(&mut self, width: usize) -> Vec<String> {
        // Line 1: pwd + git branch
        let mut pwd = self.pwd.clone();

        if let Some(branch) = &self.git_branch {
            pwd = format!("{} ({})", pwd, branch);
        }

        let pwd_line = truncate_to_width(&theme::dim(&pwd), width, Some("..."));

        // Line 2: context usage (left) + model name (right).
        // Preferred form shows tokens-used / model-limit and the percentage,
        // e.g. "123k/200k (61.5%)". Falls back to percent-only, then to
        // "?/limit" when neither usage nor a percentage is known yet.
        let window = format_tokens(self.context_window);
        let context_str = match (self.context_used, self.context_percent) {
            (Some(used), Some(pct)) => {
                let display = format!("{}/{} ({:.1}%)", format_tokens(used as usize), window, pct);
                colorize_usage(display, pct)
            }
            (_, Some(pct)) => colorize_usage(format!("{:.1}%/{}", pct, window), pct),
            (_, None) => format!("?/{}", window),
        };

        let left = match self.session_cost {
            // Append the cumulative session cost after the context %, e.g.
            // "123k/200k (61.5%) · $0.0421".
            Some(cost) if cost > 0.0 => format!("{} · ${:.4}", context_str, cost),
            _ => context_str,
        };
        // Prefix the model with a streaming indicator while a response is
        // streaming so the toggled flag is actually visible (issue #760). Borrow
        // the model unchanged in the dominant idle path to avoid a per-frame
        // allocation.
        let right: Cow<str> = if self.is_streaming {
            Cow::Owned(format!("{} {}", theme::STREAMING_INDICATOR, self.model))
        } else {
            Cow::Borrowed(&self.model)
        };
        let right = right.as_ref();
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

/// Color a context-usage string by how full the window is: red past 90%,
/// yellow past 70%, plain otherwise.
fn colorize_usage(display: String, pct: f64) -> String {
    if pct > 90.0 {
        theme::error(&display)
    } else if pct > 70.0 {
        theme::warning(&display)
    } else {
        display
    }
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
        f.set_model("claude-sonnet-4-6");
        let lines = f.render(80);
        let joined = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(
            plain.contains("claude-sonnet-4-6"),
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
    fn footer_shows_cost_after_context() {
        let mut f = Footer::new();
        f.update_context_usage(120_000, 200_000);
        f.set_cost(Some(0.0421));
        let lines = f.render(120);
        let joined = lines.join("\n");
        assert!(joined.contains("120k/200k"), "context first: {joined}");
        assert!(joined.contains("$0.0421"), "cost shown: {joined}");
        // Cost must come after the context usage in the rendered line.
        let ctx_idx = joined.find("120k/200k").unwrap();
        let cost_idx = joined.find("$0.0421").unwrap();
        assert!(cost_idx > ctx_idx, "cost should follow context: {joined}");
    }

    #[test]
    fn footer_hides_zero_cost() {
        let mut f = Footer::new();
        f.update_context_usage(120_000, 200_000);
        f.set_cost(Some(0.0));
        let joined = f.render(120).join("\n");
        assert!(!joined.contains('$'), "zero cost hidden: {joined}");
    }

    #[test]
    fn footer_hides_cost_when_none() {
        let mut f = Footer::new();
        f.update_context_usage(120_000, 200_000);
        let joined = f.render(120).join("\n");
        assert!(!joined.contains('$'), "no cost by default: {joined}");
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
    fn footer_shows_used_tokens_and_limit() {
        let mut f = Footer::new();
        // 120k of a 200k window → 60.0%.
        f.update_context_usage(120_000, 200_000);
        let lines = f.render(80);
        let joined = lines.join("\n");
        assert!(
            joined.contains("120k/200k"),
            "should show used/limit: {joined}"
        );
        assert!(joined.contains("60.0%"), "should show percent: {joined}");
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
        f.set_context(None, 0);
        let lines = f.render(80);
        let joined = lines.join("");
        assert!(joined.contains("?/0"));
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

    /// The streaming flag must be reflected in `render()` (issue #760): the four
    /// production callers of `set_streaming` previously toggled a write-only
    /// field that nothing rendered. Streaming must show a spinner indicator.
    #[test]
    fn footer_renders_streaming_indicator() {
        let mut f = Footer::new();
        f.set_model("claude-sonnet-4-6");
        f.set_streaming(true);
        let joined = f.render(80).join("\n");
        assert!(
            joined.contains(theme::STREAMING_INDICATOR),
            "streaming footer should render a streaming indicator: {joined:?}"
        );
    }

    #[test]
    fn footer_hides_streaming_indicator_when_idle() {
        let mut f = Footer::new();
        f.set_model("claude-sonnet-4-6");
        f.set_streaming(false);
        let joined = f.render(80).join("\n");
        assert!(
            !joined.contains(theme::STREAMING_INDICATOR),
            "idle footer should not render a streaming indicator: {joined:?}"
        );
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
        f.set_context(Some(30.0), 128_000);
        f.set_streaming(true);
        let lines = f.render(100);
        assert_eq!(lines.len(), 2);
        let joined = lines.join("\n");
        assert!(joined.contains("feature"));
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
