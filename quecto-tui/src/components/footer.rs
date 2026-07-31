//! Footer component — status bar showing model, context, git branch.

use crate::components::component::Component;
use crate::components::theme;
use crate::components::utils::{truncate_to_width, visible_width};

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
    /// Active reasoning-effort level (#1067); `None` = effective default.
    effort: Option<String>,
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
            effort: None,
            pwd,
        }
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }

    /// Current model id shown in this footer (#1085 select restore).
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Model id for selector restore, or `None` when still the unset default.
    pub fn known_model(&self) -> Option<&str> {
        (self.model != "unknown").then_some(self.model.as_str())
    }

    /// Record the active effort level; `None` shows the effective default.
    pub fn set_effort(&mut self, effort: Option<String>) {
        self.effort = effort;
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref()
    }

    pub fn set_git_branch(&mut self, branch: Option<String>) {
        self.git_branch = branch;
    }

    pub fn set_pwd_path(&mut self, path: &std::path::Path) {
        let mut pwd = path.display().to_string();
        if let Ok(home) = std::env::var("HOME") {
            if pwd.starts_with(&home) {
                pwd = format!("~{}", &pwd[home.len()..]);
            }
        }
        self.pwd = pwd;
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

    pub fn is_streaming(&self) -> bool {
        self.is_streaming
    }

    /// Record cumulative session cost (USD). `None` hides the indicator.
    pub fn set_cost(&mut self, cost: Option<f64>) {
        self.session_cost = cost;
    }

    /// Apply typed `get_state` footer fields (model + context-window + effort).
    /// Returns the sanitized model id when present so callers can track it.
    /// Single source of truth for the get_state→footer mapping shared by the
    /// master footer path and per-session sub-agent footers (#805).
    ///
    /// Wire JSON is interpreted by [`crate::protocol::state_payloads`]; this
    /// method only applies the already-typed fields.
    pub fn apply_get_state_fields(
        &mut self,
        fields: &crate::protocol::state_payloads::GetStateFooterFields,
    ) -> Option<String> {
        let model = fields.model.as_ref().map(|m| {
            self.set_model(m);
            m.clone()
        });
        if let Some(max_ctx) = fields.max_context_tokens {
            self.set_context_window(max_ctx as usize);
        }
        // #1067: missing and explicit-null effort both arrive as None so the
        // footer always reflects the effective default rather than freezing
        // a stale level.
        self.set_effort(fields.effort.clone());
        model
    }

    /// Apply a raw `get_state` payload via the protocol mapper (compat wrapper).
    pub fn apply_get_state(&mut self, data: &serde_json::Value) -> Option<String> {
        let fields = crate::protocol::state_payloads::parse_get_state_footer(
            data,
            &crate::components::ansi::sanitize_control,
        );
        self.apply_get_state_fields(&fields)
    }

    /// Apply a parsed session-stats snapshot to the context + cost gauges.
    /// Single source of truth for the stats→footer mapping shared by the master
    /// footer path and per-session sub-agent footers (#805) — keeps the cost
    /// gate (`cost > 0`) from drifting between the two.
    pub fn apply_session_stats(&mut self, stats: &crate::protocol::session_payloads::SessionStats) {
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

        let left = context_str;
        // Prefix the model with a streaming indicator while a response is
        // streaming so the toggled flag is actually visible (issue #760), and
        // suffix the active effort level (#1067) — "default" when never set,
        // so the effective config/provider default is always visible.
        let effort = self.effort.as_deref().unwrap_or("default");
        let right = if self.is_streaming {
            format!(
                "{} {} · effort: {}",
                theme::STREAMING_INDICATOR,
                self.model,
                effort
            )
        } else {
            format!("{} · effort: {}", self.model, effort)
        };
        let right = right.as_str();
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
#[path = "footer_tests.rs"]
mod tests;
