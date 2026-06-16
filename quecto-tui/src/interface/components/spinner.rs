//! Spinner component — animated activity indicator.

use crate::interface::component::Component;
use crate::interface::theme;
use crate::interface::utils::truncate_to_width;

/// Braille spinner frames.
const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Animated spinner with a status message.
pub struct Spinner {
    message: String,
    frame: usize,
    active: bool,
}

impl Spinner {
    pub fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
            frame: 0,
            active: true,
        }
    }

    pub fn set_message(&mut self, message: &str) {
        self.message = message.to_string();
        self.invalidate();
    }

    /// Advance the spinner by one frame. Returns true if active.
    pub fn tick(&mut self) -> bool {
        if self.active {
            self.frame = (self.frame + 1) % FRAMES.len();
        }
        self.active
    }

    pub fn stop(&mut self) {
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn frame_index(&self) -> usize {
        self.frame
    }
}

impl Component for Spinner {
    fn render(&mut self, width: usize) -> Vec<String> {
        if !self.active {
            return vec![];
        }
        let spinner = theme::spinner(FRAMES[self.frame % FRAMES.len()]);
        let msg = theme::muted(&self.message);
        let line = format!("{} {}", spinner, msg);
        vec![truncate_to_width(&line, width, None)]
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_renders_message() {
        let mut s = Spinner::new("Working...");
        let lines = s.render(40);
        assert_eq!(lines.len(), 1);
        // Should contain the message text (ignoring ANSI)
        let plain: String = lines[0].chars().filter(|c| !c.is_control()).collect();
        assert!(
            plain.contains("Working..."),
            "should contain message: {}",
            plain
        );
    }

    #[test]
    fn spinner_tick_advances_frame() {
        let mut s = Spinner::new("test");
        let f0 = s.frame_index();
        s.tick();
        let f1 = s.frame_index();
        assert_ne!(f0, f1);
    }

    #[test]
    fn spinner_stop_renders_empty() {
        let mut s = Spinner::new("test");
        s.stop();
        let lines = s.render(40);
        assert!(lines.is_empty());
    }

    #[test]
    fn spinner_tick_cycles() {
        let mut s = Spinner::new("test");
        for _ in 0..FRAMES.len() {
            s.tick();
        }
        assert_eq!(s.frame_index(), 0); // wrapped around
    }

    #[test]
    fn spinner_set_message() {
        let mut s = Spinner::new("old");
        s.set_message("new");
        let lines = s.render(40);
        let plain: String = lines[0].chars().filter(|c| !c.is_control()).collect();
        assert!(plain.contains("new"));
    }

    #[test]
    fn spinner_is_active() {
        let mut s = Spinner::new("test");
        assert!(s.is_active());
        s.stop();
        assert!(!s.is_active());
    }

    #[test]
    fn spinner_tick_returns_true_when_active() {
        let mut s = Spinner::new("test");
        assert!(s.tick());
    }

    #[test]
    fn spinner_tick_returns_false_when_stopped() {
        let mut s = Spinner::new("test");
        s.stop();
        assert!(!s.tick());
    }

    #[test]
    fn spinner_invalidate() {
        let mut s = Spinner::new("test");
        let _ = s.render(40);
        s.invalidate();
        // Re-render should still work
        assert_eq!(s.render(40).len(), 1);
    }

    #[test]
    fn spinner_respects_width() {
        let mut s = Spinner::new("a very long spinner message that exceeds the width");
        let lines = s.render(20);
        for line in &lines {
            assert!(crate::interface::utils::visible_width(line) <= 20);
        }
    }
}
