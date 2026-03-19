//! Compaction indicator — shows progress during context compaction.

use crate::component::Component;
use crate::components::spinner::Spinner;
use crate::keys::Key;

/// Result of a compaction interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionResult {
    Cancelled,
    Pending,
}

/// Auto-compaction indicator with cancel support.
pub struct CompactionIndicator {
    spinner: Spinner,
    result: CompactionResult,
}

impl CompactionIndicator {
    pub fn new(message: &str) -> Self {
        let msg = format!("{} (Esc to cancel)", message);
        Self {
            spinner: Spinner::new(&msg),
            result: CompactionResult::Pending,
        }
    }

    pub fn tick(&mut self) -> bool {
        self.spinner.tick()
    }

    pub fn take_result(&mut self) -> CompactionResult {
        std::mem::replace(&mut self.result, CompactionResult::Pending)
    }
}

impl Component for CompactionIndicator {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.spinner.render(width)
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        if matches!(key, Key::Escape) {
            self.result = CompactionResult::Cancelled;
            self.spinner.stop();
            return true;
        }
        false
    }

    fn invalidate(&mut self) {}
}

/// Queue for messages received during compaction.
///
/// Messages are held until compaction completes, then drained in order.
pub struct MessageQueue {
    messages: Vec<QueuedMessage>,
}

/// A queued message with its delivery mode.
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub text: String,
    pub mode: DeliveryMode,
}

/// How a queued message should be delivered after compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    Steer,
    FollowUp,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn push(&mut self, text: String, mode: DeliveryMode) {
        self.messages.push(QueuedMessage { text, mode });
    }

    pub fn drain(&mut self) -> Vec<QueuedMessage> {
        std::mem::take(&mut self.messages)
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
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
    fn renders_message() {
        let mut c = CompactionIndicator::new("Auto-compacting...");
        let lines = c.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("");
        assert!(
            plain.contains("Auto-compacting"),
            "should show message: {}",
            plain
        );
    }

    #[test]
    fn escape_cancels() {
        let mut c = CompactionIndicator::new("test");
        c.handle_input(&Key::Escape);
        assert_eq!(c.take_result(), CompactionResult::Cancelled);
    }

    #[test]
    fn queue_push_and_drain() {
        let mut q = MessageQueue::new();
        q.push("first".to_string(), DeliveryMode::Steer);
        q.push("second".to_string(), DeliveryMode::FollowUp);
        assert_eq!(q.len(), 2);
        let msgs = q.drain();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "first");
        assert_eq!(msgs[1].text, "second");
        assert!(q.is_empty());
    }

    #[test]
    fn queue_empty() {
        let q = MessageQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn tick_advances() {
        let mut c = CompactionIndicator::new("test");
        assert!(c.tick());
    }
}
