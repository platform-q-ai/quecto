//! Notification component — transient status messages.

use crate::interface::component::Component;
use crate::interface::theme;
use crate::interface::utils::truncate_to_width;
use std::time::{Duration, Instant};

/// Severity level for notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// A transient notification that auto-dismisses after a timeout.
pub struct Notification {
    message: String,
    level: NotifyLevel,
    created: Instant,
    duration: Duration,
}

impl Notification {
    pub fn new(message: &str, level: NotifyLevel) -> Self {
        Self {
            message: message.to_string(),
            level,
            created: Instant::now(),
            duration: Duration::from_secs(3),
        }
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Whether the notification has expired and should be removed.
    pub fn is_expired(&self) -> bool {
        self.created.elapsed() >= self.duration
    }
}

impl Component for Notification {
    fn render(&mut self, width: usize) -> Vec<String> {
        if self.is_expired() {
            return vec![];
        }

        let (icon, color_fn): (&str, fn(&str) -> String) = match self.level {
            NotifyLevel::Info => ("ℹ", theme::accent),
            NotifyLevel::Success => ("✓", theme::success),
            NotifyLevel::Warning => ("⚠", theme::warning),
            NotifyLevel::Error => ("✗", theme::error),
        };

        let line = format!("{} {}", color_fn(icon), color_fn(&self.message));
        vec![truncate_to_width(&line, width, None)]
    }

    fn invalidate(&mut self) {}
}

/// Maximum number of notifications kept on the stack; the oldest is evicted
/// once this limit is exceeded.
const MAX_NOTIFICATIONS: usize = 5;

/// Manages a stack of notifications (newest on top).
pub struct NotificationStack {
    notifications: Vec<Notification>,
}

impl Default for NotificationStack {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationStack {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
        }
    }

    pub fn push(&mut self, notification: Notification) {
        // Limit stack size.
        if self.notifications.len() >= MAX_NOTIFICATIONS {
            self.notifications.remove(0);
        }
        self.notifications.push(notification);
    }

    /// Remove expired notifications. Returns true if any were removed.
    pub fn gc(&mut self) -> bool {
        let before = self.notifications.len();
        self.notifications.retain(|n| !n.is_expired());
        self.notifications.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty()
    }
}

impl Component for NotificationStack {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for notif in &mut self.notifications {
            lines.extend(notif.render(width));
        }
        lines
    }

    fn invalidate(&mut self) {}
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
    fn renders_message() {
        let mut n = Notification::new("Saved!", NotifyLevel::Success);
        let lines = n.render(40);
        assert_eq!(lines.len(), 1);
        let plain = strip_ansi(&lines[0]);
        assert!(plain.contains("Saved!"), "should show message: {}", plain);
    }

    #[test]
    fn expired_renders_empty() {
        let mut n =
            Notification::new("test", NotifyLevel::Info).with_duration(Duration::from_millis(0));
        std::thread::sleep(Duration::from_millis(1));
        let lines = n.render(40);
        assert!(lines.is_empty());
    }

    #[test]
    fn info_level_icon() {
        let mut n = Notification::new("info", NotifyLevel::Info);
        let lines = n.render(40);
        let plain = strip_ansi(&lines[0]);
        assert!(plain.contains("ℹ"), "should have info icon: {}", plain);
    }

    #[test]
    fn error_level_icon() {
        let mut n = Notification::new("error", NotifyLevel::Error);
        let lines = n.render(40);
        let plain = strip_ansi(&lines[0]);
        assert!(plain.contains("✗"), "should have error icon: {}", plain);
    }

    #[test]
    fn stack_gc_removes_expired() {
        let mut stack = NotificationStack::new();
        stack.push(
            Notification::new("old", NotifyLevel::Info).with_duration(Duration::from_millis(0)),
        );
        std::thread::sleep(Duration::from_millis(1));
        assert!(stack.gc());
        assert!(stack.is_empty());
    }

    #[test]
    fn stack_limits_size() {
        let mut stack = NotificationStack::new();
        for i in 0..10 {
            stack.push(Notification::new(&format!("msg{}", i), NotifyLevel::Info));
        }
        assert!(stack.notifications.len() <= MAX_NOTIFICATIONS);
    }
}
