//! Retry indicator component — shows countdown during auto-retry.

use crate::component::Component;
use crate::components::spinner::Spinner;
use crate::keys::Key;
use crate::theme;
use crate::utils::truncate_to_width;

/// Result of a retry interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryResult {
    /// User cancelled the retry.
    Cancelled,
    /// Still waiting.
    Pending,
}

/// Auto-retry indicator with countdown and cancel support.
pub struct RetryIndicator {
    attempt: u32,
    max_attempts: u32,
    delay_secs: u32,
    spinner: Spinner,
    result: RetryResult,
}

impl RetryIndicator {
    pub fn new(attempt: u32, max_attempts: u32, delay_secs: u32) -> Self {
        let msg = format!(
            "Retrying ({}/{}) in {}s... (Esc to cancel)",
            attempt, max_attempts, delay_secs
        );
        Self {
            attempt,
            max_attempts,
            delay_secs,
            spinner: Spinner::new(&msg),
            result: RetryResult::Pending,
        }
    }

    pub fn tick(&mut self) -> bool {
        self.spinner.tick()
    }

    pub fn take_result(&mut self) -> RetryResult {
        std::mem::replace(&mut self.result, RetryResult::Pending)
    }
}

impl Component for RetryIndicator {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.spinner.render(width)
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        if matches!(key, Key::Escape) {
            self.result = RetryResult::Cancelled;
            self.spinner.stop();
            return true;
        }
        false
    }

    fn invalidate(&mut self) {}
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
    fn renders_attempt_info() {
        let mut r = RetryIndicator::new(2, 3, 5);
        let lines = r.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("");
        assert!(plain.contains("2/3"), "should show attempt: {}", plain);
        assert!(plain.contains("5s"), "should show delay: {}", plain);
    }

    #[test]
    fn escape_cancels() {
        let mut r = RetryIndicator::new(1, 3, 5);
        r.handle_input(&Key::Escape);
        assert_eq!(r.take_result(), RetryResult::Cancelled);
    }

    #[test]
    fn tick_advances() {
        let mut r = RetryIndicator::new(1, 3, 5);
        assert!(r.tick());
    }
}
