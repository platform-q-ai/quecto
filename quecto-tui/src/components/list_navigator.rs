//! Shared selected-index navigation for list-like TUI components.
//!
//! This helper intentionally owns only index movement, clamping, and visible
//! window calculation. Components keep their own item storage, rendering, and
//! result enums.

use std::ops::Range;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListNavigator {
    selected: usize,
}

impl ListNavigator {
    pub fn new() -> Self {
        Self { selected: 0 }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn reset(&mut self) {
        self.selected = 0;
    }

    /// Set the selected index directly (caller is responsible for bounds; pair
    /// with [`Self::clamp`] if `len` may have shrunk).
    pub fn set_selected(&mut self, index: usize) {
        self.selected = index;
    }

    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    pub fn move_next(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected + 1) % len;
    }

    pub fn move_previous(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = if self.selected == 0 {
            len - 1
        } else {
            self.selected - 1
        };
    }

    pub fn visible_range(&self, len: usize, max_visible: usize) -> Range<usize> {
        if len == 0 || max_visible == 0 {
            return 0..0;
        }
        let visible = len.min(max_visible);
        let selected = self.selected.min(len - 1);
        let start = if selected >= visible {
            (selected + 1).saturating_sub(visible)
        } else {
            0
        };
        start..(start + visible).min(len)
    }
}

#[cfg(test)]
#[path = "list_navigator_tests.rs"]
mod tests;
