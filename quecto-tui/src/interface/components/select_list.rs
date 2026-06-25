//! Select list component — navigable list with selection indicator.

use crate::interface::component::Component;
use crate::interface::components::list_navigator::ListNavigator;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

/// An item in a select list.
#[derive(Debug, Clone)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// Result of a select list interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectResult {
    /// User selected an item.
    Selected(String),
    /// User cancelled (Escape).
    Cancelled,
    /// No action yet.
    Pending,
}

/// A navigable list with selection indicator and optional descriptions.
pub struct SelectList {
    items: Vec<SelectItem>,
    navigator: ListNavigator,
    max_visible: usize,
    result: SelectResult,
}

impl SelectList {
    pub fn new(items: Vec<SelectItem>, max_visible: usize) -> Self {
        Self {
            items,
            navigator: ListNavigator::new(),
            max_visible,
            result: SelectResult::Pending,
        }
    }

    pub fn take_result(&mut self) -> SelectResult {
        std::mem::replace(&mut self.result, SelectResult::Pending)
    }

    pub fn selected_item(&self) -> Option<&SelectItem> {
        self.items.get(self.navigator.selected())
    }

    /// Replace the items in place, preserving the selection by `value` when the
    /// previously-selected item still exists (otherwise clamping into range).
    /// Lets a live-updating list (e.g. the sub-agent inspector) refresh without
    /// resetting the user's highlight (#795).
    pub fn sync_items(&mut self, items: Vec<SelectItem>) {
        let selected_value = self.selected_item().map(|i| i.value.clone());
        self.items = items;
        if let Some(value) = selected_value
            && let Some(idx) = self.items.iter().position(|i| i.value == value)
        {
            self.navigator.set_selected(idx);
        }
        self.navigator.clamp(self.items.len());
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    #[cfg(test)]
    pub fn render_text(&mut self, width: usize) -> String {
        self.render(width).join("\n")
    }
}

impl Component for SelectList {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();

        if self.items.is_empty() {
            lines.push(theme::dim("  No items"));
            return lines;
        }

        // Calculate visible window with scrolling.
        let total = self.items.len();
        let range = self.navigator.visible_range(total, self.max_visible);
        let start = range.start;
        let end = range.end;

        // Calculate primary column width for alignment over the visible window
        // only — the off-screen items are never drawn, so widening the column
        // for them would just waste a full-list scan every frame (#757).
        let primary_width = self.items[start..end]
            .iter()
            .map(|i| visible_width(&i.label))
            .max()
            .unwrap_or(10)
            .min(32);

        for i in start..end {
            let item = &self.items[i];
            let is_sel = i == self.navigator.selected();
            let prefix = if is_sel { "→ " } else { "  " };
            let prefix_width = 2;

            let label = if is_sel {
                theme::accent(&item.label)
            } else {
                item.label.clone()
            };

            if let Some(desc) = &item.description {
                let label_vis = visible_width(&item.label);
                let gap = primary_width.saturating_sub(label_vis) + 2;
                let desc_start = prefix_width + label_vis + gap;
                let desc_width = width.saturating_sub(desc_start + 1);
                if desc_width > 10 {
                    let truncated_desc = truncate_to_width(desc, desc_width, Some(""));
                    let spacing = " ".repeat(gap);
                    let line = format!(
                        "{}{}{}{}",
                        prefix,
                        label,
                        spacing,
                        theme::dim(&truncated_desc)
                    );
                    lines.push(truncate_to_width(&line, width, None));
                } else {
                    lines.push(truncate_to_width(
                        &format!("{}{}", prefix, label),
                        width,
                        None,
                    ));
                }
            } else {
                lines.push(truncate_to_width(
                    &format!("{}{}", prefix, label),
                    width,
                    None,
                ));
            }
        }

        // Scroll indicator.
        if start > 0 || end < total {
            let info = format!("  ({}/{})", self.navigator.selected() + 1, total);
            lines.push(theme::dim(&info));
        }

        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Up => {
                self.navigator.move_previous(self.items.len());
                true
            }
            Key::Down => {
                self.navigator.move_next(self.items.len());
                true
            }
            Key::Enter => {
                if let Some(item) = self.items.get(self.navigator.selected()) {
                    self.result = SelectResult::Selected(item.value.clone());
                }
                true
            }
            Key::Escape => {
                self.result = SelectResult::Cancelled;
                true
            }
            _ => false,
        }
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_items(labels: &[&str]) -> Vec<SelectItem> {
        labels
            .iter()
            .map(|l| SelectItem {
                value: l.to_string(),
                label: l.to_string(),
                description: None,
            })
            .collect()
    }

    #[test]
    fn renders_items() {
        let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
        let lines = list.render(40);
        let joined: String = lines.join("\n");
        assert!(joined.contains("A"));
        assert!(joined.contains("B"));
        assert!(joined.contains("C"));
    }

    #[test]
    fn selection_indicator() {
        let mut list = SelectList::new(make_items(&["A", "B"]), 10);
        let lines = list.render(40);
        // First item should have selection indicator.
        assert!(lines[0].contains("→") || lines[0].contains("→"));
    }

    #[test]
    fn navigate_down() {
        let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
        list.handle_input(&Key::Down);
        assert_eq!(list.selected_item().unwrap().value, "B");
    }

    #[test]
    fn navigate_up_wraps() {
        let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
        list.handle_input(&Key::Up);
        assert_eq!(list.selected_item().unwrap().value, "C");
    }

    #[test]
    fn navigate_down_wraps() {
        let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
        list.handle_input(&Key::Down);
        list.handle_input(&Key::Down);
        list.handle_input(&Key::Down);
        assert_eq!(list.selected_item().unwrap().value, "A");
    }

    #[test]
    fn enter_selects() {
        let mut list = SelectList::new(make_items(&["A", "B"]), 10);
        list.handle_input(&Key::Down);
        list.handle_input(&Key::Enter);
        assert_eq!(list.take_result(), SelectResult::Selected("B".to_string()));
    }

    #[test]
    fn escape_cancels() {
        let mut list = SelectList::new(make_items(&["A"]), 10);
        list.handle_input(&Key::Escape);
        assert_eq!(list.take_result(), SelectResult::Cancelled);
    }

    #[test]
    fn empty_list() {
        let mut list = SelectList::new(vec![], 10);
        let lines = list.render(40);
        assert!(!lines.is_empty());
    }

    #[test]
    fn with_descriptions() {
        let items = vec![SelectItem {
            value: "model".to_string(),
            label: "model".to_string(),
            description: Some("Select model".to_string()),
        }];
        let mut list = SelectList::new(items, 10);
        let lines = list.render(60);
        let joined: String = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(plain.contains("model"), "should show label");
        assert!(
            plain.contains("Select model"),
            "should show description: {}",
            plain
        );
    }

    #[test]
    fn scroll_indicator_on_overflow() {
        let mut list = SelectList::new(make_items(&["A", "B", "C", "D", "E"]), 3);
        let lines = list.render(40);
        let joined: String = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(
            plain.contains("1/5"),
            "should show scroll position: {}",
            plain
        );
    }
}
