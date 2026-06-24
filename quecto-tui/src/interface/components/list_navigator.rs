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
mod tests {
    use super::*;

    #[test]
    fn next_wraps_and_previous_wraps() {
        let mut nav = ListNavigator::new();
        nav.move_previous(3);
        assert_eq!(nav.selected(), 2);
        nav.move_next(3);
        assert_eq!(nav.selected(), 0);
    }

    #[test]
    fn empty_lists_keep_selection_at_zero() {
        let mut nav = ListNavigator::new();
        nav.move_next(0);
        assert_eq!(nav.selected(), 0);
        nav.move_previous(0);
        assert_eq!(nav.selected(), 0);
    }

    #[test]
    fn clamp_keeps_selected_in_bounds() {
        let mut nav = ListNavigator::new();
        nav.move_previous(5);
        assert_eq!(nav.selected(), 4);
        nav.clamp(2);
        assert_eq!(nav.selected(), 1);
        nav.clamp(0);
        assert_eq!(nav.selected(), 0);
    }

    #[test]
    fn visible_range_scrolls_to_selected_item() {
        let mut nav = ListNavigator::new();
        for _ in 0..4 {
            nav.move_next(10);
        }
        assert_eq!(nav.selected(), 4);
        assert_eq!(nav.visible_range(10, 3), 2..5);
        assert_eq!(nav.visible_range(2, 10), 0..2);
        assert_eq!(nav.visible_range(0, 3), 0..0);
    }
}
