//! Session info component — displays session stats and token usage.

use crate::interface::component::Component;
use crate::interface::theme;
use crate::interface::utils::truncate_to_width;

/// Accumulated token usage statistics.
#[derive(Debug, Clone, Default)]
pub struct TokenStats {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

/// Session information display.
pub struct SessionInfo {
    session_key: String,
    message_count: usize,
    tokens: TokenStats,
    context_percent: Option<f64>,
    context_window: usize,
}

impl SessionInfo {
    pub fn new(session_key: &str) -> Self {
        Self {
            session_key: session_key.to_string(),
            message_count: 0,
            tokens: TokenStats::default(),
            context_percent: None,
            context_window: 0,
        }
    }

    pub fn set_message_count(&mut self, count: usize) {
        self.message_count = count;
    }

    pub fn set_tokens(&mut self, tokens: TokenStats) {
        self.tokens = tokens;
    }

    pub fn set_context(&mut self, percent: Option<f64>, window: usize) {
        self.context_percent = percent;
        self.context_window = window;
    }
}

impl Component for SessionInfo {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let pad = "  ";

        lines.push(theme::bold(&theme::accent("Session Info")));
        lines.push(String::new());

        lines.push(truncate_to_width(
            &format!("{}Key:      {}", pad, self.session_key),
            width,
            None,
        ));
        lines.push(truncate_to_width(
            &format!("{}Messages: {}", pad, self.message_count),
            width,
            None,
        ));

        // Token stats.
        if self.tokens.input > 0 || self.tokens.output > 0 {
            lines.push(String::new());
            lines.push(truncate_to_width(
                &format!(
                    "{}Tokens:   ↑{} ↓{}",
                    pad,
                    format_tokens(self.tokens.input),
                    format_tokens(self.tokens.output)
                ),
                width,
                None,
            ));
            if self.tokens.cache_read > 0 || self.tokens.cache_write > 0 {
                lines.push(truncate_to_width(
                    &format!(
                        "{}Cache:    R{} W{}",
                        pad,
                        format_tokens(self.tokens.cache_read),
                        format_tokens(self.tokens.cache_write)
                    ),
                    width,
                    None,
                ));
            }
            if self.tokens.cost > 0.0 {
                lines.push(truncate_to_width(
                    &format!("{}Cost:     ${:.4}", pad, self.tokens.cost),
                    width,
                    None,
                ));
            }
        }

        // Context usage.
        if let Some(pct) = self.context_percent {
            let window_str = format_tokens(self.context_window as u64);
            let pct_str = format!("{:.1}%/{}", pct, window_str);
            let styled = if pct > 90.0 {
                theme::error(&pct_str)
            } else if pct > 70.0 {
                theme::warning(&pct_str)
            } else {
                theme::success(&pct_str)
            };
            lines.push(truncate_to_width(
                &format!("{}Context:  {}", pad, styled),
                width,
                None,
            ));
        }

        lines
    }

    fn invalidate(&mut self) {}
}

/// Format a token count for compact display.
pub fn format_tokens(count: u64) -> String {
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

    fn strip_ansi(s: &str) -> String {
        let mut r = String::new();
        let mut esc = false;
        for c in s.chars() {
            if esc {
                if c.is_ascii_alphabetic() || c == '~' {
                    esc = false;
                }
            } else if c == '\x1b' {
                esc = true;
            } else {
                r.push(c);
            }
        }
        r
    }

    #[test]
    fn shows_session_key() {
        let mut s = SessionInfo::new("cli:default");
        let lines = s.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("cli:default"), "should show key: {}", plain);
    }

    #[test]
    fn shows_message_count() {
        let mut s = SessionInfo::new("test");
        s.set_message_count(10);
        let lines = s.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("10"), "should show count: {}", plain);
    }

    #[test]
    fn shows_token_stats() {
        let mut s = SessionInfo::new("test");
        s.set_tokens(TokenStats {
            input: 15000,
            output: 3200,
            ..Default::default()
        });
        let lines = s.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("15k"), "should show input: {}", plain);
        assert!(plain.contains("3.2k"), "should show output: {}", plain);
    }

    #[test]
    fn context_warning_color() {
        let mut s = SessionInfo::new("test");
        s.set_context(Some(85.0), 200000);
        let lines = s.render(60);
        let joined = lines.join("");
        // Warning color uses \x1b[33m (yellow).
        assert!(
            joined.contains("\x1b[33m"),
            "85% should use warning color: {}",
            joined
        );
    }

    #[test]
    fn context_error_color() {
        let mut s = SessionInfo::new("test");
        s.set_context(Some(95.0), 200000);
        let lines = s.render(60);
        let joined = lines.join("");
        // Error color uses \x1b[31m (red).
        assert!(
            joined.contains("\x1b[31m"),
            "95% should use error color: {}",
            joined
        );
    }

    #[test]
    fn format_tokens_various() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1500), "1.5k");
        assert_eq!(format_tokens(15000), "15k");
        assert_eq!(format_tokens(1500000), "1.5M");
    }
}
