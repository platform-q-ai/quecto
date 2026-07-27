//! Select list component — navigable list with selection indicator.

use crate::components::component::Component;
use crate::components::list_navigator::ListNavigator;
use crate::components::list_rows::{DescriptionMode, ListRow, render_windowed};
use crate::components::theme;
use crate::shell::keys::Key;

/// An item in a select list.
#[derive(Debug, Clone)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// Result of a select list interaction — the shared list-interaction result
/// (`Selected` / `Dismissed` / `Pending`), re-exported under this surface's
/// historical name.
pub use crate::components::autocomplete::AutocompleteResult as SelectResult;

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
}

/// Route a key into an overlay selector slot (#997 dedup of the resume and
/// rewind overlay key handlers): forwards the key, closes the overlay on
/// `Selected`/`Dismissed`, and returns the selected value, if any.
pub(crate) fn route_overlay_key(slot: &mut Option<SelectList>, key: &Key) -> Option<String> {
    let selector = slot.as_mut()?;
    selector.handle_input(key);
    let result = selector.take_result();
    if !matches!(result, SelectResult::Pending) {
        *slot = None;
    }
    match result {
        SelectResult::Selected(value) => Some(value),
        _ => None,
    }
}

impl Component for SelectList {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();

        if self.items.is_empty() {
            lines.push(theme::dim("  No items"));
            return lines;
        }

        // Shared row renderer (#997): the alignment column covers the visible
        // window only, capped at 32 (#757) — see `DescriptionMode::AlignedWindow`.
        lines.extend(render_windowed(
            &self.items,
            &self.navigator,
            self.max_visible,
            width,
            "",
            DescriptionMode::AlignedWindow { min_desc_width: 10 },
            |item| ListRow {
                description: item.description.clone(),
                ..ListRow::plain(item.label.clone())
            },
        ));
        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Up => self.navigator.move_previous(self.items.len()),
            Key::Down => self.navigator.move_next(self.items.len()),
            Key::Enter => {
                if let Some(item) = self.items.get(self.navigator.selected()) {
                    self.result = SelectResult::Selected(item.value.clone());
                }
            }
            Key::Escape => self.result = SelectResult::Dismissed,
            _ => return false,
        }
        true
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
#[path = "select_list_tests.rs"]
mod tests;
