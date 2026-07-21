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

    pub fn expires_at(&self) -> Instant {
        self.created + self.duration
    }

    /// The raw message text, independent of expiry/rendering. Test seam —
    /// gated like its only consumers so it never ships in a plain build.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn message(&self) -> &str {
        &self.message
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

    /// Raw messages of every notification pushed and not yet gc'd, INCLUDING
    /// expired ones. Test seam (#1067): asserting "a notification was pushed
    /// with this content" must not race the 3s display lifetime. Gated like
    /// its only consumers (the cfg'd test-harness modules) so a plain build
    /// never ships a zero-caller accessor.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn messages(&self) -> Vec<String> {
        self.notifications
            .iter()
            .map(|n| n.message().to_string())
            .collect()
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

    pub fn next_expiry(&self) -> Option<Instant> {
        self.notifications
            .iter()
            .map(Notification::expires_at)
            .min()
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
#[path = "notification_tests.rs"]
mod tests;
