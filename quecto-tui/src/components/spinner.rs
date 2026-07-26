//! Spinner component — animated activity indicator.

use crate::components::component::Component;
use crate::interface::theme;
use crate::interface::utils::truncate_to_width;

/// Braille spinner frames.
use crate::interface::theme::SPINNER_FRAMES as FRAMES;

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

    /// Current animation frame index (for tests / diagnostics).
    pub fn frame_index(&self) -> usize {
        self.frame
    }

    /// Current status message text (for tests / diagnostics).
    #[cfg(test)]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Component for Spinner {
    fn render(&mut self, width: usize) -> Vec<String> {
        if !self.active {
            return vec![];
        }
        let spinner = theme::spinner(FRAMES[self.frame]);
        let msg = theme::muted(&self.message);
        // 2-space gutter so the spinner sits under the `▸` column of the
        // subagent/workflow panels in the bottom section (shared left margin).
        let line = format!("  {} {}", spinner, msg);
        vec![truncate_to_width(&line, width, None)]
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
#[path = "spinner_tests.rs"]
mod tests;
